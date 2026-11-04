//! Loop event backtest. Lihat doc crate untuk urutan & aturan fill.
use crate::context::{AltInput, RegimeMode, RegimeTimeline};
use crate::stats::Metrics;
use apex_core::config::RiskConfig;
use apex_core::filters::SymbolFilters;
use apex_core::num::{to_dec, to_f64};
use apex_core::risk::{ExitKind, GateContext, RiskState};
use apex_core::sizing::{EntryKind, SizingInput};
use apex_core::types::{Bar, Ms, Side, Tf};
use apex_market_reader::UniverseScope;
use apex_screener::regime::{classify, ScreenerConfig, ScreenerMachine, SymState, SymbolFeatures};
use apex_strategies::{evaluate, Signal, StrategyId, StrategyParams, SymbolSeries};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

pub struct SymbolInput {
    pub symbol: String,
    pub m15: Vec<Bar>,
    pub h1: Vec<Bar>,
    pub funding: Vec<(Ms, f64)>,
    pub filters: SymbolFilters,
    /// Simbol boleh ditradingkan (allowlist). BTCUSDT = konteks saja.
    pub tradeable: bool,
}

pub struct MarketInput {
    pub symbols: Vec<SymbolInput>,
    pub btc_m15: Vec<Bar>,
    pub btc_h1: Vec<Bar>,
    pub dom_h1: Option<Vec<Bar>>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct BacktestConfig {
    pub start: Ms,
    pub end: Ms,
    pub risk: RiskConfig,
    pub params: StrategyParams,
    pub strategies: Vec<StrategyId>,
    pub screener: ScreenerConfig,
    pub regime_mode: RegimeMode,
    /// Terapkan TV-06 (sumber cadangan → max_pos 1) di backtest.
    pub cap_on_proxy: bool,
    /// Maintenance margin rate untuk likuidasi cross (konservatif).
    pub mmr: f64,
    pub fee_mult: f64,
    pub funding_mult: f64,
    /// Slip tambahan per fill (stress G-05 = 0,0005).
    pub extra_slip: f64,
    /// Slip entry MARKET.
    pub market_slip: f64,
    /// Volume gate historis parsial (V-01, V-02 dari klines 24 jam).
    pub volume_gate: bool,
    pub min_quote_volume_24h: f64,
    pub min_trades_24h: u64,
}

impl Default for BacktestConfig {
    fn default() -> Self {
        BacktestConfig {
            start: 0,
            end: i64::MAX,
            risk: RiskConfig::default(),
            params: StrategyParams::default(),
            strategies: StrategyId::ALL.to_vec(),
            screener: ScreenerConfig::default(),
            regime_mode: RegimeMode::Reader,
            cap_on_proxy: false,
            mmr: 0.01,
            fee_mult: 1.0,
            funding_mult: 1.0,
            extra_slip: 0.0,
            market_slip: 0.0002,
            volume_gate: true,
            min_quote_volume_24h: 20_000_000.0,
            min_trades_24h: 100_000,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Trade {
    pub symbol: String,
    pub strategy: String,
    pub side: Side,
    pub signal_ms: Ms,
    pub entry_ms: Ms,
    pub exit_ms: Ms,
    pub entry: f64,
    pub sl: f64,
    pub tp: f64,
    pub exit: f64,
    pub qty: f64,
    pub notional: f64,
    pub margin: f64,
    pub risk_planned: f64,
    pub fees: f64,
    pub funding: f64,
    pub pnl_net: f64,
    pub r: f64,
    pub exit_kind: ExitKind,
    pub regime: String,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct BacktestReport {
    pub start: String,
    pub end: String,
    pub strategies: Vec<String>,
    pub regime_mode: RegimeMode,
    pub trades: Vec<Trade>,
    pub metrics: Metrics,
    pub per_strategy: BTreeMap<String, Metrics>,
    pub signals: BTreeMap<String, u64>,
    pub skips: BTreeMap<String, u64>,
    pub regime_bars: BTreeMap<String, u64>,
    pub screener_trend_share: f64,
    pub data_gaps: u64,
    pub final_wallet: f64,
    pub liquidated: bool,
}

#[derive(Clone, Debug)]
struct Pending {
    sig: Signal,
    sym: usize,
    entry: f64,
    sl: f64,
    tp: f64,
    qty: f64,
    risk: f64,
    margin: f64,
    created_k: usize,
    tick: f64,
    regime: String,
    ofcc_watch: Option<apex_strategies::ofcc_watch::S1Watch>,
}

#[derive(Clone, Debug)]
struct Position {
    p: Pending,
    entry_ms: Ms,
    fill: f64,
    fee_in: f64,
    funding: f64,
    fill_k: usize,
}

struct SymCtx {
    series: SymbolSeries,
    states: Vec<SymState>, // per indeks 1h
    ot15: Vec<Ms>,
    qv24: Vec<f64>,
    tr24: Vec<f64>,
    vg_in: bool,
    vg_fail: u32,
    vg_pass: u32,
}

fn rolling_sum(v: &[f64], n: usize) -> Vec<f64> {
    let mut out = vec![f64::NAN; v.len()];
    let mut s = 0.0;
    for i in 0..v.len() {
        s += v[i];
        if i >= n {
            s -= v[i - n];
        }
        if i + 1 >= n {
            out[i] = s;
        }
    }
    out
}

pub fn screener_states(h1: &[Bar], m15: &[Bar], cfg: &ScreenerConfig) -> Vec<SymState> {
    let f = SymbolFeatures::compute(h1, cfg);
    let c15 = apex_indicators::closes(m15);
    let (e21, e55) = (apex_indicators::ema(&c15, 21), apex_indicators::ema(&c15, 55));
    let ot15: Vec<Ms> = m15.iter().map(|b| b.open_time).collect();
    let mut m = ScreenerMachine::default();
    let mut out = Vec::with_capacity(h1.len());
    for i in 0..h1.len() {
        let close = h1[i].open_time + Tf::H1.ms();
        let j = ot15.partition_point(|&t| t + Tf::M15.ms() <= close).checked_sub(1);
        let mtf15 = j
            .map(|j| if j < 55 { 0 } else if e21[j] > e55[j] && c15[j] > e21[j] { 1 } else if e21[j] < e55[j] && c15[j] < e21[j] { -1 } else { 0 })
            .unwrap_or(0);
        let r = classify(&f, i, mtf15, cfg);
        out.push(m.step(r.raw, cfg));
    }
    out
}

pub fn run_backtest(mkt: &MarketInput, cfg: &BacktestConfig) -> anyhow::Result<BacktestReport> {
    let rc = &cfg.risk;
    rc.validate_golden_rules().map_err(|e| anyhow::anyhow!("{e}"))?;
    // master clock = BTC 15m
    let ks: Vec<usize> = (0..mkt.btc_m15.len()).filter(|&i| mkt.btc_m15[i].open_time >= cfg.start && mkt.btc_m15[i].open_time + Tf::M15.ms() <= cfg.end).collect();
    let close_times: Vec<Ms> = ks.iter().map(|&i| mkt.btc_m15[i].open_time + Tf::M15.ms()).collect();
    let alts: Vec<AltInput> = mkt.symbols.iter().filter(|s| s.symbol != "BTCUSDT").map(|s| AltInput { h1: &s.h1 }).collect();
    let timeline = match cfg.regime_mode {
        RegimeMode::Reader => RegimeTimeline::build(&mkt.btc_m15, &mkt.btc_h1, mkt.dom_h1.as_deref(), &alts, &close_times, cfg.cap_on_proxy),
        RegimeMode::ScreenerOnly => RegimeTimeline::permissive(&close_times),
    };
    let mut syms: Vec<SymCtx> = mkt
        .symbols
        .iter()
        .map(|s| {
            let qv: Vec<f64> = s.m15.iter().map(|b| b.quote_volume).collect();
            let tr: Vec<f64> = s.m15.iter().map(|b| b.trades as f64).collect();
            SymCtx {
                series: SymbolSeries::new(&s.symbol, &s.m15, &s.h1),
                states: screener_states(&s.h1, &s.m15, &cfg.screener),
                ot15: s.m15.iter().map(|b| b.open_time).collect(),
                qv24: rolling_sum(&qv, 96),
                tr24: rolling_sum(&tr, 96),
                vg_in: false,
                vg_fail: 0,
                vg_pass: 0,
            }
        })
        .collect();

    let mut wallet = rc.start_balance_usd;
    let mut risk = RiskState::new(cfg.start.max(0));
    let mut pending: Vec<Pending> = Vec::new();
    let mut market_q: Vec<Pending> = Vec::new();
    let mut open: Vec<Position> = Vec::new();
    let mut trades: Vec<Trade> = Vec::new();
    let mut skips: BTreeMap<String, u64> = BTreeMap::new();
    let mut signals: BTreeMap<String, u64> = BTreeMap::new();
    let mut regime_bars: BTreeMap<String, u64> = BTreeMap::new();
    let (mut trend_bars, mut sym_bars) = (0u64, 0u64);
    let mut gaps = 0u64;
    let mut liquidated = false;
    let slip_sl = rc.slip_sl + cfg.extra_slip;
    let fee_t = rc.fee_taker * cfg.fee_mult;
    let fee_m = rc.fee_maker * cfg.fee_mult;
    let bump = |m: &mut BTreeMap<String, u64>, k: &str| *m.entry(k.to_string()).or_insert(0) += 1;

    for (kk, &bi) in ks.iter().enumerate() {
        let t_open = mkt.btc_m15[bi].open_time;
        let t_close = t_open + Tf::M15.ms();
        // bar tiap simbol pada T
        let idx: Vec<Option<usize>> = syms.iter().map(|s| s.ot15.binary_search(&t_open).ok()).collect();

        // --- 0. MARKET entry antrian dari close sebelumnya → isi di open bar ini
        for p in std::mem::take(&mut market_q) {
            let Some(i) = idx[p.sym] else {
                bump(&mut skips, "MARKET_NO_BAR");
                continue;
            };
            let b = &mkt.symbols[p.sym].m15[i];
            let s = p.sig.side.sign();
            let fill = b.open * (1.0 + s * (cfg.market_slip + cfg.extra_slip));
            // SL/TP relatif harga terencana; bila open sudah melewati SL → langsung keluar SL di open (konservatif)
            open.push(Position { entry_ms: t_open, fill, fee_in: fill * p.qty * fee_t, funding: 0.0, fill_k: kk, p });
        }

        // --- 1. funding, 2. exit intrabar
        let mut still = Vec::new();
        for mut pos in std::mem::take(&mut open) {
            let Some(i) = idx[pos.p.sym] else {
                gaps += 1;
                still.push(pos);
                continue;
            };
            let si = &mkt.symbols[pos.p.sym];
            let b = &si.m15[i];
            let s = pos.p.sig.side.sign();
            for &(ft, rate) in si.funding.iter().filter(|(ft, _)| *ft >= t_open && *ft < t_close && *ft > pos.entry_ms) {
                let _ = ft;
                pos.funding += s * rate * cfg.funding_mult * pos.p.qty * b.open;
            }
            let (sl, tp) = (pos.p.sl, pos.p.tp);
            let sl_hit = if s > 0.0 { b.low <= sl } else { b.high >= sl };
            let tp_hit = if s > 0.0 { b.high >= tp } else { b.low <= tp };
            let on_fill_bar = pos.fill_k == kk;
            let exit = if sl_hit {
                let gap = if s > 0.0 { b.open <= sl } else { b.open >= sl };
                let px = if gap { b.open } else { sl };
                Some((px * (1.0 - s * slip_sl), ExitKind::StopLoss))
            } else if tp_hit && !on_fill_bar {
                let gap = if s > 0.0 { b.open >= tp } else { b.open <= tp };
                let px = if gap { b.open } else { tp };
                Some((px * (1.0 - s * slip_sl), ExitKind::TakeProfit))
            } else {
                None
            };
            match exit {
                Some((px, kind)) => {
                    let tr = close_trade(&pos, px, t_open + if kind == ExitKind::StopLoss { 1 } else { 2 }, kind, fee_t, si);
                    wallet += tr.pnl_net;
                    risk.record_exit(kind, tr.pnl_net, t_close, rc);
                    trades.push(tr);
                }
                None => still.push(pos),
            }
        }
        open = still;

        // --- 3. order pending: fill / expire
        let mut keep = Vec::new();
        // The setup has reserved a slot, but submission after OFCC is a new decision.
        let mut remaining = std::mem::take(&mut pending).into_iter();
        while let Some(mut p) = remaining.next() {
            if kk >= p.created_k + p.sig.ttl_bars as usize {
                bump(&mut skips, "EXPIRED");
                continue;
            }
            let Some(i) = idx[p.sym] else {
                keep.push(p);
                continue;
            };
            let si = &mkt.symbols[p.sym];
            let b = &si.m15[i];
            let s = p.sig.side.sign();
            if let Some(watch) = p.ofcc_watch.as_mut() {
                use apex_strategies::ofcc_watch::Outcome;
                let series = &syms[p.sym].series;
                let feature = if p.sig.side == Side::Long { &series.long.f15 } else { &series.short.f15 };
                match watch.observe(b, t_close, feature.p90_absdelta[i]) {
                    Outcome::Waiting => keep.push(p),
                    Outcome::Confirmed => {
                        // A post-only order cannot be submitted if the close already crosses its limit.
                        let crosses = if s > 0.0 { b.close <= p.entry } else { b.close >= p.entry };
                        let d = &timeline.decisions[kk];
                        let upnl: f64 = open.iter().map(|pos| {
                            idx[pos.p.sym].map(|j| pos.p.sig.side.sign() * (mkt.symbols[pos.p.sym].m15[j].close - pos.fill) * pos.p.qty).unwrap_or(0.0)
                        }).sum();
                        let state = syms[p.sym].series.h1_idx_at(i).map(|h| syms[p.sym].states[h]).unwrap_or(SymState::Blocked);
                        let allowed_universe = match d.universe {
                            UniverseScope::All => true,
                            UniverseScope::None => false,
                            UniverseScope::Majors => {
                                let mut v: Vec<(usize, f64)> = (0..syms.len()).filter_map(|j| idx[j].map(|i| (j, syms[j].qv24[i]))).filter(|x| x.1.is_finite()).collect();
                                v.sort_by(|a, b| b.1.total_cmp(&a.1).then(a.0.cmp(&b.0)));
                                v.into_iter().take(10).any(|(j, _)| j == p.sym)
                            }
                        };
                        // Also require the latest volume bar to pass, so a pending confirmation cannot use stale gate state.
                        let current_volume_ok = !cfg.volume_gate || (syms[p.sym].qv24[i] >= cfg.min_quote_volume_24h && syms[p.sym].tr24[i] >= cfg.min_trades_24h as f64);
                        let ctx = GateContext {
                            now: t_close, armed: true,
                            regime_ok: d.state.is_trade_state() && allowed_universe && syms[p.sym].vg_in && current_volume_ok,
                            regime_side_ok: d.side_ok(p.sig.side) && state.dir() == p.sig.side.sign() as i8,
                            regime_max_pos: d.max_positions as usize,
                            open_positions: open.len(), pending_entries: keep.len() + remaining.len(),
                            symbol_busy: false, equity: wallet + upnl, spread_bps: None,
                        };
                        if crosses { bump(&mut skips, "OFCC_POST_ONLY_CROSS"); }
                        else if let Err(reason) = risk.check_gates(&ctx, (p.entry - p.sl).abs() / p.entry, rc) { bump(&mut skips, reason.code()); }
                        else { p.ofcc_watch = None; keep.push(p); }
                    }
                    Outcome::Rejected => bump(&mut skips, "OFCC_REJECTED"),
                    Outcome::Expired => bump(&mut skips, "EXPIRED"),
                }
                continue;
            }
            let penetrated = if s > 0.0 { b.low <= p.entry - p.tick } else { b.high >= p.entry + p.tick };
            if penetrated {
                let fill = p.entry;
                let mut pos = Position { entry_ms: t_open, fill, fee_in: fill * p.qty * fee_m, funding: 0.0, fill_k: kk, p };
                // bar fill: SL boleh kena (konservatif), TP tidak
                let sl_hit = if s > 0.0 { b.low <= pos.p.sl } else { b.high >= pos.p.sl };
                if sl_hit {
                    let px = pos.p.sl * (1.0 - s * slip_sl);
                    pos.funding = 0.0;
                    let tr = close_trade(&pos, px, t_open + 1, ExitKind::StopLoss, fee_t, si);
                    wallet += tr.pnl_net;
                    risk.record_exit(ExitKind::StopLoss, tr.pnl_net, t_close, rc);
                    trades.push(tr);
                } else {
                    open.push(pos);
                }
            } else if p.sig.cancel_beyond.is_some_and(|lv| if s > 0.0 { b.close < lv } else { b.close > lv }) {
                bump(&mut skips, "CANCEL_BEYOND");
            } else {
                keep.push(p);
            }
        }
        pending = keep;

        // --- 4. likuidasi cross seluruh wallet (ekstrem adverse serentak)
        if !open.is_empty() {
            let mut worst_upnl = 0.0;
            let mut maint = 0.0;
            for pos in &open {
                if let Some(i) = idx[pos.p.sym] {
                    let b = &mkt.symbols[pos.p.sym].m15[i];
                    let s = pos.p.sig.side.sign();
                    let adverse = if s > 0.0 { b.low } else { b.high };
                    worst_upnl += s * (adverse - pos.fill) * pos.p.qty;
                    maint += adverse * pos.p.qty * cfg.mmr;
                }
            }
            if wallet + worst_upnl <= maint {
                liquidated = true;
                for pos in std::mem::take(&mut open) {
                    let si = &mkt.symbols[pos.p.sym];
                    let px = idx[pos.p.sym].map(|i| if pos.p.sig.side.sign() > 0.0 { si.m15[i].low } else { si.m15[i].high }).unwrap_or(pos.fill);
                    let tr = close_trade(&pos, px, t_close, ExitKind::Liquidation, fee_t, si);
                    wallet += tr.pnl_net;
                    risk.record_exit(ExitKind::Liquidation, tr.pnl_net, t_close, rc);
                    trades.push(tr);
                }
                wallet = wallet.max(0.0);
            }
        }

        // --- 5. keputusan di close bar
        let dec = &timeline.decisions[kk];
        bump(&mut regime_bars, dec.state.as_str());
        let mut upnl = 0.0;
        for pos in &open {
            if let Some(i) = idx[pos.p.sym] {
                let s = pos.p.sig.side.sign();
                upnl += s * (mkt.symbols[pos.p.sym].m15[i].close - pos.fill) * pos.p.qty;
            }
        }
        let equity = wallet + upnl;
        // majors (BTC_LED): 10 teratas quote volume 24 jam
        let majors: Vec<usize> = if dec.universe == UniverseScope::Majors {
            let mut v: Vec<(usize, f64)> = (0..syms.len()).filter_map(|j| idx[j].map(|i| (j, syms[j].qv24[i]))).filter(|x| x.1.is_finite()).collect();
            v.sort_by(|a, b| b.1.total_cmp(&a.1).then(a.0.cmp(&b.0)));
            v.into_iter().take(10).map(|x| x.0).collect()
        } else {
            vec![]
        };
        let new_hour = t_close % Tf::H1.ms() == 0;
        let mut cands: Vec<(usize, usize, Signal)> = Vec::new();
        for (j, sc) in syms.iter_mut().enumerate() {
            let Some(i) = idx[j] else { continue };
            if !mkt.symbols[j].tradeable {
                continue;
            }
            // volume gate parsial (V-01, V-02) + histeresis VG-02 dievaluasi tiap jam
            if new_hour || kk == 0 {
                let pass = !cfg.volume_gate || (sc.qv24[i] >= cfg.min_quote_volume_24h && sc.tr24[i] >= cfg.min_trades_24h as f64);
                if pass {
                    sc.vg_pass += 1;
                    sc.vg_fail = 0;
                    if !sc.vg_in && (sc.vg_pass >= 2 || kk == 0) {
                        sc.vg_in = true;
                    }
                } else {
                    sc.vg_fail += 1;
                    sc.vg_pass = 0;
                    if sc.vg_in && sc.vg_fail >= 3 {
                        sc.vg_in = false;
                    }
                }
            }
            let state = sc.series.h1_idx_at(i).map(|h| sc.states[h]).unwrap_or(SymState::Blocked);
            sym_bars += 1;
            if state.dir() != 0 {
                trend_bars += 1;
            }
            if state.dir() == 0 {
                continue;
            }
            for sig in evaluate(&sc.series, i, state, &cfg.strategies, &cfg.params) {
                bump(&mut signals, sig.strategy.code());
                if !sc.vg_in {
                    bump(&mut skips, "VOLUME_GATE");
                    continue;
                }
                let prio = cfg.strategies.iter().position(|x| *x == sig.strategy).unwrap_or(99);
                cands.push((prio, j, sig));
            }
        }
        cands.sort_by(|a, b| a.0.cmp(&b.0).then(a.1.cmp(&b.1)));
        for (_, j, sig) in cands {
            let si = &mkt.symbols[j];
            let in_universe = match dec.universe {
                UniverseScope::All => true,
                UniverseScope::Majors => majors.contains(&j),
                UniverseScope::None => false,
            };
            let ctx = GateContext {
                now: t_close,
                armed: true, // simulasi
                regime_ok: dec.state.is_trade_state() && dec.max_positions > 0 && in_universe,
                regime_side_ok: dec.side_ok(sig.side),
                regime_max_pos: dec.max_positions as usize,
                open_positions: open.len(),
                pending_entries: pending.len() + market_q.len(),
                symbol_busy: open.iter().any(|p| p.p.sym == j) || pending.iter().any(|p| p.sym == j) || market_q.iter().any(|p| p.sym == j),
                equity,
                spread_bps: None,
            };
            let kind = sig.kind;
            let inp = SizingInput { side: sig.side, entry: to_dec(sig.entry), sl: to_dec(sig.sl), kind, filters: &si.filters, risk: rc };
            match risk.evaluate_entry(&ctx, &inp, rc) {
                Ok(sz) => {
                    let ofcc_watch = if sig.strategy == StrategyId::S1Tfp {
                        match apex_strategies::ofcc_watch::S1Watch::new(sig.clone()) {
                            Some(w) => Some(w),
                            None => { bump(&mut skips, "OFCC_INVALID_SETUP"); continue; }
                        }
                    } else { None };
                    let p = Pending {
                        sym: j,
                        entry: to_f64(sz.entry),
                        sl: to_f64(sz.sl),
                        tp: to_f64(sz.tp),
                        qty: to_f64(sz.qty),
                        risk: to_f64(sz.risk),
                        margin: to_f64(sz.margin),
                        created_k: kk + 1,
                        tick: to_f64(si.filters.tick_size),
                        regime: dec.state.as_str().to_string(),
                        ofcc_watch,
                        sig,
                    };
                    match kind {
                        EntryKind::Market => market_q.push(p),
                        EntryKind::LimitMaker => pending.push(p),
                    }
                }
                Err(r) => bump(&mut skips, r.code()),
            }
        }
    }
    // end of data: tutup posisi di close terakhir
    for pos in std::mem::take(&mut open) {
        let si = &mkt.symbols[pos.p.sym];
        let last = si.m15.iter().rev().find(|b| b.open_time + Tf::M15.ms() <= cfg.end).map(|b| (b.close, b.open_time + Tf::M15.ms())).unwrap_or((pos.fill, cfg.end));
        let tr = close_trade(&pos, last.0, last.1, ExitKind::EndOfData, fee_t, si);
        wallet += tr.pnl_net;
        trades.push(tr);
    }
    let skipped_pending = pending.len() as u64;
    if skipped_pending > 0 {
        *skips.entry("PENDING_AT_END".into()).or_insert(0) += skipped_pending;
    }
    let metrics = Metrics::from_trades(&trades, rc.start_balance_usd);
    let mut per_strategy = BTreeMap::new();
    for id in &cfg.strategies {
        let v: Vec<Trade> = trades.iter().filter(|t| t.strategy == id.code()).cloned().collect();
        per_strategy.insert(id.code().to_string(), Metrics::from_trades(&v, rc.start_balance_usd));
    }
    Ok(BacktestReport {
        start: apex_core::time::fmt_utc(cfg.start),
        end: apex_core::time::fmt_utc(cfg.end),
        strategies: cfg.strategies.iter().map(|s| s.code().to_string()).collect(),
        regime_mode: cfg.regime_mode,
        trades,
        metrics,
        per_strategy,
        signals,
        skips,
        regime_bars,
        screener_trend_share: if sym_bars > 0 { trend_bars as f64 / sym_bars as f64 } else { 0.0 },
        data_gaps: gaps,
        final_wallet: wallet,
        liquidated,
    })
}

fn close_trade(pos: &Position, px: f64, exit_ms: Ms, kind: ExitKind, fee_t: f64, si: &SymbolInput) -> Trade {
    let s = pos.p.sig.side.sign();
    let fee_out = px * pos.p.qty * fee_t;
    let gross = s * (px - pos.fill) * pos.p.qty;
    let fees = pos.fee_in + fee_out;
    let pnl = gross - fees - pos.funding;
    Trade {
        symbol: si.symbol.clone(),
        strategy: pos.p.sig.strategy.code().to_string(),
        side: pos.p.sig.side,
        signal_ms: pos.p.sig.bar_close_ms,
        entry_ms: pos.entry_ms,
        exit_ms,
        entry: pos.fill,
        sl: pos.p.sl,
        tp: pos.p.tp,
        exit: px,
        qty: pos.p.qty,
        notional: pos.fill * pos.p.qty,
        margin: pos.p.margin,
        risk_planned: pos.p.risk,
        fees,
        funding: pos.funding,
        pnl_net: pnl,
        r: if pos.p.risk > 0.0 { pnl / pos.p.risk } else { 0.0 },
        exit_kind: kind,
        regime: pos.p.regime.clone(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rust_decimal::Decimal;
    use std::str::FromStr;

    fn walk(n: usize, seed: u64, p0: f64, regime_len: usize) -> Vec<Bar> {
        let mut s = seed;
        let mut p = p0;
        (0..n)
            .map(|i| {
                s ^= s << 13;
                s ^= s >> 7;
                s ^= s << 17;
                let u = (s >> 11) as f64 / (1u64 << 53) as f64;
                let drift = if (i / regime_len) % 3 == 0 { 0.0009 } else if (i / regime_len) % 3 == 1 { -0.0009 } else { 0.0 };
                let r = (u - 0.5) * 0.012 + drift;
                let o = p;
                p *= 1.0 + r;
                let v = 50_000.0 + (s % 20_000) as f64;
                let tb = v * (0.35 + 0.3 * ((s >> 17) % 100) as f64 / 100.0);
                Bar { open_time: 1_700_006_400_000 - 1_700_006_400_000 % 86_400_000 + i as i64 * 900_000, open: o, high: o.max(p) * (1.0 + u * 0.004), low: o.min(p) * (1.0 - (1.0 - u) * 0.004), close: p, volume: v, quote_volume: v * p * 10.0, trades: 2_000, taker_buy_base: tb }
            })
            .collect()
    }

    fn market(n: usize) -> MarketInput {
        let btc = walk(n, 1, 30_000.0, 700);
        let mk = |sym: &str, seed: u64, p0: f64| {
            let m15 = walk(n, seed, p0, 500 + seed as usize * 37);
            let h1 = apex_indicators::aggregate(&m15, Tf::M15, Tf::H1);
            let tick = Decimal::from_str(if p0 > 100.0 { "0.01" } else { "0.0001" }).unwrap();
            let step = Decimal::from_str(if p0 > 100.0 { "0.001" } else { "0.1" }).unwrap();
            SymbolInput { symbol: sym.into(), funding: (0..n / 32).map(|k| (m15[0].open_time + k as i64 * 28_800_000, 0.0001)).collect(), filters: SymbolFilters::synthetic(sym, tick, step, Decimal::from(5)), h1, m15, tradeable: true }
        };
        let btc_h1 = apex_indicators::aggregate(&btc, Tf::M15, Tf::H1);
        MarketInput {
            symbols: vec![mk("AAAUSDT", 11, 2000.0), mk("BBBUSDT", 12, 5.0), mk("CCCUSDT", 13, 1.2), mk("DDDUSDT", 14, 150.0)],
            btc_m15: btc.clone(),
            btc_h1,
            dom_h1: None,
        }
    }

    fn cfg(m: &MarketInput, start_i: usize, end_i: usize) -> BacktestConfig {
        BacktestConfig { start: m.btc_m15[start_i].open_time, end: m.btc_m15[end_i].open_time, regime_mode: RegimeMode::ScreenerOnly, volume_gate: false, ..Default::default() }
    }

    #[test]
    fn deterministic_and_risk_invariants() {
        let m = market(14_000);
        let c = cfg(&m, 4000, 13_999);
        let a = run_backtest(&m, &c).unwrap();
        let b = run_backtest(&m, &c).unwrap();
        assert_eq!(a.trades, b.trades);
        assert!(a.trades.len() >= 5, "terlalu sedikit trade sintetis: {} sinyal={:?} skip={:?}", a.trades.len(), a.signals, a.skips);
        for t in &a.trades {
            if t.strategy == StrategyId::S1Tfp.code() {
                assert!(t.entry_ms >= t.signal_ms + Tf::M15.ms(), "S1 cannot fill during its confirmation bar: {t:?}");
            }
            assert!(t.risk_planned >= 0.80 - 1e-9 && t.risk_planned <= 1.0 + 1e-9, "{t:?}");
            let slp = (t.entry - t.sl).abs() / t.entry;
            assert!(slp >= 0.02 - 1e-9 && slp <= 0.04 + 1e-9, "{t:?}");
            assert!((t.margin - t.notional / 25.0).abs() < 0.05 * t.margin.max(0.01) + 1e-6);
        }
        // max 2 posisi bersamaan
        for t in &a.trades {
            let overlap = a.trades.iter().filter(|o| o.entry_ms <= t.entry_ms && o.exit_ms > t.entry_ms).count();
            assert!(overlap <= 2, "overlap {overlap}");
        }
        // cooldown 1 jam setelah fill SL (bar close exit + 60 menit), stop harian ≤ 6 SL
        for s in a.trades.iter().filter(|t| t.exit_kind == ExitKind::StopLoss) {
            let exit_close = s.exit_ms - s.exit_ms.rem_euclid(900_000) + 900_000;
            for o in &a.trades {
                assert!(!(o.signal_ms >= exit_close && o.signal_ms < exit_close + 3_600_000), "sinyal dalam cooldown: {o:?}");
            }
        }
        let mut per_day: BTreeMap<i64, u32> = BTreeMap::new();
        for s in a.trades.iter().filter(|t| t.exit_kind == ExitKind::StopLoss) {
            *per_day.entry(apex_core::time::utc_day(s.exit_ms)).or_insert(0) += 1;
        }
        assert!(per_day.values().all(|&n| n <= 6));
    }

    /// No-look-ahead: menjalankan sampai `cut` di data penuh == di data yang dipotong di `cut`
    /// == di data yang masa depannya (> cut) dimutasi.
    #[test]
    fn backtest_no_lookahead() {
        let m = market(12_000);
        let c = cfg(&m, 4000, 9000);
        let full = run_backtest(&m, &c).unwrap();
        let cut_t = c.end;
        let trunc = |bars: &Vec<Bar>, tf: Tf| bars.iter().copied().filter(|b| b.open_time + tf.ms() <= cut_t).collect::<Vec<_>>();
        let mut m2 = market(12_000);
        m2.btc_m15 = trunc(&m.btc_m15, Tf::M15);
        m2.btc_h1 = trunc(&m.btc_h1, Tf::H1);
        for s in m2.symbols.iter_mut() {
            s.m15 = trunc(&s.m15, Tf::M15);
            s.h1 = trunc(&s.h1, Tf::H1);
        }
        let part = run_backtest(&m2, &c).unwrap();
        assert_eq!(full.trades, part.trades);
        assert_eq!(full.skips, part.skips);
        let mut m3 = market(12_000);
        for s in m3.symbols.iter_mut() {
            for b in s.m15.iter_mut().filter(|b| b.open_time >= cut_t) {
                b.close *= 1.5;
                b.high *= 1.6;
                b.low *= 0.6;
            }
            s.h1 = apex_indicators::aggregate(&s.m15, Tf::M15, Tf::H1);
        }
        let mutated = run_backtest(&m3, &c).unwrap();
        assert_eq!(full.trades, mutated.trades);
    }

    #[test]
    fn golden_rules_enforced() {
        let m = market(3000);
        let mut c = cfg(&m, 2000, 2900);
        c.risk.risk_usd = 2.0;
        assert!(run_backtest(&m, &c).is_err());
    }
}
