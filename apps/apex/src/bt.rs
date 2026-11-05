//! `apex backtest` dan `apex validate` (Rules §4.3 G-01..G-11, parsial).
use crate::cfg::AllConfig;
use apex_backtest::*;
use apex_core::time::{fmt_utc, parse_ymd};
use apex_core::types::Tf;
use apex_data::csvio::{funding_path, kline_path, load_funding, load_klines};
use apex_strategies::StrategyId;
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

#[derive(Clone)]
pub struct BtArgs {
    pub from: String,
    pub to: String,
    pub strategies: Option<String>,
    pub data: PathBuf,
    pub out: PathBuf,
    pub regime: String,
    pub stress: bool,
    pub unlock_holdout: bool,
    pub filters: Option<PathBuf>,
    pub no_volume_gate: bool,
    /// Pakai grid CHOP yang sudah tersimpan di validation_summary.json. Tidak menyentuh holdout.
    pub gates_only: bool,
}

pub fn load_market(cfg: &AllConfig, data: &Path, filters_path: &Path) -> anyhow::Result<MarketInput> {
    let filters = crate::cfg::load_filters(filters_path)?;
    let metadata: serde_json::Value = serde_json::from_str(&std::fs::read_to_string(filters_path)?)?;
    let onboarding: BTreeMap<&str, i64> = metadata["symbols"].as_array().into_iter().flatten().filter_map(|s| Some((s["symbol"].as_str()?, s["onboardDate"].as_i64()?))).collect();
    let allow = crate::cfg::load_allowlist(&cfg.dir)?;
    let btc_m15 = load_klines(&kline_path(data, "BTCUSDT", Tf::M15))?;
    let btc_m5 = if cfg.strategies.research_entry_5m { Some(load_klines(&kline_path(data, "BTCUSDT", Tf::M5))?) } else { None };
    let btc_h1 = load_klines(&kline_path(data, "BTCUSDT", Tf::H1))?;
    let dom = load_klines(&kline_path(data, "BTCDOMUSDT", Tf::H1)).ok().filter(|v| !v.is_empty());
    let mut symbols = Vec::new();
    for s in cfg.universe() {
        if s == "BTCUSDT" || s == "BTCDOMUSDT" {
            continue;
        }
        let (Ok(m15), Ok(h1)) = (load_klines(&kline_path(data, &s, Tf::M15)), load_klines(&kline_path(data, &s, Tf::H1))) else {
            eprintln!("  [lewati] {s}: CSV klines tidak ada (jalankan `apex fetch`)");
            continue;
        };
        let Some(f) = filters.get(&s).cloned() else {
            eprintln!("  [lewati] {s}: filter exchangeInfo tidak ada");
            continue;
        };
        let funding = load_funding(&funding_path(data, &s)).map(|v| v.into_iter().map(|r| (r.time, r.rate)).collect()).unwrap_or_default();
        let tradeable = allow.contains(&s);
        if !tradeable {
            eprintln!("  [konteks saja] {s}: tidak ada di allowlist crypto");
        }
        let m5 = if cfg.strategies.research_entry_5m { Some(load_klines(&kline_path(data, &s, Tf::M5))?) } else { None };
        let onboard_ms = onboarding.get(s.as_str()).copied();
        symbols.push(SymbolInput { symbol: s.clone(), m15, m5, h1, onboard_ms, funding, filters: f, tradeable });
    }
    Ok(MarketInput { symbols, btc_m15, btc_m5, btc_h1, dom_h1: dom })
}

pub fn make_cfg(cfg: &AllConfig, a: &BtArgs, ids: Vec<StrategyId>) -> anyhow::Result<BacktestConfig> {
    let start = parse_ymd(&a.from).ok_or_else(|| anyhow::anyhow!("--from YYYY-MM-DD"))?;
    let end = parse_ymd(&a.to).ok_or_else(|| anyhow::anyhow!("--to YYYY-MM-DD"))?;
    let mut c = BacktestConfig {
        start,
        end,
        risk: cfg.risk.clone(),
        params: cfg.strategies.params.clone(),
        strategies: ids,
        screener: cfg.screener.regime.clone(),
        chop: cfg.screener.chop,
        regime_mode: if a.regime == "screener" { RegimeMode::ScreenerOnly } else { RegimeMode::Reader },
        entry_tf: if cfg.strategies.research_entry_5m { Tf::M5 } else { Tf::M15 },
        volume_gate: !a.no_volume_gate,
        min_quote_volume_24h: cfg.screener.volume_gate.min_quote_volume_24h,
        min_trades_24h: cfg.screener.volume_gate.min_trades_24h,
        ..Default::default()
    };
    if a.stress {
        stress(&mut c);
    }
    Ok(c)
}

/// G-05: fee 2×, slip +0,05%, funding 2×.
pub fn stress(c: &mut BacktestConfig) {
    c.fee_mult = 2.0;
    c.extra_slip = 0.0005;
    c.funding_mult = 2.0;
}

fn parse_ids(s: &Option<String>, cfg: &AllConfig) -> Vec<StrategyId> {
    match s {
        Some(s) => s.split(',').filter_map(|x| StrategyId::parse(x.trim())).collect(),
        None => cfg.enabled(),
    }
}

pub fn print_metrics(name: &str, m: &Metrics) {
    println!(
        "  {name:<6} trades={:>4} WR={:>5.1}% PF={:>5.2} E[R]={:>+6.3} net=${:>+7.2} maxDD=${:>6.2} ({:.1}R) SL={} TP={} fee=${:.2} fund=${:.2} streak={}",
        m.trades,
        m.win_rate * 100.0,
        if m.profit_factor.is_finite() { m.profit_factor } else { 99.0 },
        m.expectancy_r,
        m.net_usd,
        m.max_dd_usd,
        m.max_dd_r,
        m.sl_exits,
        m.tp_exits,
        m.fees_usd,
        m.funding_usd,
        m.longest_loss_streak
    );
}

