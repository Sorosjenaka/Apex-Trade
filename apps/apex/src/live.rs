//! Perintah berbasis data PUBLIK (fetch, reader, screen, record) + loop `run`.
use crate::cfg::{filters_from_ei, AllConfig};
use apex_backtest::context::{AltInput, RegimeTimeline};
use apex_backtest::screener_states;
use apex_core::config::Mode;
use apex_core::num::to_dec;
use apex_core::risk::GateContext;
use apex_core::sizing::SizingInput;
use apex_core::time::{fmt_utc, now_ms, parse_ymd};
use apex_core::types::{Bar, Ms, SkipReason, Tf};
use apex_data::csvio::{funding_path, kline_path, save_klines, Recorder};
use apex_data::PublicClient;
use apex_market_reader::{RegimeDecision, UniverseScope};
use apex_oms::creds::Credentials;
use apex_oms::mock::MockExchange;
use apex_oms::store::StateStore;
use apex_oms::{authorize, EntryIntent, ExecMode, FuturesExchange, OrderManager, UserEvent};
use apex_screener::allowlist::Allowlist;
use apex_screener::regime::SymState;
use apex_screener::volume_gate::{depth_within, GateMetrics, VolumeGateTracker};
use apex_strategies::{evaluate, SymbolSeries};
use serde_json::Value;
use std::collections::BTreeMap;
use std::path::Path;
use std::sync::Arc;

fn num(v: &Value) -> f64 {
    v.as_str().and_then(|s| s.parse().ok()).or_else(|| v.as_f64()).unwrap_or(f64::NAN)
}

pub async fn fetch(cfg: &AllConfig, data: &Path, from: &str, to: &str) -> anyhow::Result<()> {
    let pc = PublicClient::new(&cfg.engine.public_rest_base)?;
    let (s, e) = (parse_ymd(from).ok_or_else(|| anyhow::anyhow!("--from"))?, parse_ymd(to).ok_or_else(|| anyhow::anyhow!("--to"))?);
    let now = now_ms();
    let mut syms = vec!["BTCUSDT".to_string(), "BTCDOMUSDT".to_string()];
    syms.extend(cfg.universe());
    syms.dedup();
    for sym in &syms {
        for tf in [Tf::M15, Tf::H1] {
            let bars = pc.klines(sym, tf, s, e, now).await?;
            save_klines(&kline_path(data, sym, tf), tf, &bars)?;
            println!("{sym} {} {} bar", tf.binance(), bars.len());
        }
        if sym != "BTCDOMUSDT" {
            let f = pc.funding_history(sym, s, e).await?;
            let p = funding_path(data, sym);
            if let Some(d) = p.parent() {
                std::fs::create_dir_all(d)?;
            }
            let mut w = String::from("time,rate\n");
            for (t, r) in &f {
                w.push_str(&format!("{t},{r}\n"));
            }
            std::fs::write(&p, w)?;
            println!("{sym} funding {} baris", f.len());
        }
    }
    Ok(())
}

pub struct Snapshot {
    pub btc_m15: Vec<Bar>,
    pub btc_h1: Vec<Bar>,
    pub dom_h1: Option<Vec<Bar>>,
    pub btc_daily: Option<Vec<Bar>>,
    pub eth_daily: Option<Vec<Bar>>,
    pub syms: Vec<(String, Vec<Bar>, Vec<Bar>)>,
}

