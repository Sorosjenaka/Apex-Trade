//! `apex backtest` dan `apex validate` (Rules §4.3 G-01..G-11, parsial).
use crate::cfg::AllConfig;
use apex_backtest::*;
use apex_core::time::{fmt_utc, parse_ymd};
use apex_core::types::Tf;
use apex_data::csvio::{funding_path, kline_path, load_funding, load_klines};
use apex_strategies::StrategyId;
use std::path::{Path, PathBuf};

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
}

pub fn load_market(cfg: &AllConfig, data: &Path, filters_path: &Path) -> anyhow::Result<MarketInput> {
    let filters = crate::cfg::load_filters(filters_path)?;
    let allow = crate::cfg::load_allowlist(&cfg.dir)?;
    let btc_m15 = load_klines(&kline_path(data, "BTCUSDT", Tf::M15))?;
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
        symbols.push(SymbolInput { symbol: s.clone(), m15, h1, funding, filters: f, tradeable });
    }
    Ok(MarketInput { symbols, btc_m15, btc_h1, dom_h1: dom })
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
        regime_mode: if a.regime == "screener" { RegimeMode::ScreenerOnly } else { RegimeMode::Reader },
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
        append_ledger(&ledger, &hash, &c, ho, &r.metrics);
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

pub fn append_ledger(ledger: &Path, hash: &str, c: &BacktestConfig, holdout: bool, m: &Metrics) {
    use std::io::Write;
    let line = serde_json::json!({
        "ts": apex_core::time::now_ms(),
        "param_hash": hash,
        "strategies": c.strategies.iter().map(|s| s.code()).collect::<Vec<_>>(),
        "start": fmt_utc(c.start), "end": fmt_utc(c.end),
        "holdout": holdout, "stress": c.fee_mult > 1.0,
        "trades": m.trades, "expectancy_r": m.expectancy_r, "pf": if m.profit_factor.is_finite() { m.profit_factor } else { 99.0 },
    });
    if let Ok(mut f) = std::fs::OpenOptions::new().create(true).append(true).open(ledger) {
        let _ = writeln!(f, "{line}");
    }
}

fn gate(name: &str, ok: bool, detail: String) -> bool {
    println!("  [{}] {name}: {detail}", if ok { "PASS" } else { "FAIL" });
    ok
}

/// Validasi gate pada satu jendela (IS/VAL dari Rules §4.3 butuh data 2023+; ini versi jendela tunggal).
pub fn validate(cfg: &AllConfig, a: &BtArgs) -> anyhow::Result<()> {
    let fpath = a.filters.clone().unwrap_or_else(|| cfg.dir.join("filters_snapshot.json"));
    let mkt = load_market(cfg, &a.data, &fpath)?;
    std::fs::create_dir_all(&a.out)?;
    let ledger = a.out.join("trials_ledger.jsonl");
    let n_ledger = std::fs::read_to_string(&ledger).map(|s| s.lines().count()).unwrap_or(0);
    for id in parse_ids(&a.strategies, cfg) {
        let c = make_cfg(cfg, a, vec![id])?;
        let hash = param_hash(&serde_json::to_value(&c)?);
        let ho = holdout_guard(c.start, c.end, a.unlock_holdout, &hash, &ledger)?;
        let r = run_backtest(&mkt, &c)?;
        append_ledger(&ledger, &hash, &c, ho, &r.metrics);
        let mut cs = c.clone();
        stress(&mut cs);
        let rs = run_backtest(&mkt, &cs)?;
        let m = &r.metrics;
        println!("\n== Validate {} | {} → {}", id.code(), fmt_utc(c.start), fmt_utc(c.end));
        print_metrics("base", m);
        print_metrics("stress", &rs.metrics);
        let mut all = true;
        all &= gate("G-02 jumlah trade (≥60 untuk jendela VAL)", m.trades >= 60, format!("{}", m.trades));
        all &= gate("G-03 expectancy ≥ +0.10R", m.expectancy_r >= 0.10, format!("{:+.3}R", m.expectancy_r));
        all &= gate("G-04 PF ≥ 1.25", m.profit_factor >= 1.25, format!("{:.2}", m.profit_factor));
        all &= gate("G-05 stress E ≥ 0.03R & PF ≥ 1.10", rs.metrics.expectancy_r >= 0.03 && rs.metrics.profit_factor >= 1.10, format!("{:+.3}R / {:.2}", rs.metrics.expectancy_r, rs.metrics.profit_factor));
        // G-06 walk-forward anchored 6 fold (param tetap → ekspektasi per segmen waktu)
        let span = (c.end - c.start) / 7;
        let mut folds = Vec::new();
        for k in 1..=6 {
            let (fs, fe) = (c.start + span * k, c.start + span * (k + 1));
            let rr: Vec<f64> = r.trades.iter().filter(|t| t.exit_ms >= fs && t.exit_ms < fe).map(|t| t.r).collect();
            folds.push(if rr.is_empty() { 0.0 } else { rr.iter().sum::<f64>() / rr.len() as f64 });
        }
        let pos = folds.iter().filter(|x| **x > 0.0).count();
        let worst = folds.iter().cloned().fold(f64::INFINITY, f64::min);
        all &= gate("G-06 WF ≥4/6 fold positif, terburuk ≥ −0.15R", pos >= 4 && worst >= -0.15, format!("{pos}/6, terburuk {worst:+.3}R, folds={folds:.3?}"));
        let rvec: Vec<f64> = r.trades.iter().map(|t| t.r).collect();
        let mc = monte_carlo(&rvec, rvec.len().max(1), 10_000, cfg.risk.start_balance_usd, cfg.risk.equity_floor_usd, 42);
        all &= gate("G-07 MC p95 DD ≤ 15R & P(<$40) ≤ 5%", mc.p95_max_dd_r <= 15.0 && mc.p_below_floor <= 0.05, format!("p95DD={:.1}R P(<40)={:.1}%", mc.p95_max_dd_r, mc.p_below_floor * 100.0));
        let n_trials = n_ledger + 40;
        let d = dsr(&rvec, n_trials, 0.01);
        all &= gate("G-08 DSR ≥ 0.95", d >= 0.95, format!("{d:.3} (N={n_trials})"));
        all &= gate("G-10 konsentrasi simbol ≤40% & bulan ≤35%", m.net_usd > 0.0 && m.max_symbol_profit_share <= 0.40 && m.max_month_profit_share <= 0.35, format!("profit terbesar per simbol {:.0}% / per bulan {:.0}% dari net profit", m.max_symbol_profit_share * 100.0, m.max_month_profit_share * 100.0));
        println!("  G-01/G-09/G-11: tidak dievaluasi otomatis di sini (G-09 plateau butuh sweep parameter; G-11 holdout dikunci).");
        println!("  ==> {} : {}", id.code(), if all { "LOLOS gate yang diuji (tetap butuh G-09/G-11 + review user sebelum GO)" } else { "NO-GO" });
    }
    Ok(())
}