pub fn backtest(cfg: &AllConfig, a: &BtArgs, per_strategy: bool) -> anyhow::Result<()> {
    let fpath = a.filters.clone().unwrap_or_else(|| cfg.dir.join("filters_snapshot.json"));
    let mkt = load_market(cfg, &a.data, &fpath)?;
    let ids = parse_ids(&a.strategies, cfg);
    let runs: Vec<Vec<StrategyId>> = if per_strategy { ids.iter().map(|i| vec![*i]).chain(std::iter::once(ids.clone())).collect() } else { vec![ids.clone()] };
    std::fs::create_dir_all(&a.out)?;
    let ledger = a.out.join("trials_ledger.jsonl");
    for run in runs {
        let c = make_cfg(cfg, a, run.clone())?;
        let hash = param_hash(&serde_json::to_value(&c)?);
        let ho = holdout_guard(c.start, c.end, a.unlock_holdout, &hash, &ledger)?;
        let t0 = std::time::Instant::now();
        let r = run_backtest(&mkt, &c)?;
        append_ledger(&ledger, &hash, &c, ho, &r.metrics, "backtest")?;
        let tag: Vec<&str> = run.iter().map(|s| s.code()).collect();
        let tag = tag.join("+");
        println!("\n== Backtest {} | {} → {} | regime={:?}{} | {:.1}s", tag, fmt_utc(c.start), fmt_utc(c.end), c.regime_mode, if a.stress { " | STRESS" } else { "" }, t0.elapsed().as_secs_f64());
        print_metrics("ALL", &r.metrics);
        if run.len() > 1 {
            for (k, m) in &r.per_strategy {
                print_metrics(k, m);
            }
        }
        println!("  sinyal={:?}", r.signals);
        println!("  skip={:?}", r.skips);
        println!("  regime bars={:?} | screener trend share={:.1}% | liquidated={} | wallet akhir=${:.2}", r.regime_bars, r.screener_trend_share * 100.0, r.liquidated, r.final_wallet);
        let base = a.out.join(format!("bt_{}_{}", tag.replace('+', "_"), hash));
        std::fs::write(base.with_extension("json"), serde_json::to_vec_pretty(&r)?)?;
        let mut w = csv::Writer::from_path(base.with_extension("trades.csv"))?;
        w.write_record(["symbol", "strategy", "side", "entry_utc", "exit_utc", "entry", "sl", "tp", "exit", "qty", "fees", "funding", "pnl_net", "r", "exit_kind", "regime"])?;
        for t in &r.trades {
            w.write_record([t.symbol.clone(), t.strategy.clone(), format!("{:?}", t.side), fmt_utc(t.entry_ms), fmt_utc(t.exit_ms), t.entry.to_string(), t.sl.to_string(), t.tp.to_string(), t.exit.to_string(), t.qty.to_string(), format!("{:.4}", t.fees), format!("{:.4}", t.funding), format!("{:.4}", t.pnl_net), format!("{:.3}", t.r), format!("{:?}", t.exit_kind), t.regime.clone()])?;
        }
        w.flush()?;
        println!("  laporan: {}", base.with_extension("json").display());
    }
    Ok(())
}

pub fn append_ledger(ledger: &Path, hash: &str, c: &BacktestConfig, holdout: bool, m: &Metrics, kind: &str) -> anyhow::Result<()> {
    append_ledger_with(ledger, hash, c, holdout, m, None, kind)
}

fn append_ledger_with(
    ledger: &Path,
    hash: &str,
    c: &BacktestConfig,
    holdout: bool,
    m: &Metrics,
    per_strategy: Option<&BTreeMap<String, Metrics>>,
    kind: &str,
) -> anyhow::Result<()> {
    use std::io::Write;
    let mut line = serde_json::json!({
        "ts": apex_core::time::now_ms(),
        "param_hash": hash,
        "strategies": c.strategies.iter().map(|s| s.code()).collect::<Vec<_>>(),
        "start": fmt_utc(c.start), "end": fmt_utc(c.end),
        "holdout": holdout, "stress": c.fee_mult > 1.0,
        "kind": kind,
        "trades": m.trades, "expectancy_r": m.expectancy_r,
        "pf": if m.profit_factor.is_finite() { m.profit_factor } else { 99.0 },
        "chop_adx_max": c.chop.adx_max,
        "chop_er_max": c.chop.er_max,
        "chop_trend_share_min": c.chop.trend_share_min,
        "chop_range": c.screener.chop_range,
        "chop_trend": c.screener.chop_trend,
    });
    if let Some(ps) = per_strategy {
        let map: serde_json::Map<String, serde_json::Value> = ps
            .iter()
            .map(|(k, v)| (k.clone(), serde_json::json!(v.trades)))
            .collect();
        line["strategy_trades"] = serde_json::Value::Object(map);
    }
    let mut f = std::fs::OpenOptions::new().create(true).append(true).open(ledger)?;
    writeln!(f, "{line}")?;
    f.sync_data()?;
    Ok(())
}

/// Gate dengan status INCONCLUSIVE saat sampel kurang (Rules G-02: kurang -> INCONCLUSIVE).
/// sample_ok=false -> label INCONCLUSIVE dan tidak boleh dihitung lulus (tidak boleh klaim PASS).
fn gate_v(name: &str, ok: bool, sample_ok: bool, detail: String) -> bool {
    let tag = if !sample_ok { "INCONCLUSIVE" } else if ok { "PASS" } else { "FAIL" };
    println!("  [{tag}] {name}: {detail}");
    ok && sample_ok
}

/// Independent segment: signal, entry and real exit must all be within [from, to).
/// EndOfData is a mark-to-market, not a completed trade.
fn seg_trades(trades: &[Trade], from: i64, to: i64) -> Vec<Trade> {
    trades.iter().filter(|t| t.signal_ms >= from && t.signal_ms <= t.entry_ms && t.entry_ms <= t.exit_ms && t.entry_ms >= from && t.exit_ms >= from && t.exit_ms < to && t.exit_kind != apex_core::risk::ExitKind::EndOfData).cloned().collect()
}

fn fnum(x: f64) -> String {
    if x.is_finite() {
        format!("{x:.6}")
    } else if x.is_nan() {
        "nan".into()
    } else if x.is_sign_positive() {
        "inf".into()
    } else {
        "-inf".into()
    }
}

fn pf_print(x: f64) -> String {
    if x.is_finite() { format!("{x:.2}") } else if x.is_sign_positive() { "inf".into() } else { "nan".into() }
}

/// Rules §4.3 validation boundary: IS berakhir 2025-07-01, VAL mulai 2025-07-01 s.d. 2026-05-01.
const FIT_FROM: &str = "2025-01-01";
const VAL_FROM: &str = "2025-07-01";
const FIT_TO: &str = VAL_FROM;
const OOS_FROM: &str = VAL_FROM;
const OOS_TO: &str = "2026-05-01";
const TREND_GRID: [f64; 7] = [0.0, 5.0, 10.0, 15.0, 20.0, 25.0, 30.0];
const CHOP_RANGE_GRID: [f64; 7] = [45.0, 50.0, 55.0, 60.0, 65.0, 70.0, 75.0];
/// Knob kedua klausa CHOP regime: ADX 1h (default 15).
const ADX_GRID: [f64; 7] = [6.0, 9.0, 12.0, 15.0, 18.0, 24.0, 30.0];
/// Knob ketiga klausa CHOP regime: ER 15m (default 0,25).
const ER_GRID: [f64; 7] = [0.10, 0.15, 0.20, 0.25, 0.30, 0.35, 0.40];

struct Session {
    runs: BTreeMap<String, usize>,
    srs: BTreeMap<String, Vec<f64>>,
}

struct Row {
    threshold: f64,
    fit_block: f64,
    oos_block: f64,
    fit_chop: f64,
    oos_chop: f64,
    fit_non_trade: f64,
    oos_non_trade: f64,
    fit_screen: f64,
    oos_screen: f64,
    fit_e: f64,
    oos_e: f64,
    fit_pf: f64,
    oos_pf: f64,
    fit_trades: usize,
    oos_trades: usize,
    fit_e_by: BTreeMap<String, f64>,
    oos_e_by: BTreeMap<String, f64>,
    oos_pf_by: BTreeMap<String, f64>,
}