pub async fn snapshot(pc: &PublicClient, universe: &[String], now: Ms) -> anyhow::Result<Snapshot> {
    let btc_m15 = pc.recent_closed("BTCUSDT", Tf::M15, 1000, now).await?;
    let btc_h1 = pc.recent_closed("BTCUSDT", Tf::H1, 1000, now).await?;
    let dom_h1 = pc.recent_closed("BTCDOMUSDT", Tf::H1, 1000, now).await.ok().filter(|v| !v.is_empty());
    // Optional shadow-only daily context; failures never change trading gates.
    let btc_daily = pc.recent_closed("BTCUSDT", Tf::D1, 250, now).await.ok().filter(|v| !v.is_empty());
    let eth_daily = pc.recent_closed("ETHUSDT", Tf::D1, 250, now).await.ok().filter(|v| !v.is_empty());
    let mut syms = Vec::new();
    for s in universe {
        if s == "BTCUSDT" || s == "BTCDOMUSDT" {
            continue;
        }
        match (pc.recent_closed(s, Tf::M15, 1000, now).await, pc.recent_closed(s, Tf::H1, 1000, now).await) {
            (Ok(a), Ok(b)) if !a.is_empty() && !b.is_empty() => syms.push((s.clone(), a, b)),
            _ => eprintln!("  [lewati] {s}: klines gagal"),
        }
    }
    Ok(Snapshot { btc_m15, btc_h1, dom_h1, btc_daily, eth_daily, syms })
}

/// MR-09 is informational only and does not alter the regime decision.
fn reversal_shadow(s: &Snapshot) -> Option<(Ms, f64, f64)> {
    let eth = s.syms.iter().find(|(symbol, _, _)| symbol == "ETHUSDT")?;
    apex_market_reader::reversal::shadow_scores(&s.btc_h1, &eth.2)?.last().copied()
}

/// Independent uncalibrated stage/Fibonacci evidence, never used to gate trades.
fn reversal_details(s: &Snapshot) -> Option<serde_json::Value> {
    use apex_market_reader::reversal_context::{closed_context, hourly_context};
    let eth = s.syms.iter().find(|(symbol, _, _)| symbol == "ETHUSDT")?;
    let as_of = s.btc_h1.last()?.open_time + Tf::H1.ms();
    let build = |h: &[Bar], daily: &Option<Vec<Bar>>| serde_json::json!({
        "1h": hourly_context(h, 1, as_of), "2h": hourly_context(h, 2, as_of),
        "4h": hourly_context(h, 4, as_of),
        "1d": daily.as_ref().and_then(|v| closed_context(v, 24, as_of))
    });
    Some(serde_json::json!({"as_of": as_of, "btc": build(&s.btc_h1, &s.btc_daily), "eth": build(&eth.2, &s.eth_daily)}))
}

/// Keputusan regime pada close 15m terakhir; mesin dihangatkan 96 close sebelumnya (histeresis).
pub fn decide(s: &Snapshot) -> Option<RegimeDecision> {
    let closes: Vec<Ms> = s.btc_m15.iter().rev().take(96).rev().map(|b| b.open_time + Tf::M15.ms()).collect();
    let alts: Vec<AltInput> = s.syms.iter().map(|x| AltInput { h1: &x.2 }).collect();
    let tl = RegimeTimeline::build(&s.btc_m15, &s.btc_h1, s.dom_h1.as_deref(), &alts, &closes, false, apex_market_reader::ChopThresholds::default());
    tl.decisions.last().cloned()
}

pub async fn reader(cfg: &AllConfig) -> anyhow::Result<()> {
    let pc = PublicClient::new(&cfg.engine.public_rest_base)?;
    let now = now_ms();
    let s = snapshot(&pc, &cfg.universe(), now).await?;
    let d = decide(&s).ok_or_else(|| anyhow::anyhow!("data kurang"))?;
    println!("Market Reader @ {} (bar 15m tertutup terakhir: {})", fmt_utc(now), s.btc_m15.last().map(|b| fmt_utc(b.open_time + Tf::M15.ms())).unwrap_or_default());
    println!("  state     : {}  (raw {})", d.state.as_str(), d.raw.as_str());
    println!("  long_ok={} short_ok={} universe={:?} max_pos={}", d.long_ok, d.short_ok, d.universe, d.max_positions);
    println!("  alasan    : {}", d.reasons.join("; "));
    match reversal_shadow(&s) {
        Some((ts, up, down)) => println!("  MR-09 shadow @ {}: up={:.2} down={:.2}", fmt_utc(ts), up, down),
        None => println!("  MR-09 shadow: data tidak cukup atau tidak sinkron"),
    }
    if let Some(details) = reversal_details(&s) { println!("  MR-09 stage/Fib shadow: {details}"); }
    match apex_data::tv::scan(&["CRYPTOCAP:BTC.D", "CRYPTOCAP:TOTAL", "CRYPTOCAP:TOTAL3"]).await {
        Ok(m) => {
            for (k, (c, ch)) in m {
                println!("  TV {k:<18} {c:>14.2}  {ch:+.2}%");
            }
        }
        Err(e) => println!("  TV scanner tidak tersedia ({e}); dominance pakai proxy BTCDOMUSDT"),
    }
    Ok(())
}

