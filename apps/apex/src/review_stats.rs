//! Statistik agregasi deterministik untuk subcommand `apex review` (`WP-7`).
//!
//! Prinsip desain:
//! Dengan sekitar ±4 trade per bulan, evaluasi sampel kecil sangat rentan terhadap ilusi pola acak.
//! Seluruh estimasi `E[R]` menggunakan **block-bootstrap 10.000 resample per minggu ISO** memakai
//! RNG deterministik [`apex_backtest::stats::SplitMix`], interval win-rate memakai [`apex_oms::wilson95`],
//! dan kuantil memakai [`apex_indicators::percentile`]. Profit Factor hanya dihitung bila `n >= 100`.
#![cfg_attr(
    not(test),
    deny(clippy::unwrap_used, clippy::expect_used, clippy::panic)
)]

use std::collections::BTreeMap;
use std::fs;
use std::path::Path;

use apex_backtest::stats::SplitMix;
use apex_core::num::to_f64;
use apex_core::time::{fmt_ymd, parse_ymd, DAY_MS};
use apex_core::types::Ms;
use apex_indicators::percentile;
use apex_oms::{
    build_five_whys, classify_trade_mistake, wilson95, EvidenceStatus, ForwardGateReport,
    TelemetryExitKind, TradeTelemetry, WilsonInterval, GAVE_BACK_STRONG_MFE_R,
};
use rust_decimal::Decimal;
use serde::{Deserialize, Serialize};

