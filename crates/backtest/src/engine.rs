//! Loop event backtest. Lihat doc crate untuk urutan & aturan fill.
use crate::context::{AltInput, RegimeMode, RegimeTimeline};
use crate::stats::Metrics;
use apex_core::config::RiskConfig;
use apex_core::filters::SymbolFilters;
use apex_core::num::{to_dec, to_f64};
use apex_core::risk::{ExitKind, GateContext, RiskState};
use apex_core::sizing::{EntryKind, SizingInput};
use apex_core::types::{Bar, Ms, Side, Tf};
use apex_market_reader::{ChopThresholds, RegimeDecision, RegimeState, UniverseScope};
use apex_screener::regime::{classify, ScreenerConfig, ScreenerMachine, SymState, SymbolFeatures};
use apex_strategies::{evaluate, Signal, StrategyId, StrategyParams, SymbolSeries};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

pub struct SymbolInput {
    pub symbol: String,
    pub m15: Vec<Bar>,
    pub m5: Option<Vec<Bar>>,
    pub h1: Vec<Bar>,
    /// Exchange onboarding time; first observed bar is a conservative fallback.
    pub onboard_ms: Option<Ms>,
    pub funding: Vec<(Ms, f64)>,
    pub filters: SymbolFilters,
    /// Simbol boleh ditradingkan (allowlist). BTCUSDT = konteks saja.
    pub tradeable: bool,
}

pub struct MarketInput {
    pub symbols: Vec<SymbolInput>,
    pub btc_m15: Vec<Bar>,
    pub btc_m5: Option<Vec<Bar>>,
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
    /// Research-only entry clock; global reader and symbol regime stay 15m/1h.
    #[serde(default = "default_entry_tf")]
    pub entry_tf: Tf,
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
    /// Ambang klausa CHOP Market Reader. Default = hipotesis dokumen.
    #[serde(default)]
    pub chop: ChopThresholds,
}

fn default_entry_tf() -> Tf { Tf::M15 }

fn closed_reader_decision<'a>(timeline: &'a RegimeTimeline, as_of: Ms, invalid: &'a RegimeDecision) -> &'a RegimeDecision {
    timeline.times.partition_point(|&time| time <= as_of).checked_sub(1)
        .filter(|&i| as_of - timeline.times[i] < Tf::M15.ms())
        .map(|i| &timeline.decisions[i]).unwrap_or(invalid)
}

fn valid_five_minute_bars(bars: &[Bar]) -> bool {
    !bars.is_empty() && bars.iter().all(|b| b.open_time.rem_euclid(Tf::M5.ms()) == 0 && b.is_valid())
        && bars.windows(2).all(|pair| pair[1].open_time - pair[0].open_time == Tf::M5.ms())
}

fn reader_permits_fill(decision: &RegimeDecision, side: Side) -> bool {
    decision.state.is_trade_state() && decision.side_ok(side) && decision.universe != UniverseScope::None
}