pub struct LiveMeta {
    pub filters: BTreeMap<String, apex_core::SymbolFilters>,
    pub allow: Allowlist,
    pub metrics: BTreeMap<String, GateMetrics>,
    pub qv_rank: Vec<String>,
}

pub async fn live_meta(pc: &PublicClient, cfg: &AllConfig, depth_for: &[String]) -> anyhow::Result<LiveMeta> {
    let ei = pc.exchange_info().await?;
    let filters = filters_from_ei(&ei);
    let (allow, _denied) = Allowlist::from_exchange_info(&ei);
    let onboard: BTreeMap<String, i64> = ei["symbols"].as_array().into_iter().flatten().map(|s| (s["symbol"].as_str().unwrap_or("").to_string(), s["onboardDate"].as_i64().unwrap_or(0))).collect();
    let t24 = pc.ticker_24h().await?;
    let bt = pc.book_ticker().await?;
    let pi = pc.premium_index().await?;
    let now = now_ms();
    let mut metrics: BTreeMap<String, GateMetrics> = BTreeMap::new();
    for t in t24.as_array().into_iter().flatten() {
        let s = t["symbol"].as_str().unwrap_or("").to_string();
        if !allow.contains(&s) {
            continue;
        }
        let f = filters.get(&s);
        let m = GateMetrics {
            symbol: s.clone(),
            price: num(&t["lastPrice"]),
            quote_volume_24h: Some(num(&t["quoteVolume"])),
            trades_24h: t["count"].as_u64(),
            tick_size: f.map(|f| apex_core::num::to_f64(f.tick_size)),
            step_size: f.map(|f| apex_core::num::to_f64(f.step_size)),
            min_qty: f.map(|f| apex_core::num::to_f64(f.min_qty)),
            min_notional: f.map(|f| apex_core::num::to_f64(f.min_notional)),
            age_days: onboard.get(&s).filter(|x| **x > 0).map(|o| (now - o) as f64 / 86_400_000.0),
            ..Default::default()
        };
        metrics.insert(s, m);
    }
    for b in bt.as_array().into_iter().flatten() {
        if let Some(m) = metrics.get_mut(b["symbol"].as_str().unwrap_or("")) {
            let (bid, ask) = (num(&b["bidPrice"]), num(&b["askPrice"]));
            if bid > 0.0 && ask > 0.0 {
                m.spread_bps = Some((ask - bid) / ((ask + bid) / 2.0) * 1e4);
            }
        }
    }
    for p in pi.as_array().into_iter().flatten() {
        if let Some(m) = metrics.get_mut(p["symbol"].as_str().unwrap_or("")) {
            m.funding_rate = Some(num(&p["lastFundingRate"]));
        }
    }
    for s in depth_for {
        if let (Some(m), Ok(d)) = (metrics.get(s).cloned(), pc.depth(s, 100).await) {
            let lv = |k: &str| -> Vec<(f64, f64)> { d[k].as_array().into_iter().flatten().map(|r| (num(&r[0]), num(&r[1]))).collect() };
            if let Some((usd, _, _)) = depth_within(&lv("bids"), &lv("asks"), 0.1) {
                let mut m = m;
                m.depth_usd = Some(usd);
                metrics.insert(s.clone(), m);
            }
        }
    }
    let mut qv: Vec<(String, f64)> = metrics.iter().map(|(k, m)| (k.clone(), m.quote_volume_24h.unwrap_or(0.0))).collect();
    qv.sort_by(|a, b| b.1.total_cmp(&a.1));
    let _ = cfg;
    Ok(LiveMeta { filters, allow, metrics, qv_rank: qv.into_iter().map(|x| x.0).collect() })
}