fn window_cfg(cfg: &AllConfig, a: &BtArgs, ids: Vec<StrategyId>, from: &str, to: &str) -> anyhow::Result<BacktestConfig> {
    let mut b = a.clone();
    b.from = from.to_string();
    b.to = to.to_string();
    b.stress = false;
    b.unlock_holdout = false;
    make_cfg(cfg, &b, ids)
}

/// SR satu trial dari return per trade ternormalisasi (R-multiple, `t.r` = pnl/risk).
/// Trial dengan <10 trade dibuang: SR small-sample (sd->0) bisa ratusan dan membuat
/// var_sr antar trial meledak (definisi trial DSR, 2026-11-05).
fn trial_sr(rs: &[f64]) -> Option<f64> {
    if rs.len() < 10 {
        return None;
    }
    Some(sharpe_ratio(rs))
}

fn note_session(session: &mut Session, c: &BacktestConfig, r: &BacktestReport) {
    for id in &c.strategies {
        let rs: Vec<f64> = r.trades.iter().filter(|t| t.strategy == id.code()).map(|t| t.r).collect();
        if let Some(sr) = trial_sr(&rs) {
            *session.runs.entry(id.code().to_string()).or_insert(0) += 1;
            session.srs.entry(id.code().to_string()).or_default().push(sr);
        }
    }
}

/// Satu backtest. `unlock` hanya untuk jalur G-11; sweep dan fold selalu `false` supaya holdout tidak ikut grid.
fn exec(mkt: &MarketInput, c: &BacktestConfig, ledger: &Path, kind: &str, session: &mut Session, unlock: bool) -> anyhow::Result<(BacktestReport, bool)> {
    let hash = param_hash(&serde_json::to_value(c)?);
    let ho = holdout_guard(c.start, c.end, unlock, &hash, ledger)?;
    let t0 = std::time::Instant::now();
    eprintln!("  … {kind} {} → {}", fmt_utc(c.start), fmt_utc(c.end));
    let r = run_backtest(mkt, c)?;
    // Persist before exposing results so an unrecorded trial cannot inform selection.
    append_ledger_with(ledger, &hash, c, ho, &r.metrics, Some(&r.per_strategy), kind)?;
    eprintln!(
        "    {:.1}s trades={} E={:+.3}R PF={} chop={:.1}% nontrade={:.1}%",
        t0.elapsed().as_secs_f64(),
        r.metrics.trades,
        r.metrics.expectancy_r,
        pf_print(r.metrics.profit_factor),
        chop_block_rate(&r.regime_bars) * 100.0,
        non_trade_rate(&r.regime_bars) * 100.0
    );
    note_session(session, c, &r);
    Ok((r, ho))
}

/// Tuning CHOP ulang hanya pada data train fold [train_start, train_end).
/// Tidak menyentuh data test fold [test_start, test_end).
fn refit_chop_fold(
    mkt: &MarketInput,
    base_cfg: &BacktestConfig,
    train_start: i64,
    train_end: i64,
) -> anyhow::Result<(apex_market_reader::ChopThresholds, Metrics)> {
    let mut tc = base_cfg.clone();
    tc.start = train_start;
    tc.end = train_end;
    tc.fee_mult = 1.0;
    tc.funding_mult = 1.0;
    tc.extra_slip = 0.0;

    let mut trend_pts = Vec::with_capacity(TREND_GRID.len());
    for th in TREND_GRID {
        let mut c = tc.clone();
        c.chop.trend_share_min = th;
        let r = run_backtest(mkt, &c)?;
        trend_pts.push(SweepPoint {
            threshold: th,
            fit_block: chop_block_rate(&r.regime_bars),
            oos_pf: r.metrics.profit_factor,
            fit_e: r.metrics.expectancy_r,
        });
    }
    let trend_share_min = refit_threshold_from_train(&trend_pts, tc.chop.trend_share_min);

    let mut adx_pts = Vec::with_capacity(ADX_GRID.len());
    for th in ADX_GRID {
        let mut c = tc.clone();
        c.chop.adx_max = th;
        let r = run_backtest(mkt, &c)?;
        adx_pts.push(SweepPoint {
            threshold: th,
            fit_block: chop_block_rate(&r.regime_bars),
            oos_pf: r.metrics.profit_factor,
            fit_e: r.metrics.expectancy_r,
        });
    }
    let adx_max = refit_threshold_from_train(&adx_pts, tc.chop.adx_max);

    let mut er_pts = Vec::with_capacity(ER_GRID.len());
    for th in ER_GRID {
        let mut c = tc.clone();
        c.chop.er_max = th;
        let r = run_backtest(mkt, &c)?;
        er_pts.push(SweepPoint {
            threshold: th,
            fit_block: chop_block_rate(&r.regime_bars),
            oos_pf: r.metrics.profit_factor,
            fit_e: r.metrics.expectancy_r,
        });
    }
    let er_max = refit_threshold_from_train(&er_pts, tc.chop.er_max);

    let refitted = apex_market_reader::ChopThresholds { adx_max, er_max, trend_share_min };
    tc.chop = refitted;
    let train_rep = run_backtest(mkt, &tc)?;
    Ok((refitted, train_rep.metrics))
}

fn row_from(threshold: f64, fit: &BacktestReport, oos: &BacktestReport, block: impl Fn(&BacktestReport) -> f64) -> Row {
    let emap = |r: &BacktestReport| r.per_strategy.iter().map(|(k, m)| (k.clone(), m.expectancy_r)).collect();
    let pmap = |r: &BacktestReport| r.per_strategy.iter().map(|(k, m)| (k.clone(), m.profit_factor)).collect();
    Row {
        threshold,
        fit_block: block(fit),
        oos_block: block(oos),
        fit_chop: chop_block_rate(&fit.regime_bars),
        oos_chop: chop_block_rate(&oos.regime_bars),
        fit_non_trade: non_trade_rate(&fit.regime_bars),
        oos_non_trade: non_trade_rate(&oos.regime_bars),
        fit_screen: (1.0 - fit.screener_trend_share).clamp(0.0, 1.0),
        oos_screen: (1.0 - oos.screener_trend_share).clamp(0.0, 1.0),
        fit_e: fit.metrics.expectancy_r,
        oos_e: oos.metrics.expectancy_r,
        fit_pf: fit.metrics.profit_factor,
        oos_pf: oos.metrics.profit_factor,
        fit_trades: fit.metrics.trades,
        oos_trades: oos.metrics.trades,
        fit_e_by: emap(fit),
        oos_e_by: emap(oos),
        oos_pf_by: pmap(oos),
    }
}

fn nearest<'a>(rows: &'a [Row], threshold: f64) -> &'a Row {
    rows.iter().min_by(|a, b| (a.threshold - threshold).abs().total_cmp(&(b.threshold - threshold))).expect("grid kosong")
}

fn sweep_points(rows: &[Row]) -> Vec<SweepPoint> {
    rows.iter().map(|r| SweepPoint { threshold: r.threshold, fit_block: r.fit_block, oos_pf: r.oos_pf, fit_e: r.fit_e }).collect()
}

