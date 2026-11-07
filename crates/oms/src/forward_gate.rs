//! Evaluator gerbang forward testing (`demo` dan `shadow`) sesuai `Rules.md` GR-9 (`WP-8`).

use std::fs;
use std::path::Path;

use apex_core::num::to_f64;
use apex_core::types::Ms;
use serde::{Deserialize, Serialize};

use crate::telemetry::{read_telemetry_jsonl, ExecSource, TradeTelemetry};

const MS_PER_DAY: f64 = 86_400_000.0;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ForwardStage {
    Demo,
    Shadow,
}

impl ForwardStage {
    pub fn parse_stage(s: &str) -> Result<Self, String> {
        match s.trim().to_ascii_lowercase().as_str() {
            "demo" | "testnet" => Ok(ForwardStage::Demo),
            "shadow" => Ok(ForwardStage::Shadow),
            other => Err(format!(
                "stage tidak dikenal '{other}' (gunakan demo atau shadow)"
            )),
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            ForwardStage::Demo => "demo",
            ForwardStage::Shadow => "shadow",
        }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ForwardStageThresholds {
    pub min_days: f64,
    pub min_trades: usize,
    pub enforce_slip_diff: bool,
    pub max_mean_slip_diff_r: f64,
    pub max_sl_missing: usize,
    pub max_rule_violation: usize,
    pub max_unprotected_ms: i64,
    pub max_orphan_positions: usize,
    pub min_uptime_pct: f64,
    pub max_heartbeat_gap_secs: u64,
    pub max_unfilled_ws_gaps: usize,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ForwardGateFile {
    pub demo: ForwardStageThresholds,
    pub shadow: ForwardStageThresholds,
}

impl Default for ForwardGateFile {
    fn default() -> Self {
        Self {
            demo: ForwardStageThresholds {
                min_days: 14.0,
                min_trades: 30,
                enforce_slip_diff: true,
                max_mean_slip_diff_r: 0.05,
                max_sl_missing: 0,
                max_rule_violation: 0,
                max_unprotected_ms: 2_000,
                max_orphan_positions: 0,
                min_uptime_pct: 95.0,
                max_heartbeat_gap_secs: 1_800,
                max_unfilled_ws_gaps: 0,
            },
            shadow: ForwardStageThresholds {
                min_days: 7.0,
                min_trades: 0,
                enforce_slip_diff: false,
                max_mean_slip_diff_r: 0.05,
                max_sl_missing: 0,
                max_rule_violation: 0,
                max_unprotected_ms: 2_000,
                max_orphan_positions: 0,
                min_uptime_pct: 95.0,
                max_heartbeat_gap_secs: 1_800,
                max_unfilled_ws_gaps: 0,
            },
        }
    }
}

impl ForwardGateFile {
    pub fn load(path: &Path) -> Result<Self, String> {
        if !path.exists() {
            return Err(format!("file {} tidak ditemukan", path.display()));
        }
        apex_core::config::load_toml(path)
            .map_err(|e| format!("gagal memuat {}: {e}", path.display()))
    }

    pub fn thresholds_for(&self, stage: ForwardStage) -> &ForwardStageThresholds {
        match stage {
            ForwardStage::Demo => &self.demo,
            ForwardStage::Shadow => &self.shadow,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "UPPERCASE")]
pub enum GateVerdict {
    Pass,
    Fail,
    Insufficient,
}

impl GateVerdict {
    pub fn as_str(self) -> &'static str {
        match self {
            GateVerdict::Pass => "PASS",
            GateVerdict::Fail => "FAIL",
            GateVerdict::Insufficient => "INSUFFICIENT",
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct GateCheckRow {
    pub id: String,
    pub metric: String,
    pub observed: String,
    pub threshold: String,
    pub verdict: GateVerdict,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct AuditForwardStats {
    pub event_count: usize,
    pub first_ts: Option<Ms>,
    pub last_ts: Option<Ms>,
    pub sl_missing_count: usize,
    pub rule_violation_count: usize,
    pub orphan_reconcile_count: usize,
    pub max_audit_unprotected_ms: i64,
    pub uptime_pct: Option<f64>,
    pub ws_reconnect_count: usize,
    pub ws_backfill_bars: u64,
    pub ws_unfilled_gaps: usize,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ForwardGateReport {
    pub stage: ForwardStage,
    pub since_ms: Ms,
    pub elapsed_days: f64,
    pub trade_count: usize,
    pub slip_diff_mean_r: Option<f64>,
    pub slip_diff_ci95_r: Option<(f64, f64)>,
    pub audit: AuditForwardStats,
    pub rows: Vec<GateCheckRow>,
    pub overall_pass: bool,
}

fn splitmix_next(state: &mut u64) -> u64 {
    *state = state.wrapping_add(0x9E37_79B9_7F4A_7C15);
    let mut z = *state;
    z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
    z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
    z ^ (z >> 31)
}

fn bootstrap_mean_ci95(vals: &[f64], resamples: usize, seed: u64) -> Option<(f64, (f64, f64))> {
    if vals.is_empty() {
        return None;
    }
    let mean = vals.iter().sum::<f64>() / vals.len() as f64;
    if vals.len() == 1 {
        return Some((mean, (mean, mean)));
    }
    let mut rng = seed;
    let mut means = Vec::with_capacity(resamples);
    for _ in 0..resamples {
        let mut s = 0.0;
        for _ in 0..vals.len() {
            let idx = (splitmix_next(&mut rng) % vals.len() as u64) as usize;
            s += vals[idx];
        }
        means.push(s / vals.len() as f64);
    }
    means.sort_by(|a, b| a.total_cmp(b));
    let lo = means[((resamples - 1) * 25) / 1000];
    let hi = means[((resamples - 1) * 975) / 1000];
    Some((mean, (lo, hi)))
}

fn apply_audit_event(st: &mut AuditForwardStats, ev: &str, data: &serde_json::Value) {
    match ev {
        "SL_MISSING" => {
            st.sl_missing_count += 1;
            if let Some(ms) = data.get("elapsed_ms").and_then(|x| x.as_i64()) {
                st.max_audit_unprotected_ms = st.max_audit_unprotected_ms.max(ms);
            }
        }
        "SL_PLACED" => {
            if let Some(ms) = data.get("elapsed_ms").and_then(|x| x.as_i64()) {
                st.max_audit_unprotected_ms = st.max_audit_unprotected_ms.max(ms);
            }
        }
        "RULE_VIOLATION" => st.rule_violation_count += 1,
        "ORPHAN_POSITION" => st.orphan_reconcile_count += 1,
        "WS_RECONNECTED" => st.ws_reconnect_count += 1,
        "WS_GAP_BACKFILLED" => {
            st.ws_reconnect_count += 1;
            st.ws_backfill_bars += data.get("bars").and_then(|x| x.as_u64()).unwrap_or(0);
        }
        "WS_BACKFILL_FAILED" => {
            st.ws_reconnect_count += 1;
            st.ws_unfilled_gaps += 1;
        }
        _ => {}
    }
}

/// Baca file JSONL audit dan ekstrak metrik forward sejak `since_ms`.
pub fn parse_audit_forward_stats(
    audit_path: &Path,
    since_ms: Ms,
    max_gap_ms: i64,
) -> Result<AuditForwardStats, String> {
    if !audit_path.exists() {
        return Ok(AuditForwardStats::default());
    }
    let raw = fs::read_to_string(audit_path)
        .map_err(|e| format!("gagal membaca {}: {e}", audit_path.display()))?;
    let mut st = AuditForwardStats::default();
    let mut ts_list = Vec::new();
    for line in raw.lines().map(str::trim).filter(|l| !l.is_empty()) {
        let Ok(v) = serde_json::from_str::<serde_json::Value>(line) else {
            continue;
        };
        let Some(ts) = v.get("ts").and_then(|x| x.as_i64()) else {
            continue;
        };
        if ts < since_ms {
            continue;
        }
        st.event_count += 1;
        ts_list.push(ts);
        let ev = v.get("event").and_then(|x| x.as_str()).unwrap_or("");
        apply_audit_event(
            &mut st,
            ev,
            v.get("data").unwrap_or(&serde_json::Value::Null),
        );
    }
    ts_list.sort_unstable();
    st.first_ts = ts_list.first().copied();
    st.last_ts = ts_list.last().copied();
    if let (Some(f), Some(l)) = (st.first_ts, st.last_ts) {
        if l > f {
            let active: i64 = ts_list
                .windows(2)
                .map(|w| (w[1] - w[0]).max(0))
                .filter(|&dt| dt <= max_gap_ms)
                .sum();
            st.uptime_pct = Some(100.0 * active as f64 / (l - f) as f64);
        }
    }
    Ok(st)
}

fn row(
    id: &str,
    metric: &str,
    observed: String,
    threshold: String,
    verdict: GateVerdict,
) -> GateCheckRow {
    GateCheckRow {
        id: id.into(),
        metric: metric.into(),
        observed,
        threshold,
        verdict,
    }
}

fn build_slip_row(
    th: &ForwardStageThresholds,
    has_obs: bool,
    slip_len: usize,
    slip_stats: Option<(f64, (f64, f64))>,
) -> GateCheckRow {
    let (obs, thr, verd) = if !th.enforce_slip_diff {
        (
            "N/A (fill paper ter-model)".into(),
            "diukur pada stage demo".into(),
            if has_obs {
                GateVerdict::Pass
            } else {
                GateVerdict::Insufficient
            },
        )
    } else if slip_len < th.min_trades.max(1) {
        (
            format!("n={slip_len} (<{})", th.min_trades.max(1)),
            format!("<= {:.3}R", th.max_mean_slip_diff_r),
            GateVerdict::Insufficient,
        )
    } else if let Some((m, (lo, hi))) = slip_stats {
        (
            format!("{m:+.4}R (CI95 [{lo:+.4}R, {hi:+.4}R])"),
            format!("<= {:.3}R", th.max_mean_slip_diff_r),
            if m <= th.max_mean_slip_diff_r {
                GateVerdict::Pass
            } else {
                GateVerdict::Fail
            },
        )
    } else {
        (
            "n=0".into(),
            format!("<= {:.3}R", th.max_mean_slip_diff_r),
            GateVerdict::Insufficient,
        )
    };
    row(
        "FG-03",
        "mean(realized_slip_R - model_slip_R)",
        obs,
        thr,
        verd,
    )
}

fn count_verdict(has_obs: bool, val: usize, max_ok: usize) -> GateVerdict {
    if !has_obs {
        GateVerdict::Insufficient
    } else if val <= max_ok {
        GateVerdict::Pass
    } else {
        GateVerdict::Fail
    }
}

fn build_ops_rows(
    th: &ForwardStageThresholds,
    has_obs: bool,
    max_unprot: i64,
    audit: &AuditForwardStats,
) -> Vec<GateCheckRow> {
    let unprot_v = if !has_obs {
        GateVerdict::Insufficient
    } else if max_unprot <= th.max_unprotected_ms {
        GateVerdict::Pass
    } else {
        GateVerdict::Fail
    };
    let (up_obs, up_v) = match audit.uptime_pct {
        None => ("INSUFFICIENT (<2 event)".into(), GateVerdict::Insufficient),
        Some(u) if u >= th.min_uptime_pct => (format!("{u:.2}%"), GateVerdict::Pass),
        Some(u) => (format!("{u:.2}%"), GateVerdict::Fail),
    };
    vec![
        row(
            "FG-04",
            "Jumlah SL_MISSING",
            format!("{}", audit.sl_missing_count),
            format!("<= {}", th.max_sl_missing),
            count_verdict(has_obs, audit.sl_missing_count, th.max_sl_missing),
        ),
        row(
            "FG-05",
            "Jumlah RULE_VIOLATION",
            format!("{}", audit.rule_violation_count),
            format!("<= {}", th.max_rule_violation),
            count_verdict(has_obs, audit.rule_violation_count, th.max_rule_violation),
        ),
        row(
            "FG-06",
            "Durasi tanpa proteksi maks",
            format!("{max_unprot} ms"),
            format!("<= {} ms", th.max_unprotected_ms),
            unprot_v,
        ),
        row(
            "FG-07",
            "Jumlah orphan reconcile",
            format!("{}", audit.orphan_reconcile_count),
            format!("<= {}", th.max_orphan_positions),
            count_verdict(
                has_obs,
                audit.orphan_reconcile_count,
                th.max_orphan_positions,
            ),
        ),
        row(
            "FG-08",
            "Uptime berkesinambungan",
            up_obs,
            format!(">= {:.1}%", th.min_uptime_pct),
            up_v,
        ),
        row(
            "FG-09",
            "Celah WS & backfill",
            format!(
                "reconnect={}, backfill_bars={}, gagal={}",
                audit.ws_reconnect_count, audit.ws_backfill_bars, audit.ws_unfilled_gaps
            ),
            format!("gagal <= {}", th.max_unfilled_ws_gaps),
            count_verdict(has_obs, audit.ws_unfilled_gaps, th.max_unfilled_ws_gaps),
        ),
    ]
}

/// Evaluasi gerbang forward dari daftar telemetri dan statistik audit yang sudah dimuat.
pub fn evaluate_forward_gate_data(
    stage: ForwardStage,
    th: &ForwardStageThresholds,
    since_ms: Ms,
    trades: &[TradeTelemetry],
    audit: AuditForwardStats,
) -> ForwardGateReport {
    let want_src = match stage {
        ForwardStage::Demo => ExecSource::Demo,
        ForwardStage::Shadow => ExecSource::Paper,
    };
    let filtered: Vec<&TradeTelemetry> = trades
        .iter()
        .filter(|t| t.closed_ms >= since_ms && t.fills.source == want_src)
        .collect();
    let trade_count = filtered.len();
    let min_ts = [audit.first_ts, filtered.iter().map(|t| t.created_ms).min()]
        .into_iter()
        .flatten()
        .min();
    let max_ts = [audit.last_ts, filtered.iter().map(|t| t.closed_ms).max()]
        .into_iter()
        .flatten()
        .max();
    let elapsed_days = match (min_ts, max_ts) {
        (Some(a), Some(b)) if b > a => (b - a) as f64 / MS_PER_DAY,
        _ => 0.0,
    };
    let slip_diffs: Vec<f64> = filtered
        .iter()
        .filter(|t| !t.fills.paper_slip_zero_by_construction)
        .map(|t| to_f64(t.fills.realized_slip_r - t.fills.model_slip_r))
        .collect();
    let slip_stats = bootstrap_mean_ci95(&slip_diffs, 2_000, 0xA9EC_2026_F04D_0001);
    let max_unprot = filtered
        .iter()
        .filter_map(|t| t.unprotected_ms)
        .max()
        .unwrap_or(0)
        .max(audit.max_audit_unprotected_ms);
    let has_obs = audit.event_count > 0 || trade_count > 0;

    let mut rows = vec![
        row(
            "FG-01",
            "Durasi pengamatan (hari)",
            format!("{elapsed_days:.2} hari"),
            format!(">= {:.1} hari", th.min_days),
            if elapsed_days >= th.min_days {
                GateVerdict::Pass
            } else {
                GateVerdict::Insufficient
            },
        ),
        row(
            "FG-02",
            "Jumlah trade tertutup",
            format!("{trade_count}"),
            format!(">= {}", th.min_trades),
            if trade_count >= th.min_trades {
                GateVerdict::Pass
            } else {
                GateVerdict::Insufficient
            },
        ),
        build_slip_row(th, has_obs, slip_diffs.len(), slip_stats),
    ];
    rows.extend(build_ops_rows(th, has_obs, max_unprot, &audit));
    let overall_pass = rows.iter().all(|r| r.verdict == GateVerdict::Pass);
    ForwardGateReport {
        stage,
        since_ms,
        elapsed_days,
        trade_count,
        slip_diff_mean_r: slip_stats.map(|x| x.0),
        slip_diff_ci95_r: slip_stats.map(|x| x.1),
        audit,
        rows,
        overall_pass,
    }
}

/// Evaluasi gerbang forward langsung dari file `trades_telemetry.jsonl` dan `audit.jsonl`.
pub fn evaluate_forward_gate_files(
    stage: ForwardStage,
    gate_cfg_path: &Path,
    telemetry_path: &Path,
    audit_path: &Path,
    since_ms: Ms,
) -> Result<ForwardGateReport, String> {
    let gate_file = ForwardGateFile::load(gate_cfg_path)?;
    let th = gate_file.thresholds_for(stage);
    let trades = read_telemetry_jsonl(telemetry_path).map_err(|e| e.to_string())?;
    let max_gap_ms = (th.max_heartbeat_gap_secs as i64).saturating_mul(1_000);
    let audit = parse_audit_forward_stats(audit_path, since_ms, max_gap_ms)?;
    Ok(evaluate_forward_gate_data(
        stage, th, since_ms, &trades, audit,
    ))
}