pub async fn screen(cfg: &AllConfig, top: usize) -> anyhow::Result<()> {
    let pc = PublicClient::new(&cfg.engine.public_rest_base)?;
    let pre = live_meta(&pc, cfg, &[]).await?;
    let vg = &cfg.screener.volume_gate;
    let cands: Vec<String> = pre.qv_rank.iter().filter(|s| *s != "BTCUSDT" && pre.metrics[*s].quote_volume_24h.unwrap_or(0.0) >= vg.min_quote_volume_24h).take(40).cloned().collect();
    let meta = live_meta(&pc, cfg, &cands).await?;
    println!("Screener @ {} | allowlist crypto: {} simbol | kandidat volume: {}", fmt_utc(now_ms()), meta.allow.symbols.len(), cands.len());
    println!("  {:<14} {:>10} {:>9} {:>6} {:>10} {:<10} {}", "symbol", "qv24 $M", "trades", "sprd", "depth $k", "state", "gagal volume gate");
    let now = now_ms();
    let mut shown = 0;
    for s in &cands {
        let m = &meta.metrics[s];
        let fails = m.failures(vg);
        let state = if fails.is_empty() && shown < top {
            shown += 1;
            match (pc.recent_closed(s, Tf::H1, 1000, now).await, pc.recent_closed(s, Tf::M15, 400, now).await) {
                (Ok(h1), Ok(m15)) => screener_states(&h1, &m15, &cfg.screener.regime).last().map(|x| x.as_str()).unwrap_or("-"),
                _ => "ERR",
            }
        } else {
            "-"
        };
        println!(
            "  {:<14} {:>10.1} {:>9} {:>6.2} {:>10.1} {:<10} {}",
            s,
            m.quote_volume_24h.unwrap_or(0.0) / 1e6,
            m.trades_24h.unwrap_or(0),
            m.spread_bps.unwrap_or(f64::NAN),
            m.depth_usd.unwrap_or(f64::NAN) / 1e3,
            state,
            fails.join(",")
        );
    }
    println!("  (BTCUSDT = konteks saja; non-crypto/TradFi/pre-market ditolak allowlist default-deny)");
    Ok(())
}

pub async fn record(cfg: &AllConfig, data: &Path) -> anyhow::Result<()> {
    let mut syms = vec!["BTCUSDT".to_string(), "BTCDOMUSDT".to_string()];
    syms.extend(cfg.universe());
    let mut streams = Vec::new();
    for s in &syms {
        let l = s.to_lowercase();
        streams.push(format!("{l}@kline_15m"));
        streams.push(format!("{l}@kline_1h"));
    }
    let (tx, mut rx) = tokio::sync::mpsc::channel(1024);
    let base = cfg.engine.live_ws_base.clone(); // stream PUBLIK mainnet (tanpa key, read-only)
    tokio::spawn(async move {
        let _ = apex_data::ws::run_public_stream(&base, streams, tx).await;
    });
    let mut rec = Recorder::new(data);
    println!("Recorder aktif ({} simbol) → {} | Ctrl+C untuk berhenti", syms.len(), data.display());
    loop {
        tokio::select! {
            ev = rx.recv() => match ev {
                Some(apex_data::ws::WsEvent::ClosedKline { symbol, interval, bar }) => {
                    if let Some(tf) = Tf::parse(&interval) {
                        if rec.record(&symbol, tf, &bar)? {
                            println!("{} {symbol} {interval} close={}", fmt_utc(bar.open_time + tf.ms()), bar.close);
                        }
                    }
                }
                Some(_) => {}
                None => break,
            },
            _ = tokio::signal::ctrl_c() => break,
        }
    }
    Ok(())
}

// ---------------------------------------------------------------- run loop