fn print_plateau(label: &str, rows: &[Row], choice: &PlateauChoice, baseline: f64) {
    println!("\n== Plateau {label} | baseline={baseline} | blok FIT < 87%, PF OOS tidak turun, G-09 pada VAL (E_OOS)");
    println!("  {:<8} {:>8} {:>8} {:>8} {:>8} {:>7} {:>7}", "th", "blokFIT", "blokOOS", "E_fit", "PF_oos", "trFIT", "trOOS");
    for (i, r) in rows.iter().enumerate() {
        let mark = if choice.run.contains(&i) { "*" } else { " " };
        println!("  {mark}{:<7} {:>7.1}% {:>7.1}% {:>+8.3} {:>8} {:>7} {:>7}", r.threshold, r.fit_block * 100.0, r.oos_block * 100.0, r.fit_e, pf_print(r.oos_pf), r.fit_trades, r.oos_trades);
    }
    let adopted = choice.adopted_threshold.map(|t| format!("{t}")).unwrap_or_else(|| "tidak ada".into());
    println!("  admissible={} run={} adopted={adopted} (median run, bukan PF tertinggi)", choice.admissible.len(), choice.run.len());
}

fn rows_json(rows: &[Row]) -> serde_json::Value {
    serde_json::Value::Array(rows.iter().map(|r| {
        serde_json::json!({
            "threshold": r.threshold,
            "fit_block": fnum(r.fit_block),
            "oos_block": fnum(r.oos_block),
            "fit_chop": fnum(r.fit_chop),
            "oos_chop": fnum(r.oos_chop),
            "fit_non_trade": fnum(r.fit_non_trade),
            "oos_non_trade": fnum(r.oos_non_trade),
            "fit_screen_block": fnum(r.fit_screen),
            "oos_screen_block": fnum(r.oos_screen),
            "fit_e": fnum(r.fit_e),
            "oos_e": fnum(r.oos_e),
            "fit_pf": fnum(r.fit_pf),
            "oos_pf": fnum(r.oos_pf),
            "fit_trades": r.fit_trades,
            "oos_trades": r.oos_trades,
            "fit_e_by": r.fit_e_by.iter().map(|(k, v)| (k.clone(), serde_json::Value::String(fnum(*v)))).collect::<serde_json::Map<String, serde_json::Value>>(),
            "oos_e_by": r.oos_e_by.iter().map(|(k, v)| (k.clone(), serde_json::Value::String(fnum(*v)))).collect::<serde_json::Map<String, serde_json::Value>>(),
            "oos_pf_by": r.oos_pf_by.iter().map(|(k, v)| (k.clone(), serde_json::Value::String(fnum(*v)))).collect::<serde_json::Map<String, serde_json::Value>>(),
        })
    }).collect())
}

fn parse_num(v: &serde_json::Value) -> f64 {
    match v {
        serde_json::Value::String(s) if s == "inf" => f64::INFINITY,
        serde_json::Value::String(s) if s == "-inf" => f64::NEG_INFINITY,
        serde_json::Value::String(s) => s.parse().unwrap_or(f64::NAN),
        serde_json::Value::Number(n) => n.as_f64().unwrap_or(f64::NAN),
        _ => f64::NAN,
    }
}

fn map_num(v: &serde_json::Value) -> BTreeMap<String, f64> {
    v.as_object().map(|o| o.iter().map(|(k, x)| (k.clone(), parse_num(x))).collect()).unwrap_or_default()
}

fn rows_from_json(v: &serde_json::Value) -> anyhow::Result<Vec<Row>> {
    if v.is_null() {
        return Ok(Vec::new());
    }
    let arr = v.as_array().ok_or_else(|| anyhow::anyhow!("grid CHOP di summary bukan array"))?;
    Ok(arr.iter().map(|r| Row {
        threshold: parse_num(&r["threshold"]),
        fit_block: parse_num(&r["fit_block"]),
        oos_block: parse_num(&r["oos_block"]),
        fit_chop: parse_num(&r["fit_chop"]),
        oos_chop: parse_num(&r["oos_chop"]),
        fit_non_trade: parse_num(&r["fit_non_trade"]),
        oos_non_trade: parse_num(&r["oos_non_trade"]),
        fit_screen: parse_num(&r["fit_screen_block"]),
        oos_screen: parse_num(&r["oos_screen_block"]),
        fit_e: parse_num(&r["fit_e"]),
        oos_e: parse_num(&r["oos_e"]),
        fit_pf: parse_num(&r["fit_pf"]),
        oos_pf: parse_num(&r["oos_pf"]),
        fit_trades: r["fit_trades"].as_u64().unwrap_or(0) as usize,
        oos_trades: r["oos_trades"].as_u64().unwrap_or(0) as usize,
        fit_e_by: map_num(&r["fit_e_by"]),
        oos_e_by: map_num(&r["oos_e_by"]),
        oos_pf_by: map_num(&r["oos_pf_by"]),
    }).collect())
}

fn choice_json(c: &PlateauChoice) -> serde_json::Value {
    serde_json::json!({
        "admissible": c.admissible,
        "run": c.run,
        "adopted_threshold": c.adopted_threshold.map(fnum),
    })
}