fn account_blocks_fill(risk: &RiskState, cfg: &RiskConfig, open: Ms, close: Ms) -> bool {
    risk.in_cooldown(open) || risk.in_cooldown(close) || risk.daily_stop_active(open, cfg) || risk.daily_stop_active(close, cfg)
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
            entry_tf: Tf::M15,
            cap_on_proxy: false,
            mmr: 0.01,
            fee_mult: 1.0,
            funding_mult: 1.0,
            extra_slip: 0.0,
            market_slip: 0.0002,
            volume_gate: true,
            min_quote_volume_24h: 20_000_000.0,
            min_trades_24h: 100_000,
            chop: ChopThresholds::default(),
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

fn symbol_trending_at_open(symbol: &SymCtx, as_of: Ms, side: Side) -> bool {
    let hours = symbol.series.h1();
    hours.partition_point(|b| b.open_time + Tf::H1.ms() <= as_of).checked_sub(1)
        .filter(|&i| as_of - (hours[i].open_time + Tf::H1.ms()) < Tf::H1.ms())
        .and_then(|i| symbol.states.get(i))
        .is_some_and(|state| state.dir() == side.sign() as i8)
}

fn listed_long_enough(symbol: &SymbolInput, as_of: Ms) -> bool {
    symbol.onboard_ms.or_else(|| symbol.m15.first().map(|bar| bar.open_time))
        .is_some_and(|listed| as_of >= listed.saturating_add(30 * 86_400_000))
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

fn hash_bars_full(bars: &[Bar]) -> u64 {
    use std::hash::{Hash, Hasher};
    let mut h = std::collections::hash_map::DefaultHasher::new();
    bars.len().hash(&mut h);
    for b in bars {
        b.open_time.hash(&mut h);
        b.open.to_bits().hash(&mut h);
        b.high.to_bits().hash(&mut h);
        b.low.to_bits().hash(&mut h);
        b.close.to_bits().hash(&mut h);
        b.volume.to_bits().hash(&mut h);
        b.quote_volume.to_bits().hash(&mut h);
        b.trades.hash(&mut h);
        b.taker_buy_base.to_bits().hash(&mut h);
    }
    h.finish()
}

fn hash_screener_cfg(c: &ScreenerConfig) -> u64 {
    use std::hash::{Hash, Hasher};
    let mut h = std::collections::hash_map::DefaultHasher::new();
    for x in [
        c.adx_trend, c.adx_strong, c.adx_range, c.er_trend, c.er_range,
        c.chop_trend, c.chop_range, c.srv_trend, c.srv_range,
        c.atr_ratio_trend, c.atr_ratio_range, c.atr_pct_min, c.atr_pct_max,
    ] {
        x.to_bits().hash(&mut h);
    }
    c.min_votes.hash(&mut h);
    c.min_range_flags.hash(&mut h);
    c.enter_bars.hash(&mut h);
    c.atr_median_bars.hash(&mut h);
    h.finish()
}

type SymBaseCache = (SymbolSeries, Vec<Ms>, Vec<f64>, Vec<f64>);

fn hash_strategy_params(p: &StrategyParams) -> u64 {
    use std::hash::{Hash, Hasher};
    let mut h = std::collections::hash_map::DefaultHasher::new();
    for x in [
        p.sl_buffer_atr, p.s1_gap_atr, p.s1_body_frac, p.s1_srv_min,
        p.s3_sbpo_min, p.s4_min_height, p.s5_distrank_min,
        p.s5_apexwave_min, p.s5_impulse_atr, p.sl_min_pct, p.sl_max_pct,
    ] {
        x.to_bits().hash(&mut h);
    }
    (
        p.s1_ttl, p.s2_pool_age, p.s2_reclaim_bars, p.s2_ttl,
        p.s3_retest_bars, p.s3_market, p.s3_ttl, p.s4_min_range,
        p.s4_max_range, p.s4_lps_bars, p.s4_ttl, p.s5_ttl,
    )
        .hash(&mut h);
    h.finish()
}

type SigBarCache = Vec<std::sync::OnceLock<Vec<Signal>>>;

fn get_sig_cache(s: &SymbolInput, entry: &[Bar], cfg: &BacktestConfig, step: i64) -> std::sync::Arc<SigBarCache> {
    use std::collections::HashMap;
    use std::sync::{Arc, Mutex, OnceLock};
    type SigKey = (String, i64, u64, u64, u64, u64, u64, Vec<StrategyId>);
    static SIG_CACHE: OnceLock<Mutex<HashMap<SigKey, Arc<SigBarCache>>>> = OnceLock::new();
    let h_entry = hash_bars_full(entry);
    let h_h1 = hash_bars_full(&s.h1);
    let h_m15 = if cfg.entry_tf == Tf::M15 { h_entry } else { hash_bars_full(&s.m15) };
    let key = (
        s.symbol.clone(),
        step,
        h_entry,
        h_h1,
        h_m15,
        hash_screener_cfg(&cfg.screener),
        hash_strategy_params(&cfg.params),
        cfg.strategies.clone(),
    );
    let lock = SIG_CACHE.get_or_init(|| Mutex::new(HashMap::new()));
    lock.lock().ok().and_then(|g| g.get(&key).cloned()).unwrap_or_else(|| {
        let built: Arc<SigBarCache> = Arc::new((0..entry.len()).map(|_| OnceLock::new()).collect());
        if let Ok(mut g) = lock.lock() {
            if g.len() >= 512 {
                g.clear();
            }
            g.insert(key, Arc::clone(&built));
        }
        built
    })
}

fn build_sym_ctx(s: &SymbolInput, entry: &[Bar], cfg: &BacktestConfig, step: i64) -> SymCtx {
    use std::collections::HashMap;
    use std::sync::{Arc, Mutex, OnceLock};
    static BASE_CACHE: OnceLock<Mutex<HashMap<(String, i64, u64, u64), Arc<SymBaseCache>>>> = OnceLock::new();
    static STATE_CACHE: OnceLock<Mutex<HashMap<(String, u64, u64, u64), Arc<Vec<SymState>>>>> = OnceLock::new();

    let h_entry = hash_bars_full(entry);
    let h_h1 = hash_bars_full(&s.h1);
    let h_m15 = if cfg.entry_tf == Tf::M15 { h_entry } else { hash_bars_full(&s.m15) };
    let base_key = (s.symbol.clone(), step, h_entry, h_h1);
    let base_lock = BASE_CACHE.get_or_init(|| Mutex::new(HashMap::new()));
    let base = base_lock.lock().ok().and_then(|g| g.get(&base_key).cloned()).unwrap_or_else(|| {
        let qv: Vec<f64> = entry.iter().map(|b| b.quote_volume).collect();
        let tr: Vec<f64> = entry.iter().map(|b| b.trades as f64).collect();
        let built = Arc::new((
            SymbolSeries::new_with_tf(&s.symbol, entry, &s.h1, cfg.entry_tf),
            entry.iter().map(|b| b.open_time).collect(),
            rolling_sum(&qv, (86_400_000 / step) as usize),
            rolling_sum(&tr, (86_400_000 / step) as usize),
        ));
        if let Ok(mut g) = base_lock.lock() {
            if g.len() >= 512 {
                g.clear();
            }
            g.insert(base_key, Arc::clone(&built));
        }
        built
    });

    let state_key = (s.symbol.clone(), h_h1, h_m15, hash_screener_cfg(&cfg.screener));
    let state_lock = STATE_CACHE.get_or_init(|| Mutex::new(HashMap::new()));
    let states = state_lock.lock().ok().and_then(|g| g.get(&state_key).cloned()).unwrap_or_else(|| {
        let built = Arc::new(screener_states(&s.h1, &s.m15, &cfg.screener));
        if let Ok(mut g) = state_lock.lock() {
            if g.len() >= 512 {
                g.clear();
            }
            g.insert(state_key, Arc::clone(&built));
        }
        built
    });

    SymCtx {
        series: base.0.clone(),
        states: (*states).clone(),
        ot15: base.1.clone(),
        qv24: base.2.clone(),
        tr24: base.3.clone(),
        vg_in: false,
        vg_fail: 0,
        vg_pass: 0,
    }
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

/// Never train a kNN seed across the locked holdout interval.
fn knn_training_start(first: Option<Ms>, start: Ms) -> anyhow::Result<Option<Ms>> {
    let Some(first) = first.filter(|&first| first < start) else { return Ok(None) };
    if crate::overlaps_holdout(first, start) {
        anyhow::bail!("kNN training range overlaps locked holdout");
    }
    Ok(Some(first))
}

/// Freeze the IS-trained grader under baseline costs when testing stress costs OOS.
fn knn_training_config(cfg: &BacktestConfig, first: Ms) -> BacktestConfig {
    let mut train = cfg.clone();
    train.start = first;
    train.end = cfg.start;
    train.strategies = vec![StrategyId::S5Rmp];
    train.params.s5_knn_enabled = false;
    train.fee_mult = 1.0;
    train.funding_mult = 1.0;
    train.extra_slip = 0.0;
    train
}

fn knn_sample(tr: &Trade, syms: &[SymCtx]) -> Option<apex_strategies::knn::LabeledSample> {
    if tr.strategy != StrategyId::S5Rmp.code() || tr.exit_kind == ExitKind::EndOfData || !tr.pnl_net.is_finite() { return None; }
    let sc = syms.iter().find(|s| s.series.symbol == tr.symbol)?;
    let index = sc.ot15.binary_search(&(tr.signal_ms - Tf::M15.ms())).ok()?;
    let series = if tr.side == Side::Long { &sc.series.long } else { &sc.series.short };
    let features = apex_strategies::knn::features_at(series, index)?;
    let tf = Tf::M15.ms();
    let closed_ms = tr.exit_ms.div_euclid(tf) * tf + if tr.exit_ms.rem_euclid(tf) == 0 { 0 } else { tf };
    Some(apex_strategies::knn::LabeledSample { features, signal_ms: tr.signal_ms, closed_ms, win: tr.pnl_net > 0.0 })
}

pub fn run_backtest(mkt: &MarketInput, cfg: &BacktestConfig) -> anyhow::Result<BacktestReport> {
    let rc = &cfg.risk;
    rc.validate_golden_rules().map_err(|e| anyhow::anyhow!("{e}"))?;
    if !matches!(cfg.entry_tf, Tf::M5 | Tf::M15) { anyhow::bail!("unsupported entry timeframe"); }
    if cfg.entry_tf == Tf::M5 && (cfg.regime_mode != RegimeMode::Reader || cfg.strategies.iter().any(|s| !matches!(s, StrategyId::S1Tfp | StrategyId::S3Sbr))) {
        anyhow::bail!("5m research supports only S1/S3 with the market reader");
    }
    let master: &[Bar] = if cfg.entry_tf == Tf::M5 { mkt.btc_m5.as_deref().ok_or_else(|| anyhow::anyhow!("BTC 5m data missing"))? } else { &mkt.btc_m15 };
    let entries: Vec<&[Bar]> = mkt.symbols.iter().map(|s| if cfg.entry_tf == Tf::M5 { s.m5.as_deref().filter(|bars| !bars.is_empty()).ok_or_else(|| anyhow::anyhow!("{} 5m data missing", s.symbol)) } else { Ok(s.m15.as_slice()) }).collect::<anyhow::Result<_>>()?;
    if master.is_empty() { anyhow::bail!("master clock data missing"); }
    if cfg.entry_tf == Tf::M5 && (!valid_five_minute_bars(master) || entries.iter().any(|bars| !valid_five_minute_bars(bars))) {
        anyhow::bail!("5m data contains a gap, malformed bar or alignment error");
    }
    if cfg.entry_tf == Tf::M5 {
        if master[0].open_time > cfg.start || master[master.len()-1].open_time + Tf::M5.ms() < cfg.end {
            anyhow::bail!("BTC 5m does not cover the requested window");
        }
        for (symbol, bars) in mkt.symbols.iter().zip(entries.iter()) {
            let first_15m = symbol.m15.first().ok_or_else(|| anyhow::anyhow!("{} 15m data missing", symbol.symbol))?.open_time;
            if symbol.h1.is_empty() || bars[0].open_time > first_15m.max(cfg.start) + Tf::M15.ms() || bars[bars.len()-1].open_time + Tf::M5.ms() < cfg.end {
                anyhow::bail!("{} 5m does not cover the requested history", symbol.symbol);
            }
        }
    }
    let step = cfg.entry_tf.ms();
    let ks: Vec<usize> = (0..master.len()).filter(|&i| master[i].open_time >= cfg.start && master[i].open_time + step <= cfg.end).collect();
    // Reader hysteresis advances only on closed 15m bars, even for 5m entries.
    let close_times: Vec<Ms> = mkt.btc_m15.iter().filter(|b| b.open_time >= cfg.start && b.open_time + Tf::M15.ms() <= cfg.end).map(|b| b.open_time + Tf::M15.ms()).collect();
    let alts: Vec<AltInput> = mkt.symbols.iter().filter(|s| s.symbol != "BTCUSDT").map(|s| AltInput { h1: &s.h1 }).collect();
    let timeline = match cfg.regime_mode {
        RegimeMode::Reader => RegimeTimeline::build(&mkt.btc_m15, &mkt.btc_h1, mkt.dom_h1.as_deref(), &alts, &close_times, cfg.cap_on_proxy, cfg.chop),
        RegimeMode::ScreenerOnly => RegimeTimeline::permissive(&close_times),
    };
    let mut syms: Vec<SymCtx> = mkt
        .symbols
        .iter()
        .zip(entries.iter())
        .map(|(s, entry)| {
            let _ = (s, entry);
            return build_sym_ctx(s, entry, cfg, step);
            #[allow(unreachable_code)]
            SymCtx {
                series: SymbolSeries::new_with_tf(&s.symbol, entry, &s.h1, cfg.entry_tf),
                states: Vec::new(),
                ot15: Vec::new(),
                qv24: Vec::new(),
                tr24: Vec::new(),




                vg_in: false,
                vg_fail: 0,
                vg_pass: 0,
            }
        })
        .collect();

    // Variant K: seed only with completed S5 trades from a separate pre-start IS replay.
    // This replay runs the same risk rules and is always bounded strictly before OOS.
    let sig_caches: Vec<std::sync::Arc<SigBarCache>> = mkt.symbols.iter().zip(entries.iter()).map(|(s, entry)| get_sig_cache(s, entry, cfg, step)).collect();
    let mut knn_memory = Vec::new();
    if cfg.params.s5_knn_enabled && cfg.strategies.contains(&StrategyId::S5Rmp) {
        if let Some(first) = knn_training_start(mkt.btc_m15.first().map(|b| b.open_time), cfg.start)? {
            let train = knn_training_config(cfg, first);
            let history = run_backtest(mkt, &train)?;
            knn_memory.extend(history.trades.iter().filter_map(|t| knn_sample(t, &syms)).filter(|s| s.closed_ms < cfg.start));
        }
    }
    let mut knn_seen = 0usize;
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
    let no_decision = RegimeDecision { as_of: 0, state: RegimeState::DataInvalid, raw: RegimeState::DataInvalid, long_ok: false, short_ok: false, universe: UniverseScope::None, max_positions: 0, reasons: vec!["15m regime unavailable or stale".into()] };

    for (kk, &bi) in ks.iter().enumerate() {
        let t_open = master[bi].open_time;
        let t_close = t_open + step;
        let dec = closed_reader_decision(&timeline, t_close, &no_decision);
        // Fills occur during this bar; only the reader decision known at its open is usable.
        let dec_at_open = closed_reader_decision(&timeline, t_open, &no_decision);
        // bar tiap simbol pada T
        let idx: Vec<Option<usize>> = syms.iter().map(|s| s.ot15.binary_search(&t_open).ok()).collect();
        let open_majors: Vec<usize> = if dec_at_open.universe == UniverseScope::Majors {
            let mut ranked: Vec<(usize, f64)> = syms.iter().enumerate().filter_map(|(j, sc)| {
                sc.ot15.partition_point(|&t| t + step <= t_open).checked_sub(1)
                    .map(|i| (j, sc.qv24[i])).filter(|(_, volume)| volume.is_finite())
            }).collect();
            ranked.sort_by(|a, b| b.1.total_cmp(&a.1).then(a.0.cmp(&b.0)));
            ranked.into_iter().take(10).map(|(j, _)| j).collect()
        } else { vec![] };
        let allowed_at_open = |j: usize| match dec_at_open.universe {
            UniverseScope::All => true,
            UniverseScope::Majors => open_majors.contains(&j),
            UniverseScope::None => false,
        };

        // --- 0. MARKET entry antrian dari close sebelumnya → isi di open bar ini
        for p in std::mem::take(&mut market_q) {
            if !reader_permits_fill(dec_at_open, p.sig.side) || !allowed_at_open(p.sym)
                || !symbol_trending_at_open(&syms[p.sym], t_open, p.sig.side)
                || open.len() >= dec_at_open.max_positions as usize || account_blocks_fill(&risk, rc, t_open, t_open) {
                bump(&mut skips, "ENTRY_READER_BLOCKED_AT_OPEN");
                continue;
            }
            let Some(i) = idx[p.sym] else {
                bump(&mut skips, "MARKET_NO_BAR");
                continue;
            };
            let b = &entries[p.sym][i];
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
            let b = &entries[pos.p.sym][i];
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
            if p.ofcc_watch.is_none() && (!reader_permits_fill(dec_at_open, p.sig.side) || !allowed_at_open(p.sym)
                || !symbol_trending_at_open(&syms[p.sym], t_open, p.sig.side)
                || open.len() >= dec_at_open.max_positions as usize || account_blocks_fill(&risk, rc, t_open, t_close)) {
                bump(&mut skips, "ENTRY_READER_BLOCKED_AT_OPEN");
                continue;
            }
            let Some(i) = idx[p.sym] else {
                keep.push(p);
                continue;
            };
            let si = &mkt.symbols[p.sym];
            let b = &entries[p.sym][i];
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
                        let d = dec;
                        let upnl: f64 = open.iter().map(|pos| {
                            idx[pos.p.sym].map(|j| pos.p.sig.side.sign() * (entries[pos.p.sym][j].close - pos.fill) * pos.p.qty).unwrap_or(0.0)
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
                // Baseline extra_slip = 0 → fill di limit. Stress G-05 menggeser fill berlawanan arah.
                let fill = p.entry * (1.0 + s * cfg.extra_slip);
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
                    let b = &entries[pos.p.sym][i];
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
                    let px = idx[pos.p.sym].map(|i| if pos.p.sig.side.sign() > 0.0 { entries[pos.p.sym][i].low } else { entries[pos.p.sym][i].high }).unwrap_or(pos.fill);
                    let tr = close_trade(&pos, px, t_close, ExitKind::Liquidation, fee_t, si);
                    wallet += tr.pnl_net;
                    risk.record_exit(ExitKind::Liquidation, tr.pnl_net, t_close, rc);
                    trades.push(tr);
                }
                wallet = wallet.max(0.0);
            }
        }

        // A label from this bar is stored with the bar close, and is only eligible on a later bar.
        if cfg.params.s5_knn_enabled {
            knn_memory.extend(trades[knn_seen..].iter().filter_map(|t| knn_sample(t, &syms)));
            knn_seen = trades.len();
        }
        // --- 5. keputusan di close bar
        bump(&mut regime_bars, dec.state.as_str());
        let mut upnl = 0.0;
        for pos in &open {
            if let Some(i) = idx[pos.p.sym] {
                let s = pos.p.sig.side.sign();
                upnl += s * (entries[pos.p.sym][i].close - pos.fill) * pos.p.qty;
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
            // Rules §4.3: a contract must have existed for at least 30 days.
            if !listed_long_enough(&mkt.symbols[j], t_close) {
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
            for sig in sig_caches[j][i].get_or_init(|| evaluate(&sc.series, i, state, &cfg.strategies, &cfg.params)).iter().cloned() {
                bump(&mut signals, sig.strategy.code());
                if !sc.vg_in {
                    bump(&mut skips, "VOLUME_GATE");
                    continue;
                }
                if sig.strategy == StrategyId::S5Rmp && cfg.params.s5_knn_enabled {
                    let side_series = if sig.side == Side::Long { &sc.series.long } else { &sc.series.short };
                    let grade = apex_strategies::knn::features_at(side_series, i)
                        .and_then(|f| apex_strategies::knn::grade(&knn_memory, f, t_close));
                    if !grade.is_some_and(|g| g.pass) { bump(&mut skips, "S5_KNN_REJECTED_OR_UNAVAILABLE"); continue; }
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
                        match apex_strategies::ofcc_watch::S1Watch::new_with_tf(sig.clone(), cfg.entry_tf) {
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
        let last = entries[pos.p.sym].iter().rev().find(|b| b.open_time + step <= cfg.end).map(|b| (b.close, b.open_time + step)).unwrap_or((pos.fill, cfg.end));
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

    #[test]
    fn five_minute_reader_uses_only_last_closed_fresh_quarter_hour() {
        let decision = |as_of, state| RegimeDecision { as_of, state, raw: state, long_ok: true, short_ok: true, universe: UniverseScope::All, max_positions: 2, reasons: vec![] };
        let invalid = decision(0, RegimeState::DataInvalid);
        let timeline = RegimeTimeline { times: vec![900_000, 1_800_000], decisions: vec![decision(900_000, RegimeState::RiskOnAlts), decision(1_800_000, RegimeState::RiskOff)] };
        assert_eq!(closed_reader_decision(&timeline, 899_999, &invalid).state, RegimeState::DataInvalid);
        assert_eq!(closed_reader_decision(&timeline, 1_799_999, &invalid).state, RegimeState::RiskOnAlts);
        assert_eq!(closed_reader_decision(&timeline, 1_800_000, &invalid).state, RegimeState::RiskOff);
        assert_eq!(closed_reader_decision(&timeline, 2_700_000, &invalid).state, RegimeState::DataInvalid);
        let chop = decision(900_000, RegimeState::ChopNoTrade);
        assert!(!reader_permits_fill(&chop, Side::Long));
        assert!(!reader_permits_fill(&chop, Side::Short));
    }

    #[test]
    fn five_minute_gap_is_rejected() {
        let mut bars = walk(3, 1, 10.0, 30);
        for (i, bar) in bars.iter_mut().enumerate() { bar.open_time = i as i64 * Tf::M5.ms(); }
        assert!(valid_five_minute_bars(&bars));
        bars[2].open_time += Tf::M5.ms();
        assert!(!valid_five_minute_bars(&bars));
    }

    #[test]
    fn five_minute_pending_fill_obeys_account_stops() {
        let rules = RiskConfig::default();
        let mut state = RiskState::new(0);
        assert!(!account_blocks_fill(&state, &rules, 0, Tf::M5.ms()));
        state.record_exit(ExitKind::StopLoss, -1.0, Tf::M5.ms(), &rules);
        assert!(account_blocks_fill(&state, &rules, 0, Tf::M5.ms()));
        state.cooldown_until = 0;
        state.daily_sl_count = rules.daily_sl_limit;
        assert!(account_blocks_fill(&state, &rules, Tf::M5.ms(), 2 * Tf::M5.ms()));
    }
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
            SymbolInput { symbol: sym.into(), funding: (0..n / 32).map(|k| (m15[0].open_time + k as i64 * 28_800_000, 0.0001)).collect(), filters: SymbolFilters::synthetic(sym, tick, step, Decimal::from(5)), onboard_ms: Some(m15[0].open_time - 31 * 86_400_000), h1, m15, m5: None, tradeable: true }
        };
        let btc_h1 = apex_indicators::aggregate(&btc, Tf::M15, Tf::H1);
        MarketInput {
            symbols: vec![mk("AAAUSDT", 11, 2000.0), mk("BBBUSDT", 12, 5.0), mk("CCCUSDT", 13, 1.2), mk("DDDUSDT", 14, 150.0)],
            btc_m15: btc.clone(),
            btc_m5: None,
            btc_h1,
            dom_h1: None,
        }
    }

    fn cfg(m: &MarketInput, start_i: usize, end_i: usize) -> BacktestConfig {
        BacktestConfig { start: m.btc_m15[start_i].open_time, end: m.btc_m15[end_i].open_time, regime_mode: RegimeMode::ScreenerOnly, volume_gate: false, ..Default::default() }
    }

    #[test]
    fn five_minute_replay_fails_closed_without_reader_data() {
        let mut m = market(400);
        let expand = |bars: &[Bar]| -> Vec<Bar> {
            bars.iter().flat_map(|b| (0..3).map(move |i| {
                let mut out = *b;
                out.open_time += i * Tf::M5.ms();
                out.volume /= 3.0;
                out.quote_volume /= 3.0;
                out.taker_buy_base /= 3.0;
                out
            })).collect()
        };
        m.btc_m5 = Some(expand(&m.btc_m15));
        for symbol in &mut m.symbols { symbol.m5 = Some(expand(&symbol.m15)); }
        let mut c = cfg(&m, 100, 390);
        c.entry_tf = Tf::M5;
        c.regime_mode = RegimeMode::Reader;
        c.strategies = vec![StrategyId::S1Tfp, StrategyId::S3Sbr];
        let report = run_backtest(&m, &c).unwrap();
        assert!(report.regime_bars.get("DATA_INVALID").copied().unwrap_or(0) > 0);
        assert!(report.trades.is_empty());
        c.regime_mode = RegimeMode::ScreenerOnly;
        assert!(run_backtest(&m, &c).is_err());
    }

    #[test]
    fn pending_fill_uses_only_closed_hourly_symbol_regime() {
        let m = market(100);
        let input = &m.symbols[0];
        let symbol = SymCtx { series: SymbolSeries::new(&input.symbol, &input.m15, &input.h1), states: vec![SymState::TrendUp; input.h1.len()], ot15: vec![], qv24: vec![], tr24: vec![], vg_in: true, vg_fail: 0, vg_pass: 0 };
        let close = input.h1[0].open_time + Tf::H1.ms();
        assert!(!symbol_trending_at_open(&symbol, close - 1, Side::Long));
        assert!(symbol_trending_at_open(&symbol, close, Side::Long));
        assert!(!symbol_trending_at_open(&symbol, close, Side::Short));
    }

    #[test]
    fn listing_age_is_independent_of_backtest_start() {
        let mut m = market(100);
        let symbol = &mut m.symbols[0];
        let listed = symbol.m15[0].open_time;
        symbol.onboard_ms = Some(listed);
        assert!(!listed_long_enough(symbol, listed + 30 * 86_400_000 - 1));
        assert!(listed_long_enough(symbol, listed + 30 * 86_400_000));
        symbol.onboard_ms = None;
        assert!(!listed_long_enough(symbol, listed + 29 * 86_400_000));
    }

    #[test]
    fn knn_stress_keeps_baseline_training_costs() {
        let mut c = BacktestConfig::default();
        c.start = 1000;
        c.params.s5_knn_enabled = true;
        c.fee_mult = 2.0;
        c.funding_mult = 2.0;
        c.extra_slip = 0.0005;
        let train = knn_training_config(&c, 100);
        assert_eq!((train.start, train.end), (100, 1000));
        assert_eq!((train.fee_mult, train.funding_mult, train.extra_slip), (1.0, 1.0, 0.0));
        assert!(!train.params.s5_knn_enabled);
        assert_eq!(train.risk.start_balance_usd, c.risk.start_balance_usd);
    }

    #[test]
    fn knn_training_never_crosses_holdout() {
        let day = |x| apex_core::time::parse_ymd(x).unwrap();
        let first = Some(day("2025-01-01"));
        assert_eq!(knn_training_start(first, day("2026-05-01")).unwrap(), first);
        assert!(knn_training_start(first, day("2026-05-02")).is_err());
        assert!(knn_training_start(first, day("2026-11-01")).is_err());
    }

    #[test]
    fn knn_label_is_available_only_after_exit_bar_close() {
        let m = market(500);
        let si = &m.symbols[0];
        let sc = SymCtx { series: SymbolSeries::new(&si.symbol, &si.m15, &si.h1), states: Vec::new(),
            ot15: si.m15.iter().map(|b| b.open_time).collect(), qv24: Vec::new(), tr24: Vec::new(), vg_in: true, vg_fail: 0, vg_pass: 0 };
        let signal_ms = si.m15[400].open_time + Tf::M15.ms();
        let tr = Trade { symbol:si.symbol.clone(), strategy:StrategyId::S5Rmp.code().into(), side:Side::Long,
            signal_ms, entry_ms:signal_ms+1, exit_ms:signal_ms+Tf::M15.ms()+1, entry:100., sl:97., tp:106.,
            exit:106., qty:1., notional:100., margin:4., risk_planned:1., fees:0., funding:0., pnl_net:1., r:1.,
            exit_kind:ExitKind::TakeProfit, regime:"test".into() };
        let sample = knn_sample(&tr, &[sc]).expect("valid features");
        assert_eq!(sample.closed_ms, signal_ms + 2 * Tf::M15.ms());
        let samples = vec![sample.clone(); 15];
        assert!(apex_strategies::knn::grade(&samples, sample.features, sample.closed_ms).is_none());
        assert!(apex_strategies::knn::grade(&samples, sample.features, sample.closed_ms + 1).unwrap().pass);
    }

    #[test]
    fn knn_training_replay_ignores_data_after_oos_end() {
        let first = market(1350);
        let mut second = market(1350);
        let cut = 1300;
        let mut c = cfg(&first, 1000, cut);
        c.strategies = vec![StrategyId::S5Rmp];
        c.params.s5_knn_enabled = true;
        for b in second.btc_m15.iter_mut().skip(cut) { b.open *= 1.2; b.high *= 1.2; b.low *= 1.2; b.close *= 1.2; }
        second.btc_h1 = apex_indicators::aggregate(&second.btc_m15, Tf::M15, Tf::H1);
        for sym in &mut second.symbols {
            for b in sym.m15.iter_mut().skip(cut) { b.open *= 1.2; b.high *= 1.2; b.low *= 1.2; b.close *= 1.2; b.volume *= 5.; }
            sym.h1 = apex_indicators::aggregate(&sym.m15, Tf::M15, Tf::H1);
        }
        let a = run_backtest(&first, &c).unwrap();
        let b = run_backtest(&second, &c).unwrap();
        assert_eq!(a.trades, b.trades);
        assert_eq!(a.signals, b.signals);
        assert_eq!(a.skips, b.skips);
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