#[allow(async_fn_in_trait)]
pub trait Feed {
    /// Paper: dorong harga mark publik ke MockExchange dan kembalikan event simulasi.
    async fn feed(&self, prices: &BTreeMap<String, f64>, now: Ms) -> Vec<UserEvent>;
}
impl Feed for MockExchange {
    async fn feed(&self, prices: &BTreeMap<String, f64>, now: Ms) -> Vec<UserEvent> {
        for (s, p) in prices {
            if p.is_finite() && *p > 0.0 {
                self.set_price(s, to_dec(*p), now);
            }
        }
        self.drain_events()
    }
}
impl Feed for Arc<apex_oms::binance::BinanceRest> {
    async fn feed(&self, _: &BTreeMap<String, f64>, _: Ms) -> Vec<UserEvent> {
        Vec::new()
    }
}

pub struct RunOpts {
    pub mode: Mode,
    pub once: bool,
}

pub async fn run(cfg: &mut AllConfig, o: RunOpts) -> anyhow::Result<()> {
    cfg.engine.mode = o.mode;
    let confirm = std::env::var(apex_oms::safety::LIVE_CONFIRM_ENV).ok();
    let mode = match authorize(&cfg.engine, confirm.as_deref()) {
        Ok(m) => m,
        Err(e) => {
            eprintln!("DITOLAK safety guard: {e}");
            eprintln!("Default aman: arm=false, mode=paper. Tidak ada order yang dikirim.");
            return Ok(());
        }
    };
    let pc = PublicClient::new(&cfg.engine.public_rest_base)?;
    let universe = cfg.universe();
    let meta = live_meta(&pc, cfg, &[]).await?;
    let sub = match mode {
        ExecMode::Paper => "paper",
        ExecMode::Testnet => "testnet",
        ExecMode::Live => "live",
    };
    let sdir = cfg.engine.state_dir.join(sub);
    let store = StateStore::file(&sdir.join("oms_state.json"));
    let audit = apex_core::audit::AuditLog::to_file(&sdir.join("audit.jsonl"))?;
    println!("apex run | mode={sub} | arm={} | universe={} | state={}", cfg.engine.arm, universe.len(), sdir.display());
    match mode {
        ExecMode::Paper => {
            let ex = MockExchange::new(cfg.risk.start_balance_usd);
            let mgr = OrderManager::new(ex, mode, cfg.engine.clone(), cfg.risk.clone(), store, audit, meta.filters.clone())?;
            run_loop(cfg, mgr, None, &pc, meta, o.once).await
        }
        ExecMode::Testnet | ExecMode::Live => {
            let creds = Credentials::from_env()?;
            let rest = Arc::new(apex_oms::binance::BinanceRest::new(mode, cfg.engine.rest_base(), creds, cfg.engine.recv_window_ms)?);
            let off = rest.sync_time().await?;
            println!("  offset jam server: {off} ms");
            if off.abs() > cfg.engine.max_clock_offset_ms * 10 {
                anyhow::bail!("offset jam {off} ms terlalu besar — sinkronkan jam Windows dulu");
            }
            let (tx, rx) = tokio::sync::mpsc::unbounded_channel();
            tokio::spawn(apex_oms::userstream::run_user_stream(rest.clone(), cfg.engine.ws_base().to_string(), tx));
            let mgr = OrderManager::new(rest, mode, cfg.engine.clone(), cfg.risk.clone(), store, audit, meta.filters.clone())?;
            run_loop(cfg, mgr, Some(rx), &pc, meta, o.once).await
        }
    }
}