/// Validasi Rules §4.3 pada jendela yang diminta.
/// IS berakhir 2025-07-01, VAL mulai 2025-07-01 s.d. 2026-05-01 (Rules §4.3).
/// Tuning CHOP memakai FIT 2025-01-01..2025-07-01 dan VAL/OOS 2025-07-01..2026-05-01. Holdout tidak dijalankan.
pub fn validate(cfg: &AllConfig, a: &BtArgs) -> anyhow::Result<()> {
    let mut a = a.clone();
    a.stress = false;
    let fpath = a.filters.clone().unwrap_or_else(|| cfg.dir.join("filters_snapshot.json"));
    let mkt = load_market(cfg, &a.data, &fpath)?;
    if mkt.btc_m15.is_empty() {
        anyhow::bail!("data BTCUSDT kosong — jalankan `apex fetch` dulu");
    }
    if !mkt.symbols.iter().any(|s| s.tradeable) {
        anyhow::bail!("tidak ada simbol tradeable di data — jalankan `apex fetch` dulu");
    }
    std::fs::create_dir_all(&a.out)?;
    let ledger_path = a.out.join("trials_ledger.jsonl");
    let ids = parse_ids(&a.strategies, cfg);
    if ids.is_empty() {
        anyhow::bail!("tidak ada strategi");
    }
    let probe = make_cfg(cfg, &a, ids.clone())?;
    if overlaps_holdout(probe.start, probe.end) && !a.unlock_holdout {
        anyhow::bail!(
            "rentang menyentuh HOLDOUT terkunci 2026-05-01..2026-10-31. Pakai --unlock-holdout (sekali jalan per versi parameter, G-11)."
        );
    }
    let prior_txt = std::fs::read_to_string(&ledger_path).unwrap_or_default();
    let priors: BTreeMap<String, usize> = ids.iter().map(|id| (id.code().to_string(), count_strategy_trials(&prior_txt, id.code()))).collect();
    let mut session = Session { runs: BTreeMap::new(), srs: BTreeMap::new() };
    let mut summary = serde_json::json!({
        "window": {"from": a.from, "to": a.to},
        "fit": [FIT_FROM, FIT_TO],
        "oos": [OOS_FROM, OOS_TO],
        "val_from": VAL_FROM,
        "holdout": "2026-05-01..2026-10-31 locked",
        "strategies": ids.iter().map(|s| s.code()).collect::<Vec<_>>(),
    });

    if overlaps_holdout(probe.start, probe.end) {
        println!("Jendela menyentuh HOLDOUT. Hanya G-11 sekali jalan. Sweep dan walk-forward tidak dijalankan.");
        let mut gates = Vec::new();
        for id in ids {
            let c = make_cfg(cfg, &a, vec![id])?;
            let (r, ho) = exec(&mkt, &c, &ledger_path, "holdout", &mut session, true)?;
            let holdout_start = parse_ymd(apex_backtest::HOLDOUT_START).ok_or_else(|| anyhow::anyhow!("invalid holdout start"))?;
            let holdout_end = parse_ymd(apex_backtest::HOLDOUT_END_EXCL).ok_or_else(|| anyhow::anyhow!("invalid holdout end"))?;
            let m = Metrics::from_trades(&seg_trades(&r.trades, holdout_start, holdout_end), cfg.risk.start_balance_usd);
            println!("\n== G-11 {} | {} → {}", id.code(), fmt_utc(c.start), fmt_utc(c.end));
            print_metrics("holdout", &m);
            let ok = ho && m.trades >= 40 && g11_pass(m.expectancy_r, m.profit_factor);
            gate_v("G-11 holdout trades≥40 & E>0 & PF≥1.10 (Rules G-02: HOLDOUT≥40)", ok, m.trades >= 40, format!("{} trade (min 40), {:+.3}R / {}", m.trades, m.expectancy_r, pf_print(m.profit_factor)));
            println!("  ==> {} : NO-GO{}", id.code(), if ok { " (G-11 lolos angka; status tetap NO-GO sampai review user)" } else { "" });
            gates.push(serde_json::json!({"strategy": id.code(), "g11": ok, "e": fnum(m.expectancy_r), "pf": fnum(m.profit_factor), "trades": m.trades}));
        }
        summary["g11"] = serde_json::Value::Array(gates);
        let path = a.out.join("validation_summary.json");
        std::fs::write(&path, serde_json::to_vec_pretty(&summary)?)?;
        println!("ringkasan: {}", path.display());
        return Ok(());
    }

    let summary_path = a.out.join("validation_summary.json");
    let (trend_rows, screen_rows, adx_rows, er_rows) = if a.gates_only {
        let prev: serde_json::Value = serde_json::from_str(&std::fs::read_to_string(&summary_path).map_err(|_| anyhow::anyhow!("--gates-only butuh {}", summary_path.display()))?)?;
        println!("Sweep dilewati. Grid dibaca dari {}", summary_path.display());
        (rows_from_json(&prev["regime_plateau"]["rows"])?, rows_from_json(&prev["screener_plateau"]["rows"])?, rows_from_json(&prev["adx_plateau"]["rows"])?, rows_from_json(&prev["er_plateau"]["rows"])?)
    } else {
        println!("Sweep CHOP regime (trend_share_min) dan CHOP screener (chop_range). FIT {FIT_FROM}→{FIT_TO}, VAL/OOS {OOS_FROM}→{OOS_TO}. Holdout tidak disentuh.");
        let mut trend_rows = Vec::new();
        for th in TREND_GRID {
            let mut fit = window_cfg(cfg, &a, ids.clone(), FIT_FROM, FIT_TO)?;
            let mut oos = window_cfg(cfg, &a, ids.clone(), OOS_FROM, OOS_TO)?;
            fit.chop.trend_share_min = th;
            oos.chop.trend_share_min = th;
            println!("\n[regime trend_share_min={th}]");
            let (fr, _) = exec(&mkt, &fit, &ledger_path, "chop_regime_fit", &mut session, false)?;
            let (or_, _) = exec(&mkt, &oos, &ledger_path, "chop_regime_oos", &mut session, false)?;
            trend_rows.push(row_from(th, &fr, &or_, |r| chop_block_rate(&r.regime_bars)));
        }
        let mut screen_rows = Vec::new();
        for th in CHOP_RANGE_GRID {
            let mut fit = window_cfg(cfg, &a, ids.clone(), FIT_FROM, FIT_TO)?;
            let mut oos = window_cfg(cfg, &a, ids.clone(), OOS_FROM, OOS_TO)?;
            fit.screener.chop_range = th;
            oos.screener.chop_range = th;
            let trend = (th - 10.0).max(0.0);
            fit.screener.chop_trend = trend;
            oos.screener.chop_trend = trend;
            println!("\n[screener chop_range={th} chop_trend={trend}]");
            let (fr, _) = exec(&mkt, &fit, &ledger_path, "chop_screen_fit", &mut session, false)?;
            let (or_, _) = exec(&mkt, &oos, &ledger_path, "chop_screen_oos", &mut session, false)?;
            screen_rows.push(row_from(th, &fr, &or_, |r| (1.0 - r.screener_trend_share).clamp(0.0, 1.0)));
        }
        // Knob ADX & ER dari klausa CHOP regime (trend_share_min & knob lain tetap baseline).
        let mut adx_rows = Vec::new();
        for th in ADX_GRID {
            let mut fit = window_cfg(cfg, &a, ids.clone(), FIT_FROM, FIT_TO)?;
            let mut oos = window_cfg(cfg, &a, ids.clone(), OOS_FROM, OOS_TO)?;
            fit.chop.adx_max = th;
            oos.chop.adx_max = th;
            println!("\n[regime adx_max={th}]");
            let (fr, _) = exec(&mkt, &fit, &ledger_path, "chop_adx_fit", &mut session, false)?;
            let (or_, _) = exec(&mkt, &oos, &ledger_path, "chop_adx_oos", &mut session, false)?;
            adx_rows.push(row_from(th, &fr, &or_, |r| chop_block_rate(&r.regime_bars)));
        }
        let mut er_rows = Vec::new();
        for th in ER_GRID {
            let mut fit = window_cfg(cfg, &a, ids.clone(), FIT_FROM, FIT_TO)?;
            let mut oos = window_cfg(cfg, &a, ids.clone(), OOS_FROM, OOS_TO)?;
            fit.chop.er_max = th;
            oos.chop.er_max = th;
            println!("\n[regime er_max={th}]");
            let (fr, _) = exec(&mkt, &fit, &ledger_path, "chop_er_fit", &mut session, false)?;
            let (or_, _) = exec(&mkt, &oos, &ledger_path, "chop_er_oos", &mut session, false)?;
            er_rows.push(row_from(th, &fr, &or_, |r| chop_block_rate(&r.regime_bars)));
        }
        (trend_rows, screen_rows, adx_rows, er_rows)
    };

    let regime_base = nearest(&trend_rows, probe.chop.trend_share_min);
    let screen_base = nearest(&screen_rows, probe.screener.chop_range);
    let regime_choice = select_plateau(&sweep_points(&trend_rows), regime_base.threshold, regime_base.oos_pf);
    let screen_choice = select_plateau(&sweep_points(&screen_rows), screen_base.threshold, screen_base.oos_pf);
    print_plateau("regime trend_share_min", &trend_rows, &regime_choice, regime_base.threshold);
    print_plateau("screener chop_range", &screen_rows, &screen_choice, screen_base.threshold);
    // Plateau knob ADX & ER. Kosong bila --gates-only dengan summary lama (grid belum ada).
    let adx_choice = if adx_rows.is_empty() {
        None
    } else {
        let base = nearest(&adx_rows, probe.chop.adx_max);
        let c = select_plateau(&sweep_points(&adx_rows), base.threshold, base.oos_pf);
        print_plateau("regime adx_max", &adx_rows, &c, base.threshold);
        summary["adx_plateau"] = serde_json::json!({"rows": rows_json(&adx_rows), "choice": choice_json(&c), "baseline": base.threshold});
        c.adopted_threshold
    };
    let er_choice = if er_rows.is_empty() {
        None
    } else {
        let base = nearest(&er_rows, probe.chop.er_max);
        let c = select_plateau(&sweep_points(&er_rows), base.threshold, base.oos_pf);
        print_plateau("regime er_max", &er_rows, &c, base.threshold);
        summary["er_plateau"] = serde_json::json!({"rows": rows_json(&er_rows), "choice": choice_json(&c), "baseline": base.threshold});
        c.adopted_threshold
    };
    // Verifikasi kombinasi: ketiga knob regime memakai median plateau masing-masing (bukan titik terbaik).
    if adx_choice.is_some() && er_choice.is_some() {
        let combo_th = regime_choice.adopted_threshold.unwrap_or(probe.chop.trend_share_min);
        let combo_adx = adx_choice.unwrap_or(probe.chop.adx_max);
        let combo_er = er_choice.unwrap_or(probe.chop.er_max);
        let mut fit = window_cfg(cfg, &a, ids.clone(), FIT_FROM, FIT_TO)?;
        let mut oos = window_cfg(cfg, &a, ids.clone(), OOS_FROM, OOS_TO)?;
        for c in [&mut fit, &mut oos] {
            c.chop.trend_share_min = combo_th;
            c.chop.adx_max = combo_adx;
            c.chop.er_max = combo_er;
        }
        println!("\n[kombinasi regime trend_share_min={combo_th} adx_max={combo_adx} er_max={combo_er}]");
        let (fr, _) = exec(&mkt, &fit, &ledger_path, "chop_combo_fit", &mut session, false)?;
        let (or_, _) = exec(&mkt, &oos, &ledger_path, "chop_combo_oos", &mut session, false)?;
        println!(
            "  blok FIT={:.1}% blok OOS={:.1}% | E OOS={:+.3}R PF OOS={} vs baseline PF {}",
            chop_block_rate(&fr.regime_bars) * 100.0,
            chop_block_rate(&or_.regime_bars) * 100.0,
            or_.metrics.expectancy_r,
            pf_print(or_.metrics.profit_factor),
            pf_print(regime_base.oos_pf)
        );
        summary["combo_regime"] = serde_json::json!({
            "trend_share_min": combo_th, "adx_max": combo_adx, "er_max": combo_er,
            "fit_block": fnum(chop_block_rate(&fr.regime_bars)),
            "oos_block": fnum(chop_block_rate(&or_.regime_bars)),
            "fit_e": fnum(fr.metrics.expectancy_r), "oos_e": fnum(or_.metrics.expectancy_r),
            "oos_pf": fnum(or_.metrics.profit_factor), "baseline_oos_pf": fnum(regime_base.oos_pf),
        });
    }
    let primary_regime = regime_base.fit_block >= screen_base.fit_block;
    println!(
        "\nG-09 memakai keluarga {} karena blok FIT baseline lebih tinggi (regime {:.1}% vs screener {:.1}%).",
        if primary_regime { "regime trend_share_min" } else { "screener chop_range" },
        regime_base.fit_block * 100.0,
        screen_base.fit_block * 100.0
    );
    summary["regime_plateau"] = serde_json::json!({"rows": rows_json(&trend_rows), "choice": choice_json(&regime_choice), "baseline": regime_base.threshold});
    summary["screener_plateau"] = serde_json::json!({"rows": rows_json(&screen_rows), "choice": choice_json(&screen_choice), "baseline": screen_base.threshold});
    summary["g09_family"] = serde_json::json!(if primary_regime { "regime" } else { "screener" });
    std::fs::write(&summary_path, serde_json::to_vec_pretty(&summary)?)?;

    let mut gate_rows = Vec::new();
    for id in ids {
        let c = make_cfg(cfg, &a, vec![id])?;
        let (r, ho) = exec(&mkt, &c, &ledger_path, "base", &mut session, a.unlock_holdout)?;
        let mut cs = c.clone();
        stress(&mut cs);
        let (rs, _) = exec(&mkt, &cs, &ledger_path, "stress", &mut session, false)?;
        let folds = anchored_folds(c.start, c.end, 6);
        let mut fold_e = Vec::new();
        let mut fold_meta = Vec::new();
        for f in &folds {
            let (refit_chop, train_m) = refit_chop_fold(&mkt, &c, f.train_start, f.train_end)?;
            let mut fc = c.clone();
            fc.start = f.test_start;
            fc.end = f.test_end;
            fc.chop = refit_chop;
            let (fr, _) = exec(&mkt, &fc, &ledger_path, &format!("wf{}", f.index), &mut session, false)?;
            fold_e.push(fr.metrics.expectancy_r);
            fold_meta.push(serde_json::json!({
                "fold": f.index,
                "train_from": fmt_utc(f.train_start),
                "train_to": fmt_utc(f.train_end),
                "test_from": fmt_utc(f.test_start),
                "test_to": fmt_utc(f.test_end),
                "refit_chop": {
                    "trend_share_min": refit_chop.trend_share_min,
                    "adx_max": refit_chop.adx_max,
                    "er_max": refit_chop.er_max,
                },
                "train_trades": train_m.trades,
                "train_e": fnum(train_m.expectancy_r),
                "train_pf": fnum(train_m.profit_factor),
                "trades": fr.metrics.trades,
                "e": fnum(fr.metrics.expectancy_r),
                "pf": fnum(fr.metrics.profit_factor),
            }));
        }
        let wf = walk_forward_gate(&fold_e);
        let m = &r.metrics;
        let val_from = parse_ymd(VAL_FROM).ok_or_else(|| anyhow::anyhow!("invalid validation boundary"))?;
        let is_t = seg_trades(&r.trades, i64::MIN, val_from);
        let val_t = seg_trades(&r.trades, val_from, i64::MAX);
        let m_val = Metrics::from_trades(&val_t, cfg.risk.start_balance_usd);
        let stress_val_t = seg_trades(&rs.trades, val_from, i64::MAX);
        let m_stress_val = Metrics::from_trades(&stress_val_t, cfg.risk.start_balance_usd);
        let (n_is, n_val) = (is_t.len(), val_t.len());
        let g02_ok = n_is >= 100 && n_val >= 60;
        let rvec: Vec<f64> = r.trades.iter().map(|t| t.r).collect(); // G-08 (DSR): R per trade
        let rvec_val: Vec<f64> = val_t.iter().map(|t| t.r).collect(); // G-07: bootstrap urutan trade VAL
        let per_year = trades_per_year(rvec_val.len(), c.end.saturating_sub(c.start.max(val_from)).max(1));
        let mc = monte_carlo(&rvec_val, per_year, 10_000, cfg.risk.start_balance_usd, cfg.risk.equity_floor_usd, 42);
        let prior = priors.get(id.code()).copied().unwrap_or(0);
        let session_n = session.runs.get(id.code()).copied().unwrap_or(0);
        let n_trials = honest_n_trials(prior, session_n);
        let srs = session.srs.get(id.code()).map(|v| v.as_slice()).unwrap_or(&[]);
        let var_sr = dsr_variance(srs);
        let d = dsr(&rvec, n_trials, var_sr);
        let (g09_rows, g09_th) = if primary_regime { (&trend_rows, c.chop.trend_share_min) } else { (&screen_rows, c.screener.chop_range) };
        let idx = g09_rows.iter().enumerate().min_by(|(_, a), (_, b)| (a.threshold - g09_th).abs().total_cmp(&(b.threshold - g09_th))).map(|(i, _)| i).unwrap_or(0);
        // Rules G-09: ekspektansi VAL -- pakai oos_e_by (2025-07-01..2026-05-01).
        let selected_e = g09_rows[idx].oos_e_by.get(id.code()).copied().unwrap_or(0.0);
        let mut neigh = Vec::new();
        if idx > 0 {
            neigh.push(g09_rows[idx - 1].oos_e_by.get(id.code()).copied().unwrap_or(0.0));
        }
        if idx + 1 < g09_rows.len() {
            neigh.push(g09_rows[idx + 1].oos_e_by.get(id.code()).copied().unwrap_or(0.0));
        }
        let plateau = plateau_gate(selected_e, &neigh);
        println!("\n== Validate {} | {} → {}", id.code(), fmt_utc(c.start), fmt_utc(c.end));
        print_metrics("base", m);
        print_metrics("VAL (segmen Rules)", &m_val);
        print_metrics("stress full", &rs.metrics);
        print_metrics("stress VAL", &m_stress_val);
        let mut all = gate_v("G-01 data, checksum and prefix replay", r.data_gaps == 0, false, format!("{} data gaps; checksum/prefix not verified", r.data_gaps));
        all &= gate_v("G-02 sampel IS≥100 & VAL≥60 (Rules G-02; kurang → INCONCLUSIVE)", g02_ok, g02_ok, format!("IS={} (min 100), VAL={} (min 60), total={}", n_is, n_val, m.trades));
        all &= gate_v("G-03 expectancy VAL ≥ +0.10R", g03_pass(m_val.expectancy_r), g02_ok, format!("{:+.3}R (VAL {} trade)", m_val.expectancy_r, n_val));
        all &= gate_v("G-04 PF VAL ≥ 1.25", g04_pass(m_val.profit_factor), g02_ok, format!("{} (VAL)", pf_print(m_val.profit_factor)));
        all &= gate_v("G-05 stress VAL: fee 2x, slip +0.05%, funding 2x", g05_pass(m_stress_val.expectancy_r, m_stress_val.profit_factor), g02_ok && m_stress_val.trades >= 60, format!("VAL {} trades, {:+.3}R / {} (full {:+.3}R / {})", m_stress_val.trades, m_stress_val.expectancy_r, pf_print(m_stress_val.profit_factor), rs.metrics.expectancy_r, pf_print(rs.metrics.profit_factor)));
        all &= gate_v("G-06 WF anchored 6 fold (refit CHOP per fold di train) ≥4/6 >0, terburuk ≥ −0.15R", wf.pass, g02_ok, format!("{}/6, terburuk {:+.3}R, folds={}", wf.positive, wf.worst, fold_e.iter().map(|e| format!("{e:+.3}")).collect::<Vec<_>>().join(",")));
        all &= gate_v("G-07 MC 10000 path bootstrap trade VAL, ukuran 1 tahun, p95 DD ≤ 15R & P(<$40) ≤ 5%", mc.paths == 10_000 && g07_pass(mc.p95_max_dd_r, mc.p_below_floor), g02_ok, format!("paths={} n_year={} (VAL {} trade) p95DD={:.1}R P(<40)={:.1}%", mc.paths, per_year, n_val, mc.p95_max_dd_r, mc.p_below_floor * 100.0));
        all &= gate_v("G-08 DSR ≥ 0.95, N = ledger strategi + run sesi ini + 40 legacy (trial <10 trade dibuang)", g08_pass(d), g02_ok, format!("{d:.3} (N={n_trials}, prior={prior}, sesi={session_n}, varSR={var_sr:.6})"));
        all &= gate_v("G-09 plateau ≥80% tetangga E_VAL>0 dan median ≥ 50% E_VAL terpilih", plateau.pass, g02_ok, format!("tetangga {}/{} positif ({:.0}%), median {:+.3} vs 50%×E_VAL {:+.3}", (plateau.positive_frac * plateau.n_neighbors as f64).round() as usize, plateau.n_neighbors, plateau.positive_frac * 100.0, plateau.median, 0.5 * plateau.selected));
        all &= gate_v("G-10 konsentrasi simbol ≤40% & bulan ≤35%", g10_pass(m.net_usd, m.max_symbol_profit_share, m.max_month_profit_share), g02_ok, format!("simbol {:.0}% / bulan {:.0}% dari net", m.max_symbol_profit_share * 100.0, m.max_month_profit_share * 100.0));
        let g11_ok = if ho {
            let ok = m.trades >= 40 && g11_pass(m.expectancy_r, m.profit_factor);
            gate_v("G-11 holdout trades≥40 & E>0 & PF≥1.10 (Rules G-02: HOLDOUT≥40)", ok, m.trades >= 40, format!("{} trade (min 40), {:+.3}R / {}", m.trades, m.expectancy_r, pf_print(m.profit_factor)));
            ok
        } else {
            println!("  [LOCKED] G-11 holdout 2026-05-01..2026-10-31 tidak dijalankan. Syarat bila dibuka sekali: E>0 dan PF≥1.10. Bukan PASS.");
            false
        };
        let suffix = if !g02_ok {
            " (G-02 INCONCLUSIVE: sampel IS/VAL kurang — hasil gate lain tidak sah sebagai validasi aturan)"
        } else if all && !g11_ok {
            " (gate selain G-11 lolos; holdout terkunci, bukan GO)"
        } else if all && g11_ok {
            " (angka G-11 lolos; status tetap NO-GO sampai review user)"
        } else {
            ""
        };
        println!("  ==> {} : NO-GO{}", id.code(), suffix);
        let mut gate_row = serde_json::json!({
            "strategy": id.code(),
            "trades": m.trades,
            "is_trades": n_is,
            "val_trades": n_val,
            "e": fnum(m.expectancy_r),
            "val_e": fnum(m_val.expectancy_r),
            "pf": fnum(m.profit_factor),
            "val_pf": fnum(m_val.profit_factor),
            "net_usd": fnum(m.net_usd),
            "max_dd_r": fnum(m.max_dd_r),
            "max_dd_usd": fnum(m.max_dd_usd),
            "win_rate": fnum(m.win_rate),
            "stress_e": fnum(m_stress_val.expectancy_r),
            "stress_pf": fnum(m_stress_val.profit_factor),
            "stress_trades": m_stress_val.trades,
            "wf_positive": wf.positive,
            "wf_worst": fnum(wf.worst),
            "wf_pass": wf.pass && g02_ok,
            "folds": fold_meta,
            "mc_paths": mc.paths,
            "mc_per_year": per_year,
            "mc_p95_dd_r": fnum(mc.p95_max_dd_r),
            "mc_p_below_floor": fnum(mc.p_below_floor),
            "dsr": fnum(d),
            "n_trials": n_trials,
            "prior_trials": prior,
            "session_trials": session_n,
            "var_sr": fnum(var_sr),
            "g09_pass": plateau.pass && g02_ok,
            "g09_positive_frac": fnum(plateau.positive_frac),
            "g09_median": fnum(plateau.median),
            "g09_selected_e": fnum(plateau.selected),
            "g11": if ho { serde_json::json!(g11_ok) } else { serde_json::Value::String("LOCKED".into()) },
            "tested_pass": all,
            "chop_block": fnum(chop_block_rate(&r.regime_bars)),
            "non_trade": fnum(non_trade_rate(&r.regime_bars)),
            "screen_block": fnum((1.0 - r.screener_trend_share).clamp(0.0, 1.0)),
            "data_gaps": r.data_gaps,
        });
        gate_row["wf_raw_pass"] = serde_json::json!(wf.pass);
        gate_row["max_symbol_profit_share"] = serde_json::json!(fnum(m.max_symbol_profit_share));
        gate_row["max_month_profit_share"] = serde_json::json!(fnum(m.max_month_profit_share));
        gate_row["g10_raw_pass"] = serde_json::json!(g10_pass(m.net_usd, m.max_symbol_profit_share, m.max_month_profit_share));
        gate_row["stress_full_e"] = serde_json::json!(fnum(rs.metrics.expectancy_r));
        gate_row["stress_full_pf"] = serde_json::json!(fnum(rs.metrics.profit_factor));
        gate_row["g01_verified"] = serde_json::json!(false);
        gate_row["g06_refit_available"] = serde_json::json!(true);
        gate_row["g09_sweep_diagnostic_pass"] = serde_json::json!(plateau.pass);
        gate_row["g09_raw_pass"] = serde_json::json!(plateau.pass);
        gate_rows.push(gate_row);
    }
    summary["gates"] = serde_json::Value::Array(gate_rows);
    let path = a.out.join("validation_summary.json");
    std::fs::write(&path, serde_json::to_vec_pretty(&summary)?)?;
    println!("\nringkasan: {}", path.display());
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn validation_boundary_matches_rules() {
        assert_eq!(parse_ymd(VAL_FROM), parse_ymd("2025-07-01"));
        assert_eq!(FIT_TO, VAL_FROM);
        assert_eq!(OOS_FROM, VAL_FROM);
    }

    #[test]
    fn ledger_write_failure_is_not_silenced() {
        let path = std::env::temp_dir(); // directory cannot be opened as a ledger file
        assert!(append_ledger(&path, "test", &BacktestConfig::default(), false, &Metrics::default(), "test").is_err());
    }

    #[test]
    fn stress_matches_g05() {
        let mut c = BacktestConfig::default();
        let base_market = c.market_slip;
        stress(&mut c);
        assert_eq!(c.fee_mult, 2.0);
        assert_eq!(c.funding_mult, 2.0);
        assert!((c.extra_slip - 0.0005).abs() < 1e-15);
        assert_eq!(c.market_slip, base_market);
    }

    #[test]
    fn seg_trades_splits_by_exit_ms() {
        use apex_core::risk::ExitKind;
        use apex_core::types::Side;
        let mk = |exit_ms: i64| Trade {
            symbol: "XUSDT".into(),
            strategy: "S1_TFP".into(),
            side: Side::Long,
            signal_ms: exit_ms - 2,
            entry_ms: exit_ms - 1,
            exit_ms,
            entry: 100.0,
            sl: 99.0,
            tp: 102.0,
            exit: 101.0,
            qty: 1.0,
            notional: 100.0,
            margin: 4.0,
            risk_planned: 1.0,
            fees: 0.1,
            funding: 0.0,
            pnl_net: 1.0,
            r: 1.0,
            exit_kind: ExitKind::TakeProfit,
            regime: "TREND".into(),
        };
        let tr = vec![mk(100), mk(200), mk(300)];
        assert_eq!(seg_trades(&tr, i64::MIN, 250).len(), 2);
        assert_eq!(seg_trades(&tr, 250, i64::MAX).len(), 1);
        let mut forced = mk(300);
        forced.exit_kind = ExitKind::EndOfData;
        assert!(seg_trades(&[forced], 250, i64::MAX).is_empty());
        let mut cross_boundary = mk(300);
        cross_boundary.signal_ms = 240;
        cross_boundary.entry_ms = 260;
        assert!(seg_trades(&[cross_boundary], 250, i64::MAX).is_empty());
        let mut invalid_order = mk(300);
        invalid_order.signal_ms = 301;
        assert!(seg_trades(&[invalid_order], 250, i64::MAX).is_empty());
        let mut val_loss = mk(300);
        val_loss.pnl_net = -1.0;
        val_loss.r = -1.0;
        val_loss.exit_kind = ExitKind::StopLoss;
        let mixed = vec![mk(100), mk(200), val_loss];
        let full = Metrics::from_trades(&mixed, 60.0);
        let val = Metrics::from_trades(&seg_trades(&mixed, 250, i64::MAX), 60.0);
        assert!(full.expectancy_r > 0.0);
        assert!(val.expectancy_r < 0.0);
        assert!(!g05_pass(val.expectancy_r, val.profit_factor));
    }

    #[test]
    fn gate_v_inconclusive_when_sample_short() {
        // ok=true tapi sampel kurang -> TIDAK boleh PASS
        assert!(!gate_v("x", true, false, "d".into()));
        assert!(gate_v("x", true, true, "d".into()));
        assert!(!gate_v("x", false, true, "d".into()));
        let fixed = walk_forward_gate(&[0.2; 6]);
        assert!(fixed.pass);
        assert!(!gate_v("G-06 without sample", fixed.pass, false, "sample short".into()));
    }

    #[test]
    fn trial_sr_drops_small_samples() {
        // 2 trade dengan R nyaris sama -> SR ratusan (membuat var_sr meledak); harus dibuang.
        assert!(trial_sr(&[-0.996, -1.0]).is_none());
        assert!(trial_sr(&[0.5; 9]).is_none());
        let ok: Vec<f64> = (0..10).map(|i| if i % 3 == 0 { -1.0 } else { 1.0 }).collect();
        assert_eq!(trial_sr(&ok), Some(sharpe_ratio(&ok)));
    }
}