pub const REVIEW_BOOTSTRAP_SEED: u64 = 0x517C_C1B7_2722_0A95;
pub const REVIEW_BOOTSTRAP_RESAMPLES: usize = 10_000;

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct SummaryStats {
    pub n: usize,
    pub iso_weeks: usize,
    pub sample_status: String,
    pub mean_r: Option<f64>,
    pub ci95_er: Option<(f64, f64)>,
    pub wins: usize,
    pub win_rate_pct: Option<f64>,
    pub wilson95_pct: Option<WilsonInterval>,
    pub profit_factor: Option<f64>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct AttributionCell {
    pub key: String,
    pub n: usize,
    pub sum_r: f64,
    pub mean_r: f64,
    pub flagged_small_n: bool,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct AttributionSection {
    pub by_strategy: Vec<AttributionCell>,
    pub by_symbol: Vec<AttributionCell>,
    pub by_side: Vec<AttributionCell>,
    pub by_utc_hour: Vec<AttributionCell>,
    pub by_regime: Vec<AttributionCell>,
    pub by_macro_bias: Vec<AttributionCell>,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct ExecutionQualityStats {
    pub non_paper_n: usize,
    pub slip_diff_mean_r: Option<f64>,
    pub slip_diff_p95_r: Option<f64>,
    pub maker_fill_ratio: Option<f64>,
    pub time_to_fill_p50_ms: Option<f64>,
    pub time_to_fill_p95_ms: Option<f64>,
    pub unprotected_p50_ms: Option<f64>,
    pub unprotected_p95_ms: Option<f64>,
    pub unprotected_max_ms: Option<i64>,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct ExcursionStats {
    pub n_with_excursion: usize,
    pub mfe_p50_r: Option<f64>,
    pub mfe_p95_r: Option<f64>,
    pub mae_p50_r: Option<f64>,
    pub mae_p95_r: Option<f64>,
    pub gave_back_count: usize,
    pub gave_back_rate_pct: Option<f64>,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct ConcentrationStats {
    pub net_pnl_usd: f64,
    pub top_symbol: Option<(String, f64, Option<f64>)>,
    pub top_month: Option<(String, f64, Option<f64>)>,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct ForensicsPareto {
    pub losing_trades: usize,
    pub mistake_tags: Vec<(String, usize)>,
    pub supported_layers: Vec<(String, usize)>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ReviewReportData {
    pub since: String,
    pub until: Option<String>,
    pub strategy_filter: Option<String>,
    pub summary: SummaryStats,
    pub skip_funnel: Vec<(String, usize)>,
    pub attribution: AttributionSection,
    pub execution: ExecutionQualityStats,
    pub excursion: ExcursionStats,
    pub concentration: ConcentrationStats,
    pub forensics: ForensicsPareto,
    pub forward_demo: Option<ForwardGateReport>,
    pub forward_shadow: Option<ForwardGateReport>,
    pub conclusion: String,
    pub conclusion_detail: String,
}

/// Konversi timestamp epoch ms UTC menjadi pasangan `(iso_year, iso_week)` sesuai ISO-8601.
pub fn iso_year_week(ms: Ms) -> (i32, u32) {
    let day = ms.div_euclid(DAY_MS);
    let weekday_mon0 = (day + 3).rem_euclid(7); // 1970-01-01 adalah Kamis (3)
    let thu_day = day + (3 - weekday_mon0);
    let ymd = fmt_ymd(thu_day * DAY_MS);
    let iso_year: i32 = ymd.get(..4).and_then(|s| s.parse().ok()).unwrap_or(1970);
    let jan4_ms = parse_ymd(&format!("{iso_year:04}-01-04")).unwrap_or(0);
    let jan4_day = jan4_ms.div_euclid(DAY_MS);
    let week1_mon = jan4_day - (jan4_day + 3).rem_euclid(7);
    let iso_week = ((thu_day - week1_mon).div_euclid(7) + 1).max(1) as u32;
    (iso_year, iso_week)
}

/// Block-bootstrap per minggu ISO menggunakan [`SplitMix`] deterministik.
/// Mengembalikan `(mean_r, Option<(ci_low, ci_high)>, iso_week_count)`.
/// Bila seluruh sampel berada dalam `< 2` minggu ISO, CI dikembalikan `None` karena variasi antar-blok belum terukur.
pub fn iso_week_block_bootstrap_ci95(
    ts_and_r: &[(Ms, f64)],
    resamples: usize,
) -> Option<(f64, Option<(f64, f64)>, usize)> {
    if ts_and_r.is_empty() {
        return None;
    }
    let mean_r = ts_and_r.iter().map(|(_, r)| *r).sum::<f64>() / ts_and_r.len() as f64;
    let mut grouped: BTreeMap<(i32, u32), Vec<f64>> = BTreeMap::new();
    for &(ts, r) in ts_and_r {
        grouped.entry(iso_year_week(ts)).or_default().push(r);
    }
    let blocks: Vec<Vec<f64>> = grouped.into_values().collect();
    let k = blocks.len();
    if k < 2 || resamples == 0 {
        return Some((mean_r, None, k));
    }
    let mut rng = SplitMix(REVIEW_BOOTSTRAP_SEED);
    let mut boot_means = Vec::with_capacity(resamples);
    for _ in 0..resamples {
        let mut sum = 0.0;
        let mut cnt = 0usize;
        for _ in 0..k {
            let blk = &blocks[rng.below(k)];
            sum += blk.iter().sum::<f64>();
            cnt += blk.len();
        }
        if cnt > 0 {
            boot_means.push(sum / cnt as f64);
        }
    }
    let lo = percentile(&boot_means, 2.5);
    let hi = percentile(&boot_means, 97.5);
    Some((mean_r, Some((lo, hi)), k))
}

pub fn compute_summary(trades: &[TradeTelemetry]) -> SummaryStats {
    let n = trades.len();
    let ts_r: Vec<(Ms, f64)> = trades
        .iter()
        .map(|t| (t.closed_ms, to_f64(t.r_multiple)))
        .collect();
    let boot = iso_week_block_bootstrap_ci95(&ts_r, REVIEW_BOOTSTRAP_RESAMPLES);
    let wins = trades
        .iter()
        .filter(|t| t.pnl_net_usd > Decimal::ZERO)
        .count();
    let win_rate_pct = (n > 0).then(|| 100.0 * wins as f64 / n as f64);
    let profit_factor = if n >= 100 {
        let gross_win: Decimal = trades
            .iter()
            .map(|t| t.pnl_net_usd)
            .filter(|&p| p > Decimal::ZERO)
            .sum();
        let gross_loss: Decimal = trades
            .iter()
            .map(|t| -t.pnl_net_usd)
            .filter(|&p| p > Decimal::ZERO)
            .sum();
        (gross_loss > Decimal::ZERO).then(|| to_f64(gross_win / gross_loss))
    } else {
        None
    };
    let sample_status = if n == 0 {
        "INSUFFICIENT (n=0)".into()
    } else if n < 30 {
        format!("INSUFFICIENT (n={n} < 30)")
    } else if n < 100 {
        format!("INSUFFICIENT_FOR_PF_AND_CONCLUSION (n={n} < 100)")
    } else {
        format!("COHORT_N_OK (n={n} >= 100)")
    };
    SummaryStats {
        n,
        iso_weeks: boot.map(|x| x.2).unwrap_or(0),
        sample_status,
        mean_r: boot.map(|x| x.0),
        ci95_er: boot.and_then(|x| x.1),
        wins,
        win_rate_pct,
        wilson95_pct: wilson95(wins, n),
        profit_factor,
    }
}

fn build_cells<F: Fn(&TradeTelemetry) -> String>(
    trades: &[TradeTelemetry],
    key_fn: F,
) -> Vec<AttributionCell> {
    let mut map: BTreeMap<String, Vec<f64>> = BTreeMap::new();
    for t in trades {
        map.entry(key_fn(t)).or_default().push(to_f64(t.r_multiple));
    }
    map.into_iter()
        .map(|(key, vals)| {
            let n = vals.len();
            let sum_r: f64 = vals.iter().sum();
            AttributionCell {
                key,
                n,
                sum_r,
                mean_r: if n > 0 { sum_r / n as f64 } else { 0.0 },
                flagged_small_n: n < 10,
            }
        })
        .collect()
}

pub fn compute_attribution(trades: &[TradeTelemetry]) -> AttributionSection {
    AttributionSection {
        by_strategy: build_cells(trades, |t| t.strategy.clone()),
        by_symbol: build_cells(trades, |t| t.symbol.clone()),
        by_side: build_cells(trades, |t| format!("{:?}", t.side)),
        by_utc_hour: build_cells(trades, |t| {
            let base = t.filled_ms.unwrap_or(t.created_ms);
            let hr = base.div_euclid(3_600_000).rem_euclid(24);
            format!("{hr:02}:00Z")
        }),
        by_regime: build_cells(trades, |t| {
            t.entry_ctx
                .as_ref()
                .map(|c| c.regime_label.clone())
                .filter(|s| !s.is_empty())
                .unwrap_or_else(|| "UNKNOWN".into())
        }),
        by_macro_bias: build_cells(trades, |t| {
            t.entry_ctx
                .as_ref()
                .map(|c| c.macro_bias.clone())
                .filter(|s| !s.is_empty())
                .unwrap_or_else(|| "UNKNOWN".into())
        }),
    }
}

pub fn compute_execution_quality(trades: &[TradeTelemetry]) -> ExecutionQualityStats {
    let n = trades.len();
    let slip_diffs: Vec<f64> = trades
        .iter()
        .filter(|t| !t.fills.paper_slip_zero_by_construction)
        .map(|t| to_f64(t.fills.realized_slip_r - t.fills.model_slip_r))
        .collect();
    let ttf: Vec<f64> = trades
        .iter()
        .filter_map(|t| t.filled_ms.map(|f| f.saturating_sub(t.created_ms) as f64))
        .collect();
    let unprot: Vec<f64> = trades
        .iter()
        .filter_map(|t| t.unprotected_ms.map(|u| u as f64))
        .collect();
    let makers = trades.iter().filter(|t| t.fills.maker_entry).count();

    ExecutionQualityStats {
        non_paper_n: slip_diffs.len(),
        slip_diff_mean_r: (!slip_diffs.is_empty())
            .then(|| slip_diffs.iter().sum::<f64>() / slip_diffs.len() as f64),
        slip_diff_p95_r: (!slip_diffs.is_empty()).then(|| percentile(&slip_diffs, 95.0)),
        maker_fill_ratio: (n > 0).then(|| makers as f64 / n as f64),
        time_to_fill_p50_ms: (!ttf.is_empty()).then(|| percentile(&ttf, 50.0)),
        time_to_fill_p95_ms: (!ttf.is_empty()).then(|| percentile(&ttf, 95.0)),
        unprotected_p50_ms: (!unprot.is_empty()).then(|| percentile(&unprot, 50.0)),
        unprotected_p95_ms: (!unprot.is_empty()).then(|| percentile(&unprot, 95.0)),
        unprotected_max_ms: trades.iter().filter_map(|t| t.unprotected_ms).max(),
    }
}

pub fn compute_excursion_stats(trades: &[TradeTelemetry]) -> ExcursionStats {
    let mut mfes = Vec::new();
    let mut maes = Vec::new();
    let mut gave_back_count = 0usize;
    for t in trades {
        if let Some(ex) = &t.excursion {
            mfes.push(to_f64(ex.mfe_r));
            maes.push(to_f64(ex.mae_r));
            if ex.mfe_r >= GAVE_BACK_STRONG_MFE_R && t.exit_kind == TelemetryExitKind::SlHit {
                gave_back_count += 1;
            }
        }
    }
    let n_ex = mfes.len();
    ExcursionStats {
        n_with_excursion: n_ex,
        mfe_p50_r: (n_ex > 0).then(|| percentile(&mfes, 50.0)),
        mfe_p95_r: (n_ex > 0).then(|| percentile(&mfes, 95.0)),
        mae_p50_r: (n_ex > 0).then(|| percentile(&maes, 50.0)),
        mae_p95_r: (n_ex > 0).then(|| percentile(&maes, 95.0)),
        gave_back_count,
        gave_back_rate_pct: (n_ex > 0).then(|| 100.0 * gave_back_count as f64 / n_ex as f64),
    }
}

pub fn compute_concentration(trades: &[TradeTelemetry]) -> ConcentrationStats {
    let mut by_sym: BTreeMap<String, Decimal> = BTreeMap::new();
    let mut by_month: BTreeMap<String, Decimal> = BTreeMap::new();
    let mut total = Decimal::ZERO;
    for t in trades {
        total += t.pnl_net_usd;
        *by_sym.entry(t.symbol.clone()).or_default() += t.pnl_net_usd;
        let ym = fmt_ymd(t.closed_ms)
            .get(..7)
            .unwrap_or("1970-01")
            .to_string();
        *by_month.entry(ym).or_default() += t.pnl_net_usd;
    }
    let net_f = to_f64(total);
    let top_of = |m: BTreeMap<String, Decimal>| -> Option<(String, f64, Option<f64>)> {
        let (k, v) = m.into_iter().max_by(|a, b| a.1.cmp(&b.1))?;
        let vf = to_f64(v);
        let share = (total > Decimal::ZERO).then(|| 100.0 * to_f64(v / total));
        Some((k, vf, share))
    };
    ConcentrationStats {
        net_pnl_usd: net_f,
        top_symbol: top_of(by_sym),
        top_month: top_of(by_month),
    }
}

fn sort_pareto(map: BTreeMap<String, usize>) -> Vec<(String, usize)> {
    let mut v: Vec<(String, usize)> = map.into_iter().collect();
    v.sort_by(|a, b| b.1.cmp(&a.1).then_with(|| a.0.cmp(&b.0)));
    v
}

pub fn compute_forensics_pareto(trades: &[TradeTelemetry]) -> ForensicsPareto {
    let mut tags: BTreeMap<String, usize> = BTreeMap::new();
    let mut layers: BTreeMap<String, usize> = BTreeMap::new();
    let mut losing_trades = 0usize;
    for t in trades {
        let tag = classify_trade_mistake(t);
        if tag != apex_oms::MistakeTag::None {
            losing_trades += 1;
            *tags.entry(tag.as_str().to_string()).or_default() += 1;
        }
        if let Some(fw) = build_five_whys(t) {
            for l in fw.layers.iter().filter(|l| l.layer >= 2 && l.layer <= 4) {
                if l.status == EvidenceStatus::Supported {
                    *layers
                        .entry(format!("L{}_{}", l.layer, l.focus))
                        .or_default() += 1;
                }
            }
        }
    }
    ForensicsPareto {
        losing_trades,
        mistake_tags: sort_pareto(tags),
        supported_layers: sort_pareto(layers),
    }
}

pub fn parse_skip_funnel(
    audit_paths: &[&Path],
    since_ms: Ms,
    until_ms: Option<Ms>,
) -> Vec<(String, usize)> {
    let mut counts: BTreeMap<String, usize> = BTreeMap::new();
    for p in audit_paths {
        let Ok(raw) = fs::read_to_string(p) else {
            continue;
        };
        for line in raw.lines().map(str::trim).filter(|l| !l.is_empty()) {
            let Ok(v) = serde_json::from_str::<serde_json::Value>(line) else {
                continue;
            };
            let Some(ts) = v.get("ts").and_then(|x| x.as_i64()) else {
                continue;
            };
            if ts < since_ms || until_ms.is_some_and(|u| ts >= u) {
                continue;
            }
            if v.get("event").and_then(|x| x.as_str()) == Some("SKIP") {
                let reason = v
                    .get("data")
                    .and_then(|d| d.get("reason"))
                    .and_then(|r| r.as_str())
                    .unwrap_or("SKIP_UNKNOWN");
                let key = if reason.starts_with("SKIP_") {
                    reason.to_string()
                } else {
                    format!("SKIP_{reason}")
                };
                *counts.entry(key).or_default() += 1;
            }
        }
    }
    sort_pareto(counts)
}

pub fn build_conclusion(sum: &SummaryStats) -> (String, String) {
    if sum.n < 100 {
        return (
            "INSUFFICIENT".into(),
            format!(
                "TIDAK ADA KESIMPULAN: sampel n={} (<100 minimum inferensi statistik). Dilarang menyimpulkan keunggulan strategi dari kohort ini.",
                sum.n
            ),
        );
    }
    let Some((lo, hi)) = sum.ci95_er else {
        return (
            "INSUFFICIENT".into(),
            format!(
                "TIDAK ADA KESIMPULAN: jumlah minggu ISO ({}) belum cukup untuk block-bootstrap.",
                sum.iso_weeks
            ),
        );
    };
    if lo <= 0.0 && hi >= 0.0 {
        (
            "TIDAK ADA KESIMPULAN".into(),
            format!(
                "TIDAK ADA KESIMPULAN: pada n={} ({} minggu ISO), CI 95% E[R] [{lo:+.4}R, {hi:+.4}R] masih mencakup 0.",
                sum.n, sum.iso_weeks
            ),
        )
    } else if hi < 0.0 {
        (
            "EKSPEKTASI_NEGATIF_TERUKUR".into(),
            format!(
                "Pada n={} ({} minggu ISO), seluruh CI 95% E[R] [{lo:+.4}R, {hi:+.4}R] berada di bawah 0.",
                sum.n, sum.iso_weeks
            ),
        )
    } else {
        (
            "KANDIDAT_EVALUASI_LANJUT".into(),
            format!(
                "Pada n={} ({} minggu ISO), CI 95% E[R] [{lo:+.4}R, {hi:+.4}R] berada di atas 0; tetap wajib dievaluasi lewat gerbang G-01..G-11 dan forward-gate.",
                sum.n, sum.iso_weeks
            ),
        )
    }
}