async fn run_loop<E: FuturesExchange + Feed>(cfg: &AllConfig, mut mgr: OrderManager<E>, mut rx: Option<tokio::sync::mpsc::UnboundedReceiver<UserEvent>>, pc: &PublicClient, mut meta: LiveMeta, once: bool) -> anyhow::Result<()> {
    let now = now_ms();
    let paper = mgr.mode == ExecMode::Paper;
    if paper {
        // harga awal untuk simulasi
        let prices = mark_prices(pc).await.unwrap_or_default();
        for ev in mgr.ex.feed(&prices, now).await {
            mgr.on_event(ev, now).await;
        }
    }
    mgr.boot(now).await?;
    let ids = cfg.enabled();
    let mut vg = VolumeGateTracker::default();
    // Watches are deliberately in-memory: after restart an unconfirmed setup is discarded.
    let mut watches: BTreeMap<String, apex_strategies::ofcc_watch::S1Watch> = BTreeMap::new();
    let mut last_close: Ms = 0;
    let mut last_meta = now;
    let mut last_reconcile = now;
    loop {
        let now = now_ms();
        if paper {
            if let Ok(prices) = mark_prices(pc).await {
                for ev in mgr.ex.feed(&prices, now).await {
                    mgr.on_event(ev, now).await;
                }
            }
        }
        if let Some(rx) = rx.as_mut() {
            while let Ok(ev) = rx.try_recv() {
                mgr.on_event(ev, now).await;
            }
        }
        mgr.tick(now).await;
        if !paper && now - last_reconcile > 300_000 {
            if let Err(e) = mgr.reconcile(now).await {
                eprintln!("reconcile gagal: {e}");
            }
            last_reconcile = now;
        }
        if now - last_meta > 3_600_000 {
            if let Ok(m) = live_meta(pc, cfg, &[]).await {
                meta = m; // BL-R8: allowlist/filter di-refresh tiap jam
            }
            last_meta = now;
        }
        let cur_close = now - now.rem_euclid(Tf::M15.ms());
        if cur_close > last_close && now - cur_close >= 3_000 {
            last_close = cur_close;
            if let Err(e) = on_bar_close(cfg, &mut mgr, pc, &mut meta, &mut vg, &ids, &mut watches, now).await {
                eprintln!("{} siklus 15m gagal: {e}", fmt_utc(now));
            }
            if once {
                break;
            }
        }
        tokio::select! {
            _ = tokio::time::sleep(std::time::Duration::from_secs(cfg.engine.poll_interval_secs.max(1))) => {}
            _ = tokio::signal::ctrl_c() => { println!("Ctrl+C — state disimpan, keluar."); break; }
        }
    }
    Ok(())
}

async fn mark_prices(pc: &PublicClient) -> anyhow::Result<BTreeMap<String, f64>> {
    let v = pc.premium_index().await?;
    Ok(v.as_array().into_iter().flatten().map(|p| (p["symbol"].as_str().unwrap_or("").to_string(), num(&p["markPrice"]))).collect())
}

async fn on_bar_close<E: FuturesExchange + Feed>(cfg: &AllConfig, mgr: &mut OrderManager<E>, pc: &PublicClient, meta: &mut LiveMeta, vg: &mut VolumeGateTracker, ids: &[apex_strategies::StrategyId], watches: &mut BTreeMap<String, apex_strategies::ofcc_watch::S1Watch>, now: Ms) -> anyhow::Result<()> {
    let universe = cfg.universe();
    let snap = snapshot(pc, &universe, now).await?;
    let Some(dec) = decide(&snap) else { anyhow::bail!("regime: data kurang") };
    match reversal_shadow(&snap) {
        Some((ts, up, down)) => mgr.audit.log(now, "MR09_SHADOW", serde_json::json!({"as_of": ts, "rev_up": up, "rev_down": down, "context": reversal_details(&snap)})),
        None => mgr.audit.log(now, "MR09_SHADOW_UNAVAILABLE", serde_json::json!({"reason": "insufficient_or_unaligned_data", "context": reversal_details(&snap)})),
    }
    let fresh = live_meta(pc, cfg, &universe).await?;
    meta.metrics = fresh.metrics;
    meta.qv_rank = fresh.qv_rank;
    let majors: Vec<String> = meta.qv_rank.iter().filter(|s| *s != "BTCUSDT").take(10).cloned().collect();
    let equity = mgr.ex.wallet_balance().await.unwrap_or(cfg.risk.start_balance_usd);
    println!("{} regime={} long_ok={} short_ok={} max_pos={} equity=${:.2} open={} pending={}", fmt_utc(now), dec.state.as_str(), dec.long_ok, dec.short_ok, dec.max_positions, equity, mgr.open_count(), mgr.pending_count());
    // Missing symbol data invalidates a setup instead of replaying with incomplete candles.
    watches.retain(|sym, _| snap.syms.iter().any(|(s, _, _)| s == sym));
    for (sym, m15, h1) in &snap.syms {
        // Replay missed closed bars too, for pending orders surviving a delayed cycle or restart.
        if let Some(created) = mgr.trade_on(sym).map(|t| t.created_ms) {
            for bar in m15.iter().filter(|bar| bar.open_time + Tf::M15.ms() > created) {
                mgr.on_closed_price(sym, to_dec(bar.close), bar.open_time + Tf::M15.ms()).await?;
            }
        }
        let Some(f) = meta.filters.get(sym).cloned() else { continue };
        if !meta.allow.contains(sym) {
            continue;
        }
        let gm = meta.metrics.get(sym).cloned().unwrap_or_default();
        let fails = gm.failures(&cfg.screener.volume_gate);
        let included = vg.update(sym, fails.is_empty(), &fails, &cfg.screener.volume_gate);
        let series = SymbolSeries::new(sym, m15, h1);
        let t = m15.len() - 1;
        let states = screener_states(h1, m15, &cfg.screener.regime);
        let state = series.h1_idx_at(t).and_then(|h| states.get(h).copied()).unwrap_or(SymState::Blocked);
        let mut candidates: Vec<_> = evaluate(&series, t, state, ids, &cfg.strategies.params).into_iter().map(|s| (s, false)).collect();
        if let Some(w) = watches.get_mut(sym) {
            use apex_strategies::ofcc_watch::Outcome;
            let feature = if w.signal.side == apex_core::types::Side::Long { &series.long.f15 } else { &series.short.f15 };
            let outcome = w.observe(&m15[t], m15[t].open_time + Tf::M15.ms(), feature.p90_absdelta[t]);
            match outcome {
                Outcome::Waiting => {},
                Outcome::Confirmed => {
                    let mut signal = w.signal.clone();
                    signal.ttl_bars = w.remaining_bars();
                    signal.bar_close_ms = m15[t].open_time + Tf::M15.ms();
                    watches.remove(sym);
                    if signal.ttl_bars > 0 { candidates.insert(0, (signal, true)); }
                }
                Outcome::Rejected | Outcome::Expired => { watches.remove(sym); }
            }
        }
        for (sig, ofcc_confirmed) in candidates {
            let skip = |r: &str| mgr.audit.log(now, "SKIP", serde_json::json!({"symbol": sym, "strategy": sig.strategy.code(), "reason": r}));
            if !included {
                skip(SkipReason::VolumeGate.code());
                continue;
            }
            if mgr.mode != ExecMode::Paper && !cfg.is_go(sig.strategy) {
                skip(SkipReason::StrategyNotGo.code());
                continue;
            }
            let in_universe = match dec.universe {
                UniverseScope::All => true,
                UniverseScope::Majors => majors.contains(sym),
                UniverseScope::None => false,
            };
            let ctx = GateContext {
                now,
                armed: mgr.mode == ExecMode::Paper || cfg.engine.arm,
                regime_ok: dec.state.is_trade_state() && dec.max_positions > 0 && in_universe,
                regime_side_ok: dec.side_ok(sig.side) && state.dir() == sig.side.sign() as i8,
                regime_max_pos: dec.max_positions as usize,
                open_positions: mgr.open_count(),
                pending_entries: mgr.pending_count() + watches.len(),
                symbol_busy: mgr.symbol_busy(sym) || watches.contains_key(sym),
                equity,
                spread_bps: gm.spread_bps,
            };
            let features = if sig.strategy == apex_strategies::StrategyId::S5Rmp {
                let ss = if sig.side == apex_core::types::Side::Long { &series.long } else { &series.short };
                apex_strategies::knn::features_at(ss, t)
            } else { None };
            if sig.strategy == apex_strategies::StrategyId::S5Rmp && cfg.strategies.params.s5_knn_enabled {
                let samples: Vec<_> = mgr.book.closed.iter().filter_map(|trade| {
                    if trade.strategy != apex_strategies::StrategyId::S5Rmp.code() { return None; }
                    let pnl = trade.pnl_net?;
                    if !pnl.is_finite() { return None; }
                    Some(apex_strategies::knn::LabeledSample {
                        features: trade.knn_features?, signal_ms: trade.knn_signal_ms?,
                        closed_ms: trade.closed_ms?, win: pnl > 0.0,
                    })
                }).collect();
                let grade = features.and_then(|f| apex_strategies::knn::grade(&samples, f, sig.bar_close_ms));
                if !grade.is_some_and(|g| g.pass) { skip("S5_KNN_REJECTED_OR_UNAVAILABLE"); continue; }
            }
            let inp = SizingInput { side: sig.side, entry: to_dec(sig.entry), sl: to_dec(sig.sl), kind: sig.kind, filters: &f, risk: &cfg.risk };
            match mgr.book.risk.evaluate_entry(&ctx, &inp, &cfg.risk) {
                Ok(sz) => {
                    if sig.strategy == apex_strategies::StrategyId::S1Tfp && !ofcc_confirmed {
                        if let Some(w) = apex_strategies::ofcc_watch::S1Watch::new(sig.clone()) {
                            watches.insert(sym.clone(), w);
                        } else { skip("OFCC_INVALID_SETUP"); }
                        continue;
                    }
                    if ofcc_confirmed {
                        let crossed = if sig.side == apex_core::types::Side::Long { to_dec(m15[t].close) <= sz.entry } else { to_dec(m15[t].close) >= sz.entry };
                        if crossed { skip("OFCC_POST_ONLY_CROSS"); continue; }
                    }
                    let mut ttl_ms = sig.ttl_bars as i64 * Tf::M15.ms();
                    if ofcc_confirmed {
                        ttl_ms = ttl_ms.saturating_sub((now - sig.bar_close_ms).max(0));
                        if ttl_ms <= 0 { skip("OFCC_EXPIRED"); continue; }
                    }
                    let it = EntryIntent { symbol: sym.clone(), strategy: sig.strategy.code().to_string(), sizing: sz, kind: sig.kind, ttl_ms, cancel_beyond: sig.cancel_beyond.map(to_dec), knn_features: features, knn_signal_ms: features.map(|_| sig.bar_close_ms) };
                    match mgr.submit(it, now).await {
                        Ok(cid) => println!("  ENTRY {} {} {:?} cid={cid}", sig.strategy.code(), sym, sig.side),
                        Err(e) => println!("  entry {} {} ditolak: {e}", sig.strategy.code(), sym),
                    }
                }
                Err(r) => {
                    skip(r.code());
                    println!("  sinyal {} {} {:?} → SKIP {}", sig.strategy.code(), sym, sig.side, r.code());
                }
            }
        }
    }
    Ok(())
}

#[cfg(test)]
mod reversal_shadow_tests {
    use super::*;
    fn make_bars(n: usize, start: i64, width: i64, base: f64) -> Vec<Bar> {
        (0..n).map(|i| {
            let p = base * (1.0 + 0.0001 * i as f64 + 0.03 * (i as f64 / 9.0).sin());
            Bar { open_time:start + i as i64 * width, open:p, high:p*1.01, low:p*0.99, close:p,
                volume:100.0, quote_volume:100.0*p, trades:10, taker_buy_base:50.0 }
        }).collect()
    }
    #[test]
    fn shadow_context_is_available_without_touching_execution() {
        let h = Tf::H1.ms(); let d = Tf::D1.ms(); let end = 1000*h;
        let daily_end = end - end.rem_euclid(d);
        let s = Snapshot {
            btc_m15:vec![], btc_h1:make_bars(1000,0,h,100.0), dom_h1:None,
            btc_daily:Some(make_bars(250,daily_end-250*d,d,100.0)),
            eth_daily:Some(make_bars(250,daily_end-250*d,d,10.0)),
            syms:vec![("ETHUSDT".into(),vec![],make_bars(1000,0,h,10.0))],
        };
        assert!(reversal_shadow(&s).is_some());
        let details = reversal_details(&s).unwrap();
        for symbol in ["btc", "eth"] {
            for tf in ["1h", "2h", "4h", "1d"] {
                assert!(details[symbol][tf]["stage"].is_string(), "{symbol} {tf}");
                assert!(details[symbol][tf]["fib_618"].is_number(), "{symbol} {tf}");
            }
        }
    }
}
