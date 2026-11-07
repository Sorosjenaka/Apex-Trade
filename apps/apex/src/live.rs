//! Perintah berbasis data PUBLIK (fetch, reader, screen, record) + loop `run`.
use crate::cfg::{filters_from_ei, AllConfig};
use apex_backtest::context::{AltInput, RegimeTimeline};
use apex_backtest::screener_states;
use apex_core::config::Mode;
use apex_core::num::{to_dec, to_f64};
use apex_core::risk::GateContext;
use apex_core::sizing::SizingInput;
use apex_core::time::{fmt_utc, now_ms, parse_ymd};
use apex_core::types::{Bar, Ms, Side, SkipReason, Tf};
use apex_data::csvio::{funding_path, kline_path, load_klines, save_klines, Recorder};
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
use std::time::Duration;

fn num(v: &Value) -> f64 {
    v.as_str().and_then(|s| s.parse().ok()).or_else(|| v.as_f64()).unwrap_or(f64::NAN)
}

fn fetch_intervals(only_5m: bool) -> &'static [Tf] {
    if only_5m { &[Tf::M5] } else { &[Tf::M15, Tf::H1] }
}

pub async fn fetch(cfg: &AllConfig, data: &Path, from: &str, to: &str, only_5m: bool) -> anyhow::Result<()> {
    let pc = PublicClient::new(&cfg.engine.public_rest_base)?;
    let (s, e) = (parse_ymd(from).ok_or_else(|| anyhow::anyhow!("--from"))?, parse_ymd(to).ok_or_else(|| anyhow::anyhow!("--to"))?);
    let now = now_ms();
    let mut syms = vec!["BTCUSDT".to_string(), "BTCDOMUSDT".to_string()];
    syms.extend(cfg.universe());
    syms.dedup();
    for sym in &syms {
        if only_5m && sym == "BTCDOMUSDT" {
            continue;
        }
        for &tf in fetch_intervals(only_5m) {
            let bars = pc.klines(sym, tf, s, e, now).await?;
            save_klines(&kline_path(data, sym, tf), tf, &bars)?;
            println!("{sym} {} {} bar", tf.binance(), bars.len());
        }
        if !only_5m && sym != "BTCDOMUSDT" {
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
    pub syms_m5: BTreeMap<String, Vec<Bar>>,
}

/// Ambil bar tertutup terakhir dari REST, atau fallback ke CSV lokal bila sedang cooldown rate-limit.
async fn recent_closed_or_csv(pc: &PublicClient, data_dir: &Path, sym: &str, tf: Tf, n: usize, now: Ms) -> anyhow::Result<Vec<Bar>> {
    match pc.recent_closed(sym, tf, n, now).await {
        Ok(v) if !v.is_empty() => Ok(v),
        Err(e) => {
            let p = kline_path(data_dir, sym, tf);
            if p.exists() {
                let mut rows = load_klines(&p)?;
                rows.retain(|b| b.available_at(tf) <= now);
                if rows.len() > n {
                    rows = rows.split_off(rows.len() - n);
                }
                if !rows.is_empty() {
                    return Ok(rows);
                }
            }
            Err(e)
        }
        Ok(_) => anyhow::bail!("{sym} {}: kosong", tf.binance()),
    }
}

pub async fn snapshot(pc: &PublicClient, universe: &[String], now: Ms) -> anyhow::Result<Snapshot> {
    snapshot_with_cfg(pc, Path::new("data"), universe, false, now).await
}

pub async fn snapshot_with_cfg(pc: &PublicClient, data_dir: &Path, universe: &[String], want_5m: bool, now: Ms) -> anyhow::Result<Snapshot> {
    let btc_m15 = recent_closed_or_csv(pc, data_dir, "BTCUSDT", Tf::M15, 450, now).await?;
    let btc_h1 = recent_closed_or_csv(pc, data_dir, "BTCUSDT", Tf::H1, 800, now).await?;
    let dom_h1 = recent_closed_or_csv(pc, data_dir, "BTCDOMUSDT", Tf::H1, 450, now).await.ok().filter(|v| !v.is_empty());
    // Optional shadow-only daily context; failures never change trading gates.
    let btc_daily = recent_closed_or_csv(pc, data_dir, "BTCUSDT", Tf::D1, 250, now).await.ok().filter(|v| !v.is_empty());
    let eth_daily = recent_closed_or_csv(pc, data_dir, "ETHUSDT", Tf::D1, 250, now).await.ok().filter(|v| !v.is_empty());
    let mut syms = Vec::new();
    let mut syms_m5 = BTreeMap::new();
    for s in universe {
        if s == "BTCUSDT" || s == "BTCDOMUSDT" {
            continue;
        }
        match (
            recent_closed_or_csv(pc, data_dir, s, Tf::M15, 450, now).await,
            recent_closed_or_csv(pc, data_dir, s, Tf::H1, 450, now).await,
        ) {
            (Ok(a), Ok(b)) if !a.is_empty() && !b.is_empty() => {
                if want_5m {
                    if let Ok(m5) = recent_closed_or_csv(pc, data_dir, s, Tf::M5, 480, now).await {
                        if !m5.is_empty() {
                            syms_m5.insert(s.clone(), m5);
                        }
                    }
                }
                syms.push((s.clone(), a, b));
            }
            _ => eprintln!("  [lewati] {s}: klines gagal"),
        }
    }
    Ok(Snapshot { btc_m15, btc_h1, dom_h1, btc_daily, eth_daily, syms, syms_m5 })
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
    decide_with_chop(s, apex_market_reader::ChopThresholds::default())
}

pub fn decide_with_chop(s: &Snapshot, chop: apex_market_reader::ChopThresholds) -> Option<RegimeDecision> {
    let closes: Vec<Ms> = s.btc_m15.iter().rev().take(96).rev().map(|b| b.open_time + Tf::M15.ms()).collect();
    let btc_last_h1 = s.btc_h1.last().map(|b| b.open_time).unwrap_or(0);
    let synced_alts: Vec<AltInput> = s
        .syms
        .iter()
        .filter(|x| x.2.last().is_some_and(|b| b.open_time + 2 * Tf::H1.ms() >= btc_last_h1))
        .map(|x| AltInput { h1: &x.2 })
        .collect();
    let alts: Vec<AltInput> = if synced_alts.is_empty() {
        s.syms.iter().map(|x| AltInput { h1: &x.2 }).collect()
    } else {
        synced_alts
    };
    let dom = s
        .dom_h1
        .as_deref()
        .filter(|d| d.last().is_some_and(|b| b.open_time + 2 * Tf::H1.ms() >= btc_last_h1));
    let tl = RegimeTimeline::build(&s.btc_m15, &s.btc_h1, dom, &alts, &closes, false, chop);
    tl.decisions.last().cloned()
}

pub async fn reader(cfg: &AllConfig) -> anyhow::Result<()> {
    let pc = PublicClient::new(&cfg.engine.public_rest_base)?;
    let now = now_ms();
    let s = snapshot_with_cfg(&pc, &cfg.engine.data_dir, &cfg.universe(), false, now).await?;
    let d = decide_with_chop(&s, cfg.screener.chop).ok_or_else(|| anyhow::anyhow!("data kurang"))?;
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
    let ei = match pc.exchange_info().await {
        Ok(v) => v,
        Err(_) => {
            let snap_p = cfg.dir.join("filters_snapshot.json");
            serde_json::from_str(&std::fs::read_to_string(&snap_p)?)?
        }
    };
    let filters = filters_from_ei(&ei);
    let (allow, _denied) = Allowlist::from_exchange_info(&ei);
    let onboard: BTreeMap<String, i64> = ei["symbols"].as_array().into_iter().flatten().map(|s| (s["symbol"].as_str().unwrap_or("").to_string(), s["onboardDate"].as_i64().unwrap_or(0))).collect();
    let now = now_ms();
    let mut metrics: BTreeMap<String, GateMetrics> = BTreeMap::new();
    if let Ok(t24) = pc.ticker_24h().await {
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
    } else {
        // Fallback metrics when ticker_24h is in rate-limit cooldown so paper loop keeps running
        for (s, f) in &filters {
            if !allow.contains(s) {
                continue;
            }
            metrics.insert(
                s.clone(),
                GateMetrics {
                    symbol: s.clone(),
                    price: 0.0,
                    quote_volume_24h: Some(500_000_000.0),
                    trades_24h: Some(250_000),
                    spread_bps: Some(1.0),
                    depth_usd: Some(250_000.0),
                    funding_rate: Some(0.0001),
                    tick_size: Some(apex_core::num::to_f64(f.tick_size)),
                    step_size: Some(apex_core::num::to_f64(f.step_size)),
                    min_qty: Some(apex_core::num::to_f64(f.min_qty)),
                    min_notional: Some(apex_core::num::to_f64(f.min_notional)),
                    age_days: Some(365.0),
                },
            );
        }
    }
    if let Ok(bt) = pc.book_ticker().await {
        for b in bt.as_array().into_iter().flatten() {
            if let Some(m) = metrics.get_mut(b["symbol"].as_str().unwrap_or("")) {
                let (bid, ask) = (num(&b["bidPrice"]), num(&b["askPrice"]));
                if bid > 0.0 && ask > 0.0 {
                    m.spread_bps = Some((ask - bid) / ((ask + bid) / 2.0) * 1e4);
                }
            }
        }
    }
    if let Ok(pi) = pc.premium_index().await {
        for p in pi.as_array().into_iter().flatten() {
            if let Some(m) = metrics.get_mut(p["symbol"].as_str().unwrap_or("")) {
                m.funding_rate = Some(num(&p["lastFundingRate"]));
            }
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
            match (pc.recent_closed(s, Tf::H1, 450, now).await, pc.recent_closed(s, Tf::M15, 400, now).await) {
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
        if cfg.strategies.research_entry_5m && s != "BTCDOMUSDT" {
            streams.push(format!("{l}@kline_5m"));
        }
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
    let meta = live_meta(&pc, cfg, &universe).await?;
    let sub = match mode {
        ExecMode::Paper => "paper",
        ExecMode::Testnet => "testnet",
        ExecMode::Live => "live",
    };
    let sdir = cfg.engine.state_dir.join(sub);
    let store = StateStore::file(&sdir.join("oms_state.json"));
    let audit = apex_core::audit::AuditLog::to_file(&sdir.join("audit.jsonl"))?;
    let entry_tf = if cfg.strategies.research_entry_5m { Tf::M5 } else { Tf::M15 };
    println!(
        "apex run | mode={sub} | arm={} | entry_tf={} | universe={} | state={}",
        cfg.engine.arm,
        entry_tf.binance(),
        universe.len(),
        sdir.display()
    );
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
    let universe = cfg.universe();
    if paper {
        let prices = mark_prices_for(pc, &universe).await.unwrap_or_default();
        for ev in mgr.ex.feed(&prices, now).await {
            mgr.on_event(ev, now).await;
        }
    }
    mgr.boot(now).await?;
    let ids = cfg.enabled();
    let mut vg = VolumeGateTracker::default();
    let mut watches: BTreeMap<String, apex_strategies::ofcc_watch::S1Watch> = BTreeMap::new();
    let mut yahudi_cooldown = apex_oms::YahudiCooldownGuard::default();
    let mut playbook = apex_oms::PlaybookEngine::default();
    let mut trend_flip_guard = apex_oms::TrendFlipConfirmationGuard::default();
    let mut rate_guard = apex_oms::BinanceRateGuard::default();
    let mut seen_closed = mgr.book.closed.len();
    let mut last_close: Ms = 0;
    let mut last_meta = now;
    let mut last_reconcile = now;
    let mut siklus: u64 = 0;
    let entry_tf = if cfg.strategies.research_entry_5m { Tf::M5 } else { Tf::M15 };
    loop {
        let now = now_ms();
        // Di mode paper, hanya query mark_prices di sela bar close bila ada order aktif/pending di MockExchange
        if paper && (mgr.open_count() > 0 || mgr.pending_count() > 0) {
            let active_syms: Vec<String> = mgr.book.trades.values().map(|t| t.symbol.clone()).collect();
            let _ = rate_guard.try_acquire(
                "GET /fapi/v1/premiumIndex",
                active_syms.len().max(1) as u32,
                apex_oms::RatePriority::Critical,
                now,
            );
            if let Ok(prices) = mark_prices_for(pc, &active_syms).await {
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
        // Sinkronisasi penutupan posisi baru ke Yahudi Cooldown Guard, Playbook Mistake Classifier & 5-Whys Forensics
        while seen_closed < mgr.book.closed.len() {
            let ct = mgr.book.closed[seen_closed].clone();
            seen_closed += 1;
            trend_flip_guard.clear(&ct.symbol, ct.side);
            let pnl = ct.pnl_net.unwrap_or(0.0);
            let close_reason = ct
                .exit_kind
                .map(|k| format!("{k:?}"))
                .unwrap_or_else(|| "MANUAL".to_string());
            let closed_ts = ct.closed_ms.unwrap_or(now);
            let hold_ms = closed_ts.saturating_sub(ct.filled_ms.unwrap_or(ct.created_ms));
            let sl_pct = if ct.entry_price > rust_decimal::Decimal::ZERO {
                to_f64((ct.entry_price - ct.sl).abs() / ct.entry_price)
            } else {
                0.02
            };
            yahudi_cooldown.on_trade_closed(
                &ct.symbol,
                closed_ts,
                pnl,
                &close_reason,
                cfg.risk.risk_usd,
            );
            let mistake = apex_oms::classify_trade_mistake(
                pnl,
                hold_ms,
                0.0003,
                0.0,
                true,
                50.0,
                sl_pct,
            );
            playbook.record_mistake(mistake);
            let r_mult = if cfg.risk.risk_usd > 0.0 {
                pnl / cfg.risk.risk_usd
            } else {
                0.0
            };
            let five_whys = apex_oms::generate_five_whys_forensics(
                &ct.symbol,
                &ct.strategy,
                pnl,
                r_mult,
                r_mult.max(0.0),
                (-r_mult).max(0.0),
                mistake.as_str(),
            );
            mgr.audit.log(
                closed_ts,
                "YAHUDI_POST_TRADE",
                serde_json::json!({
                    "symbol": ct.symbol,
                    "strategy": ct.strategy,
                    "pnl_net": pnl,
                    "close_reason": close_reason,
                    "mistake_tag": mistake.as_str(),
                    "five_whys": five_whys,
                    "global_cooldown_until_ms": yahudi_cooldown.global_cooldown_until_ms,
                    "global_cooldown_reason": yahudi_cooldown.global_cooldown_reason,
                    "tripwire_triggered": yahudi_cooldown.tripwire_triggered,
                }),
            );
        }
        if !paper && now - last_reconcile > 300_000 {
            if let Err(e) = mgr.reconcile(now).await {
                eprintln!("reconcile gagal: {e}");
            }
            last_reconcile = now;
        }
        if now - last_meta > 1_800_000 {
            let _ = rate_guard.try_acquire(
                "GET /fapi/v1/ticker/24hr",
                40,
                apex_oms::RatePriority::Normal,
                now,
            );
            if let Ok(m) = live_meta(pc, cfg, &universe).await {
                meta = m;
            }
            last_meta = now;
        }
        let cur_close = now - now.rem_euclid(entry_tf.ms());
        if cur_close > last_close && now - cur_close >= 2_500 {
            last_close = cur_close;
            siklus += 1;
            if let Err(e) = on_bar_close(cfg, &mut mgr, pc, &mut meta, &mut vg, &ids, &mut watches, &mut yahudi_cooldown, &mut playbook, &mut trend_flip_guard, &mut rate_guard, now, siklus).await {
                eprintln!("{} siklus {} gagal: {e}", fmt_utc(now), entry_tf.binance());
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

/// Ambil mark price hanya untuk simbol yang diperlukan (weight 1 per simbol bila <= 6 simbol).
async fn mark_prices_for(pc: &PublicClient, syms: &[String]) -> anyhow::Result<BTreeMap<String, f64>> {
    let mut out = BTreeMap::new();
    if syms.is_empty() {
        return Ok(out);
    }
    if syms.len() <= 6 {
        for s in syms {
            if let Ok(v) = pc.premium_index_symbol(s).await {
                let px = num(&v["markPrice"]);
                if px.is_finite() && px > 0.0 {
                    out.insert(s.clone(), px);
                }
            }
        }
        if !out.is_empty() {
            return Ok(out);
        }
    }
    let v = pc.premium_index().await?;
    Ok(v.as_array().into_iter().flatten().map(|p| (p["symbol"].as_str().unwrap_or("").to_string(), num(&p["markPrice"]))).collect())
}

/// Ambil konteks mikrostruktur & Vike dari dashboard lokal (127.0.0.1:8787) bila aktif (0 weight eksternal).
async fn fetch_local_micro_context(sym: &str) -> Option<Value> {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    let fut = async {
        let mut stream = tokio::net::TcpStream::connect("127.0.0.1:8787").await.ok()?;
        let req = format!(
            "GET /api/micro/snapshot?symbol={sym}&tab=largevp&window=15m HTTP/1.1\r\nHost: 127.0.0.1:8787\r\nConnection: close\r\n\r\n"
        );
        stream.write_all(req.as_bytes()).await.ok()?;
        let mut buf = Vec::new();
        stream.read_to_end(&mut buf).await.ok()?;
        let s = String::from_utf8(buf).ok()?;
        let (_, body) = s.split_once("\r\n\r\n")?;
        let v: Value = serde_json::from_str(body.trim()).ok()?;
        Some(serde_json::json!({
            "poc": v.get("poc"),
            "vah": v.get("vah"),
            "val": v.get("val"),
            "mark": v.get("mark"),
            "large_trades_count": v.get("large").and_then(|a| a.as_array()).map(|a| a.len()).unwrap_or(0),
        }))
    };
    tokio::time::timeout(Duration::from_millis(350), fut).await.ok().flatten()
}

fn build_s1_trace_json(
    now: Ms,
    sym: &str,
    siklus: u64,
    series: &SymbolSeries,
    t: usize,
    state: SymState,
    dec_long_ok: bool,
    dec_short_ok: bool,
    included: bool,
    fails: &[&str],
    f: &apex_core::SymbolFilters,
    cfg: &AllConfig,
    s1_cand_side: Option<apex_core::types::Side>,
    s1_cand_entry: Option<f64>,
    s1_cand_sl: Option<f64>,
    s1_sized: Option<(f64, f64)>,
    ofcc_trace: Option<(apex_strategies::ofcc_watch::Outcome, u32, u32, apex_core::types::Side)>,
    s1_stop: Option<&str>,
) -> (Value, String) {
    let trace_side = s1_cand_side
        .or_else(|| ofcc_trace.map(|x| x.3))
        .unwrap_or_else(|| match state {
            SymState::TrendDown => apex_core::types::Side::Short,
            SymState::TrendUp => apex_core::types::Side::Long,
            _ => {
                if !dec_long_ok && dec_short_ok {
                    apex_core::types::Side::Short
                } else {
                    apex_core::types::Side::Long
                }
            }
        });
    let side_series = match trace_side {
        apex_core::types::Side::Long => &series.long,
        apex_core::types::Side::Short => &series.short,
    };
    let view = apex_strategies::make_view(series, side_series, t, state);
    let session_ok = apex_strategies::session_ok(view.as_of, &cfg.strategies.params.s1_session);
    let session_hour_utc = apex_strategies::session_hour_utc(view.as_of);
    let tr = apex_strategies::trace_s1(&view, &cfg.strategies.params);

    let sl_val = s1_cand_sl.or(tr.sl);
    let sl_pct_val = match (s1_cand_entry, s1_cand_sl) {
        (Some(e), Some(s)) if e > 0.0 && e.is_finite() && s.is_finite() => Some((e - s).abs() / e),
        _ => tr.sl_pct,
    };

    let (ofcc_outcome, ofcc_seen, ofcc_remaining_bars) = match ofcc_trace {
        Some((out, seen, rem, _)) => (out.as_str(), seen, rem),
        None => ("none", 0, 0),
    };

    let min_qty = apex_core::num::to_f64(f.sizing_min_qty());
    let min_notional = apex_core::num::to_f64(f.min_notional);
    let (sizing_qty, notional) = if let Some((q, n)) = s1_sized {
        (Some(q), Some(n))
    } else if let (Some(e_raw), Some(s_raw)) = (s1_cand_entry.or(tr.entry_est), sl_val) {
        let entry_d = f.round_entry(trace_side, to_dec(e_raw));
        let sl_d = f.round_sl(trace_side, to_dec(s_raw));
        let fee_in = to_dec(cfg.risk.fee_maker);
        let fee_out = to_dec(cfg.risk.fee_taker);
        let slip = to_dec(cfg.risk.slip_sl);
        let sgn = match trace_side {
            apex_core::types::Side::Long => rust_decimal::Decimal::ONE,
            apex_core::types::Side::Short => -rust_decimal::Decimal::ONE,
        };
        let stop_fill = sl_d * (rust_decimal::Decimal::ONE - sgn * slip);
        let loss_unit = sgn * (entry_d - stop_fill) + entry_d * fee_in + stop_fill * fee_out;
        if entry_d > rust_decimal::Decimal::ZERO && loss_unit > rust_decimal::Decimal::ZERO {
            let q = f.floor_qty(to_dec(cfg.risk.risk_usd) / loss_unit);
            let n = q * entry_d;
            (Some(apex_core::num::to_f64(q)), Some(apex_core::num::to_f64(n)))
        } else {
            (None, None)
        }
    } else {
        (None, None)
    };

    let stop_at: &str = if let Some(s) = s1_stop {
        s
    } else if let Some((out, _, _, _)) = ofcc_trace {
        match out {
            apex_strategies::ofcc_watch::Outcome::Waiting => "ofcc_waiting",
            apex_strategies::ofcc_watch::Outcome::Confirmed => "ofcc_confirmed_expired",
            apex_strategies::ofcc_watch::Outcome::Rejected => "ofcc_rejected",
            apex_strategies::ofcc_watch::Outcome::Expired => "ofcc_expired",
        }
    } else if !included {
        "volume_gate"
    } else if let Some(fail) = tr.first_fail {
        fail
    } else {
        "signal"
    };

    let payload = serde_json::json!({
        "ts": now,
        "symbol": sym,
        "siklus": siklus,
        "session_ok": session_ok,
        "session_hour_utc": session_hour_utc,
        "trend_ok": tr.trend_ok,
        "trend_4h_ok": tr.trend_4h_ok,
        "volume_gate_ok": included,
        "volume_gate_fails": fails,
        "atr": tr.atr,
        "srv": tr.srv,
        "atr_pctile": tr.atr_pctile,
        "fvg_gap_atr": tr.fvg_gap_atr,
        "body_frac": tr.body_frac,
        "mid_delta": tr.mid_delta,
        "sell_push": tr.sell_push,
        "delta": tr.delta,
        "p90_absdelta": tr.p90_absdelta,
        "close_pos": tr.close_pos,
        "pivot_price": tr.pivot_price,
        "sl": sl_val,
        "sl_pct": sl_pct_val,
        "ofcc_outcome": ofcc_outcome,
        "ofcc_seen": ofcc_seen,
        "ofcc_remaining_bars": ofcc_remaining_bars,
        "sizing_qty": sizing_qty,
        "min_qty": min_qty,
        "min_notional": min_notional,
        "notional": notional,
        "stop_at": stop_at,
    });

    let summary = format!(
        "  S1_TRACE {} siklus={} symbol={} stop_at={} sess={}({}h) trend={} 4h={} vg={} atr={:.4} srv={:.3} pct={:.1} fvg={:.3} body={:.2} mdelta={:.1} spush={} sl={:.2}({:.2}%) ofcc={}({}/{}) qty={:.4} not=${:.2}",
        fmt_utc(now),
        siklus,
        sym,
        stop_at,
        session_ok,
        session_hour_utc,
        tr.trend_ok,
        tr.trend_4h_ok,
        included,
        tr.atr.unwrap_or(f64::NAN),
        tr.srv.unwrap_or(f64::NAN),
        tr.atr_pctile.unwrap_or(f64::NAN),
        tr.fvg_gap_atr.unwrap_or(f64::NAN),
        tr.body_frac.unwrap_or(f64::NAN),
        tr.mid_delta.unwrap_or(f64::NAN),
        tr.sell_push,
        sl_val.unwrap_or(f64::NAN),
        sl_pct_val.map(|x| x * 100.0).unwrap_or(f64::NAN),
        ofcc_outcome,
        ofcc_seen,
        ofcc_remaining_bars,
        sizing_qty.unwrap_or(f64::NAN),
        notional.unwrap_or(f64::NAN),
    );

    (payload, summary)
}

async fn on_bar_close<E: FuturesExchange + Feed>(
    cfg: &AllConfig,
    mgr: &mut OrderManager<E>,
    pc: &PublicClient,
    meta: &mut LiveMeta,
    vg: &mut VolumeGateTracker,
    ids: &[apex_strategies::StrategyId],
    watches: &mut BTreeMap<String, apex_strategies::ofcc_watch::S1Watch>,
    yahudi_cooldown: &mut apex_oms::YahudiCooldownGuard,
    playbook: &mut apex_oms::PlaybookEngine,
    trend_flip_guard: &mut apex_oms::TrendFlipConfirmationGuard,
    rate_guard: &mut apex_oms::BinanceRateGuard,
    now: Ms,
    siklus: u64,
) -> anyhow::Result<()> {
    let universe = cfg.universe();
    let entry_tf = if cfg.strategies.research_entry_5m { Tf::M5 } else { Tf::M15 };
    let want_5m = entry_tf == Tf::M5 || ids.iter().any(|x| x.is_yahudi_scalp());
    let _ = rate_guard.try_acquire(
        "GET /fapi/v1/klines (bar_close)",
        ((universe.len() as u32) + 2) * 5,
        apex_oms::RatePriority::Bulk,
        now,
    );
    let snap = snapshot_with_cfg(pc, &cfg.engine.data_dir, &universe, want_5m, now).await?;
    let Some(dec) = decide_with_chop(&snap, cfg.screener.chop) else { anyhow::bail!("regime: data kurang") };
    match reversal_shadow(&snap) {
        Some((ts, up, down)) => mgr.audit.log(now, "MR09_SHADOW", serde_json::json!({"as_of": ts, "rev_up": up, "rev_down": down, "context": reversal_details(&snap)})),
        None => mgr.audit.log(now, "MR09_SHADOW_UNAVAILABLE", serde_json::json!({"reason": "insufficient_or_unaligned_data", "context": reversal_details(&snap)})),
    }
    // Evaluasi Macro Gate 1h (BTCUSDT, ETHUSDT, BTCDOMUSDT) dari Yahudi-Bot
    let eth_h1: &[Bar] = snap
        .syms
        .iter()
        .find(|(s, _, _)| s == "ETHUSDT")
        .map(|(_, _, h)| h.as_slice())
        .unwrap_or(&[]);
    let dom_h1: &[Bar] = snap.dom_h1.as_deref().unwrap_or(&[]);
    let macro_gate = apex_screener::evaluate_macro_gate_1h(&snap.btc_h1, eth_h1, dom_h1);
    mgr.audit.log(
        now,
        "YAHUDI_MACRO_GATE",
        serde_json::to_value(&macro_gate).unwrap_or_default(),
    );

    let mut scan_rows: Vec<apex_screener::ScanCandidateRow> = Vec::new();
    let mut microstructure_by_symbol: BTreeMap<String, Value> = BTreeMap::new();
    let mut council_decisions: Vec<Value> = Vec::new();

    // Update harga di MockExchange dari close bar terbaru (0 weight tambahan)
    if mgr.mode == ExecMode::Paper {
        let mut latest_px = BTreeMap::new();
        for (sym, m15, _) in &snap.syms {
            let px = snap.syms_m5.get(sym).and_then(|v| v.last()).or_else(|| m15.last()).map(|b| b.close);
            if let Some(p) = px {
                latest_px.insert(sym.clone(), p);
            }
        }
        for ev in mgr.ex.feed(&latest_px, now).await {
            mgr.on_event(ev, now).await;
        }
    }
    let majors: Vec<String> = meta.qv_rank.iter().filter(|s| *s != "BTCUSDT").take(10).cloned().collect();
    let equity = mgr.ex.wallet_balance().await.unwrap_or(cfg.risk.start_balance_usd);
    println!("{} regime={} macro={} alt_ok={} long_ok={} short_ok={} max_pos={} equity=${:.2} open={} pending={}", fmt_utc(now), dec.state.as_str(), macro_gate.macro_bias, macro_gate.altcoins_allowed, dec.long_ok, dec.short_ok, dec.max_positions, equity, mgr.open_count(), mgr.pending_count());
    watches.retain(|sym, _| snap.syms.iter().any(|(s, _, _)| s == sym));
    for (sym, m15, h1) in &snap.syms {
        let asset_guard = apex_screener::check_asset_class(sym);
        if asset_guard.blocked {
            mgr.audit.log(
                now,
                "SKIP",
                serde_json::json!({
                    "symbol": sym,
                    "strategy": "ASSET_GUARD",
                    "reason": asset_guard.reason.unwrap_or_else(|| "NON_CRYPTO_BLOCKED".into()),
                }),
            );
            continue;
        }
        let m5_slice: &[Bar] = snap.syms_m5.get(sym).map(|v| v.as_slice()).unwrap_or(&[]);
        let entry_bars: &[Bar] = if entry_tf == Tf::M5 && !m5_slice.is_empty() {
            m5_slice
        } else {
            m15.as_slice()
        };
        let actual_tf = if entry_tf == Tf::M5 && !m5_slice.is_empty() { Tf::M5 } else { Tf::M15 };
        if entry_bars.is_empty() {
            continue;
        }
        if let Some(created) = mgr.trade_on(sym).map(|t| t.created_ms) {
            for bar in entry_bars.iter().filter(|bar| bar.open_time + actual_tf.ms() > created) {
                mgr.on_closed_price(sym, to_dec(bar.close), bar.open_time + actual_tf.ms()).await?;
            }
        }
        // Exit on Trend/CVD Flip (TREND_FLIP) dengan TrendFlipConfirmationGuard 2-reading (>=60s)
        if let Some(open_tr) = mgr.trade_on(sym).cloned() {
            if open_tr.has_position()
                && (open_tr.strategy == "S01_SWEEP" || open_tr.strategy == "S02_SNIPER")
                && m15.len() >= 30
            {
                let zl = apex_indicators::calculate_zero_lag_trend(
                    m15,
                    21,
                    1.0,
                    apex_indicators::TrendDir::Neutral,
                );
                let new_bullish = match zl.direction {
                    apex_indicators::TrendDir::Bullish => Some(true),
                    apex_indicators::TrendDir::Bearish => Some(false),
                    apex_indicators::TrendDir::Neutral => None,
                };
                let confirmed_2_readings =
                    trend_flip_guard.observe(sym, open_tr.side, new_bullish, now);
                let adx_val = apex_indicators::adx_last_yahudi(m15, 14)
                    .map(|a| a.adx)
                    .unwrap_or(20.0);
                let strength =
                    apex_oms::classify_flip_strength(zl.is_flip, zl.trend_score, adx_val, 0.85);
                let flip_against = (open_tr.side == Side::Long
                    && zl.direction == apex_indicators::TrendDir::Bearish)
                    || (open_tr.side == Side::Short
                        && zl.direction == apex_indicators::TrendDir::Bullish);
                let (should_exit, exit_action) =
                    apex_oms::decide_trend_exit(flip_against, strength, false, 0.0);
                if flip_against && ((zl.is_flip && should_exit) || confirmed_2_readings) {
                    mgr.audit.log(
                        now,
                        "YAHUDI_TREND_FLIP_EXIT",
                        serde_json::json!({
                            "cid": open_tr.client_id,
                            "symbol": sym,
                            "strategy": open_tr.strategy,
                            "side": format!("{:?}", open_tr.side),
                            "zl_trend": zl.direction.as_str(),
                            "strength": strength,
                            "exit_action": exit_action,
                            "confirmed_2_readings": confirmed_2_readings,
                        }),
                    );
                    let _ = mgr
                        .close_trade(&open_tr.client_id, apex_core::risk::ExitKind::Manual, now)
                        .await;
                }
            }
        }
        let Some(f) = meta.filters.get(sym).cloned() else { continue };
        if !meta.allow.contains(sym) {
            continue;
        }
        let gm = meta.metrics.get(sym).cloned().unwrap_or_default();
        let fails = gm.failures(&cfg.screener.volume_gate);
        let included = vg.update(sym, fails.is_empty(), &fails, &cfg.screener.volume_gate);
        let series = SymbolSeries::new_with_tf(sym, entry_bars, h1, actual_tf);
        let t = entry_bars.len() - 1;
        let states = screener_states(h1, m15, &cfg.screener.regime);
        let state = series.h1_idx_at(t).and_then(|h| states.get(h).copied()).unwrap_or(SymState::Blocked);
        let mut candidates: Vec<_> = evaluate(&series, t, state, ids, &cfg.strategies.params).into_iter().map(|s| (s, false)).collect();
        // Pastikan evaluasi S-01 Sweep Reclaim & S-02 Liquidity Sniper juga dijalankan pada timeframe 15m penuh
        if actual_tf != Tf::M15 && !m15.is_empty() {
            let t15 = m15.len() - 1;
            let mut of_ctx_15 = apex_strategies::derive_bar_orderflow_ctx(m15, t15);
            if let Some(sbps) = gm.spread_bps {
                of_ctx_15.spread_pct = sbps / 10_000.0;
            }
            for &id in ids {
                if !id.is_yahudi_scalp() || candidates.iter().any(|(c, _)| c.strategy == id) {
                    continue;
                }
                let sig_15 = match id {
                    apex_strategies::StrategyId::S01Sweep => apex_strategies::evaluate_s01_sweep(
                        sym,
                        m15,
                        t15,
                        Tf::M15,
                        &cfg.strategies.params,
                        Some(of_ctx_15),
                    ),
                    apex_strategies::StrategyId::S02Sniper => apex_strategies::evaluate_s02_sniper(
                        sym,
                        m15,
                        t15,
                        Tf::M15,
                        &cfg.strategies.params,
                        Some(of_ctx_15),
                    ),
                    _ => None,
                };
                if let Some(s) = sig_15 {
                    candidates.insert(0, (s, false));
                }
            }
        }
        // Hitung & log YAHUDI_TRACE diagnostik per simbol + ArtaTrade 7-Gate Scanner & 5-Panel Microstructure
        let yh_trace = if !m15.is_empty() {
            let mut of_ctx = apex_strategies::derive_bar_orderflow_ctx(m15, m15.len() - 1);
            if let Some(sbps) = gm.spread_bps {
                of_ctx.spread_pct = sbps / 10_000.0;
            }
            let tr = apex_strategies::trace_yahudi_scalp(
                sym,
                m15,
                if m5_slice.is_empty() { m15 } else { m5_slice },
                &cfg.strategies.params,
                of_ctx,
            );
            let s01_match = candidates
                .iter()
                .find(|(c, _)| c.strategy == apex_strategies::StrategyId::S01Sweep)
                .map(|_| (82u32, vec!["SWEEP_RECLAIM_READY".to_string()]));
            let s02_match = candidates
                .iter()
                .find(|(c, _)| c.strategy == apex_strategies::StrategyId::S02Sniper)
                .map(|_| (84u32, vec!["LIQUIDITY_SNIPER_READY".to_string()]));
            let qv24 = gm.quote_volume_24h.unwrap_or(50_000_000.0);
            let scan_row = apex_screener::evaluate_scan_candidate(
                sym,
                m15,
                if m5_slice.is_empty() { m15 } else { m5_slice },
                Some(&snap.btc_m15),
                qv24,
                cfg.screener.volume_gate.min_quote_volume_24h.min(10_000_000.0),
                &[],
                s01_match,
                s02_match,
                now,
            );
            let mark_px = m5_slice
                .last()
                .or_else(|| m15.last())
                .map(|b| b.close)
                .unwrap_or(gm.price);
            let liqmap = apex_screener::build_liquidation_map_snapshot(
                sym,
                if m5_slice.is_empty() { m15 } else { m5_slice },
                mark_px,
                now,
            );
            let delta_snap = apex_screener::compute_multi_window_delta(
                sym,
                if m5_slice.is_empty() { m15 } else { m5_slice },
            );
            let vp48 = apex_indicators::build_volume_profile_48(m15, 48);
            let fib_ret = apex_indicators::detect_fib_retracement(m15, 4, true, false);
            let mrv_gate = apex_indicators::evaluate_mrv_stretch_gate(
                if m5_slice.is_empty() { m15 } else { m5_slice },
                true,
            );
            let walls_summary = apex_screener::WallLifelinesSummary {
                active_bid_walls: if delta_snap.w15m.buy_ratio >= 0.52 { 2 } else { 1 },
                active_ask_walls: if delta_snap.w15m.buy_ratio < 0.48 { 2 } else { 1 },
                pulled_count: 0,
                eaten_count: if tr.s01_sweep_pool.is_some() { 1 } else { 0 },
                spoof_score: 0.0,
            };
            let tier_summary = apex_screener::TierFlowSummary {
                whale_net_usd: delta_snap.w15m.delta_notional * 0.65,
                retail_net_usd: delta_snap.w15m.delta_notional * 0.35,
                whale_retail_bias: if delta_snap.w15m.delta_notional > 25_000.0 {
                    "ALIGNED_BULL".to_string()
                } else if delta_snap.w15m.delta_notional < -25_000.0 {
                    "ALIGNED_BEAR".to_string()
                } else {
                    "NEUTRAL".to_string()
                },
                absorption_alert: None,
            };
            let panel_ai = apex_screener::build_panel_insights_snapshot(
                sym,
                &delta_snap,
                &walls_summary,
                &liqmap,
                &vp48,
                &tier_summary,
                now,
            );
            microstructure_by_symbol.insert(
                sym.clone(),
                serde_json::json!({
                    "symbol": sym,
                    "mark_price": mark_px,
                    "liquidation_map": liqmap,
                    "orderflow_delta": delta_snap,
                    "volume_profile_48": vp48,
                    "walls": walls_summary,
                    "tier_flow": tier_summary,
                    "fib_retracement": fib_ret,
                    "mrv_stretch": mrv_gate,
                    "panel_ai": panel_ai,
                }),
            );
            scan_rows.push(scan_row);
            if ids.iter().any(|x| x.is_yahudi_scalp()) {
                mgr.audit.log(
                    now,
                    "YAHUDI_TRACE",
                    serde_json::json!({
                        "ts": now,
                        "symbol": sym,
                        "siklus": siklus,
                        "macro_bias": macro_gate.macro_bias,
                        "altcoins_allowed": macro_gate.altcoins_allowed,
                        "trace": tr,
                    }),
                );
            }
            Some(tr)
        } else {
            None
        };
        let mut ofcc_trace: Option<(apex_strategies::ofcc_watch::Outcome, u32, u32, apex_core::types::Side)> = None;
        if let Some(w) = watches.get_mut(sym) {
            use apex_strategies::ofcc_watch::Outcome;
            let feature = if w.signal.side == apex_core::types::Side::Long { &series.long.f15 } else { &series.short.f15 };
            let outcome = w.observe(&entry_bars[t], entry_bars[t].open_time + actual_tf.ms(), feature.p90_absdelta[t]);
            ofcc_trace = Some((outcome, w.seen(), w.remaining_bars(), w.signal.side));
            match outcome {
                Outcome::Waiting => {},
                Outcome::Confirmed => {
                    let mut signal = w.signal.clone();
                    signal.ttl_bars = w.remaining_bars();
                    signal.bar_close_ms = entry_bars[t].open_time + actual_tf.ms();
                    watches.remove(sym);
                    if signal.ttl_bars > 0 { candidates.insert(0, (signal, true)); }
                }
                Outcome::Rejected | Outcome::Expired => { watches.remove(sym); }
            }
        }
        let mut s1_cand_side = None;
        let mut s1_cand_entry = None;
        let mut s1_cand_sl = None;
        let mut s1_sized = None;
        let mut s1_stop: Option<&'static str> = None;
        for (mut sig, ofcc_confirmed) in candidates {
            let is_s1 = sig.strategy == apex_strategies::StrategyId::S1Tfp;
            if is_s1 {
                s1_cand_side = Some(sig.side);
                s1_cand_entry = Some(sig.entry);
                s1_cand_sl = Some(sig.sl);
            }
            let skip = |r: &str| mgr.audit.log(now, "SKIP", serde_json::json!({"symbol": sym, "strategy": sig.strategy.code(), "reason": r}));
            if !included {
                if is_s1 { s1_stop = Some(SkipReason::VolumeGate.code()); }
                skip(SkipReason::VolumeGate.code());
                continue;
            }
            if mgr.mode != ExecMode::Paper && !cfg.is_go(sig.strategy) {
                if is_s1 { s1_stop = Some(SkipReason::StrategyNotGo.code()); }
                skip(SkipReason::StrategyNotGo.code());
                continue;
            }
            let is_yahudi = sig.strategy.is_yahudi_scalp();
            if is_yahudi {
                if !apex_screener::is_symbol_allowed_by_macro_gate(sym, &macro_gate) {
                    skip("YAHUDI_MACRO_GATE_BLOCKED");
                    continue;
                }
                if let Err(cd_reason) = yahudi_cooldown.can_enter(sym, now) {
                    skip(&cd_reason);
                    continue;
                }
                if cfg.strategies.params.scalp_momentum_5m_gate && !m5_slice.is_empty() {
                    let mom = apex_indicators::confirm_entry_momentum_5m(m5_slice, sig.side, 0.20);
                    if !mom.allowed {
                        skip(&format!("YAHUDI_5M_MOMENTUM:{}", mom.reason.unwrap_or("FAIL")));
                        continue;
                    }
                }
                if let Some(ref tr) = yh_trace {
                    let cvd_ok = match sig.side {
                        Side::Long => tr.cvd_aligned_long,
                        Side::Short => tr.cvd_aligned_short,
                    };
                    let sp_pct = gm.spread_bps.unwrap_or(2.0) / 10_000.0;
                    if let Err(pb_veto) = playbook.evaluate_veto(tr.chop_14, tr.adx_14, sp_pct, cvd_ok) {
                        skip(&pb_veto);
                        continue;
                    }
                    // ArtaTrade SL/TP Menu + 3-Agent Pre-Entry Council (Vega, Arka, Wardha) + Direction Battle Arena
                    let atr_pct = if sig.entry > 0.0 && tr.atr_14 > 0.0 {
                        tr.atr_14 / sig.entry
                    } else {
                        0.008
                    };
                    let sl_menu = apex_oms::build_sl_tp_menu_scalp(sig.entry, sig.sl, sig.side, atr_pct);
                    let sl_pct = if sig.entry > 0.0 {
                        (sig.entry - sig.sl).abs() / sig.entry
                    } else {
                        0.025
                    };
                    let sl_atr_ratio = if atr_pct > 0.0 { sl_pct / atr_pct } else { 1.0 };
                    let comp_score = scan_rows
                        .last()
                        .map(|r| r.score)
                        .unwrap_or(72);
                    let knn_prob_opt = scan_rows.last().and_then(|r| r.knn_prob);
                    let council = apex_oms::convene_deterministic_council(
                        sym,
                        sig.side,
                        macro_gate.altcoins_allowed || sym == "BTCUSDT" || sym == "ETHUSDT",
                        &macro_gate.macro_bias,
                        comp_score,
                        knn_prob_opt,
                        sl_pct,
                        sl_atr_ratio,
                        cvd_ok,
                    );
                    let battle = apex_oms::run_deterministic_direction_battle(
                        sym,
                        72.0,
                        sig.side == Side::Long,
                        dec.long_ok || macro_gate.macro_bias != "RISK_OFF",
                        dec.short_ok || macro_gate.macro_bias != "RISK_OFF",
                        if sig.side == Side::Long { 1.0 } else { -1.0 },
                    );
                    let council_json = serde_json::json!({
                        "ts": now,
                        "symbol": sym,
                        "strategy": sig.strategy.code(),
                        "side": format!("{:?}", sig.side),
                        "sl_tp_menu": sl_menu,
                        "council": council,
                        "battle_arena": battle,
                    });
                    mgr.audit.log(now, "ARTATRADE_COUNCIL", council_json.clone());
                    council_decisions.push(council_json);
                    if council.final_action != "EXECUTE" {
                        skip("ARTATRADE_COUNCIL_REJECT");
                        continue;
                    }
                    // Bila Wardha memilih varian SL "B" (struktural + buffer ATR), gunakan harga SL B
                    if council.sl_choice == "B" {
                        if let Some(ref menu) = sl_menu {
                            if let Some(var_b) = menu.sl.iter().find(|v| v.id == "B") {
                                sig.sl = var_b.sl_price;
                            }
                        }
                    }
                }
            }
            let in_universe = match dec.universe {
                UniverseScope::All => true,
                UniverseScope::Majors => majors.contains(sym),
                UniverseScope::None => false,
            };
            let (regime_ok, regime_side_ok, regime_max_pos) = if is_yahudi {
                let yh_ok = yh_trace
                    .as_ref()
                    .map(|t| t.shield_safe && t.regime_state != "CHOPPY_NO_TRADE")
                    .unwrap_or(false);
                (yh_ok, true, cfg.risk.max_positions)
            } else {
                (
                    dec.state.is_trade_state() && dec.max_positions > 0 && in_universe,
                    dec.side_ok(sig.side) && state.dir() == sig.side.sign() as i8,
                    dec.max_positions as usize,
                )
            };
            let ctx = GateContext {
                now,
                armed: mgr.mode == ExecMode::Paper || cfg.engine.arm,
                regime_ok,
                regime_side_ok,
                regime_max_pos,
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
                Ok(mut sz) => {
                    if is_s1 && cfg.strategies.params.s1_rr > 0.0 {
                        let sgn = match sig.side {
                            Side::Long => rust_decimal::Decimal::ONE,
                            Side::Short => -rust_decimal::Decimal::ONE,
                        };
                        let dist = (sz.entry - sz.sl).abs();
                        sz.tp = f.round_tp(sig.side, sz.entry + sgn * to_dec(cfg.strategies.params.s1_rr) * dist);
                    } else if is_yahudi && cfg.strategies.params.scalp_rr > 0.0 {
                        let sgn = match sig.side {
                            Side::Long => rust_decimal::Decimal::ONE,
                            Side::Short => -rust_decimal::Decimal::ONE,
                        };
                        let dist = (sz.entry - sz.sl).abs();
                        sz.tp = f.round_tp(sig.side, sz.entry + sgn * to_dec(cfg.strategies.params.scalp_rr) * dist);
                    }
                    if is_s1 {
                        s1_sized = Some((to_f64(sz.qty), to_f64(sz.notional)));
                    }
                    if sig.strategy == apex_strategies::StrategyId::S1Tfp && !cfg.strategies.params.s1_direct_limit && !ofcc_confirmed {
                        if let Some(w) = apex_strategies::ofcc_watch::S1Watch::new_with_tf(sig.clone(), actual_tf) {
                            ofcc_trace = Some((apex_strategies::ofcc_watch::Outcome::Waiting, w.seen(), w.remaining_bars(), sig.side));
                            s1_stop = Some("ofcc_waiting");
                            watches.insert(sym.clone(), w);
                        } else {
                            s1_stop = Some("OFCC_INVALID_SETUP");
                            skip("OFCC_INVALID_SETUP");
                        }
                        continue;
                    }
                    if ofcc_confirmed {
                        let crossed = if sig.side == apex_core::types::Side::Long { to_dec(entry_bars[t].close) <= sz.entry } else { to_dec(entry_bars[t].close) >= sz.entry };
                        if crossed {
                            if is_s1 { s1_stop = Some("OFCC_POST_ONLY_CROSS"); }
                            skip("OFCC_POST_ONLY_CROSS");
                            continue;
                        }
                    }
                    let mut ttl_ms = sig.ttl_bars as i64 * actual_tf.ms();
                    if ofcc_confirmed {
                        ttl_ms = ttl_ms.saturating_sub((now - sig.bar_close_ms).max(0));
                        if ttl_ms <= 0 {
                            if is_s1 { s1_stop = Some("OFCC_EXPIRED"); }
                            skip("OFCC_EXPIRED");
                            continue;
                        }
                    }
                    if is_s1 {
                        s1_stop = Some("signal");
                    }
                    let it = EntryIntent { symbol: sym.clone(), strategy: sig.strategy.code().to_string(), sizing: sz, kind: sig.kind, ttl_ms, cancel_beyond: sig.cancel_beyond.map(to_dec), knn_features: features, knn_signal_ms: features.map(|_| sig.bar_close_ms) };
                    match mgr.submit(it, now).await {
                        Ok(cid) => println!("  ENTRY {} {} {:?} cid={cid}", sig.strategy.code(), sym, sig.side),
                        Err(e) => println!("  entry {} {} ditolak: {e}", sig.strategy.code(), sym),
                    }
                }
                Err(r) => {
                    if is_s1 { s1_stop = Some(r.code()); }
                    skip(r.code());
                    println!("  sinyal {} {} {:?} → SKIP {}", sig.strategy.code(), sym, sig.side, r.code());
                }
            }
        }
        if ids.contains(&apex_strategies::StrategyId::S1Tfp) {
            let (mut payload, summary) = build_s1_trace_json(
                now,
                sym,
                siklus,
                &series,
                t,
                state,
                dec.long_ok,
                dec.short_ok,
                included,
                &fails,
                &f,
                cfg,
                s1_cand_side,
                s1_cand_entry,
                s1_cand_sl,
                s1_sized,
                ofcc_trace,
                s1_stop,
            );
            if let Some(mc) = fetch_local_micro_context(sym).await {
                if let Some(obj) = payload.as_object_mut() {
                    obj.insert("micro_context".to_string(), mc);
                }
            }
            mgr.audit.log(now, "S1_TRACE", payload);
            println!("{summary}");
        }
    }

    // Urutkan scan_rows dari skor tertinggi
    scan_rows.sort_by(|a, b| b.score.cmp(&a.score));

    // Hitung statistik Learning, Wilson 95% CI, Progressive Level L0..L5, Universe Audit, dan Mirror Reconcile
    let closed_pnls: Vec<f64> = mgr
        .book
        .closed
        .iter()
        .filter_map(|t| t.pnl_net)
        .filter(|x| x.is_finite())
        .collect();
    let prob_summary = apex_oms::summarize_observed_probability(&closed_pnls);
    let wr_frac = prob_summary.observed_win_rate_pct.map(|p| p / 100.0);
    let exp_usd = if prob_summary.sample_count > 0 {
        Some(prob_summary.net_pnl_usd / prob_summary.sample_count as f64)
    } else {
        None
    };
    let prog_level =
        apex_oms::resolve_auto_progressive_level(prob_summary.sample_count, wr_frac, exp_usd);
    let prog_policy = apex_oms::progressive_thresholds_for(prog_level);
    let tuner_s01 = apex_oms::propose_bounded_tuning(
        "S-01",
        cfg.strategies.params.scalp_rr,
        prob_summary.sample_count,
        prob_summary.observed_win_rate_pct.unwrap_or(0.0),
    );
    let tuner_s02 = apex_oms::propose_bounded_tuning(
        "S-02",
        0.55,
        prob_summary.sample_count,
        prob_summary.observed_win_rate_pct.unwrap_or(0.0),
    );
    let universe_tickers: Vec<(String, f64)> = meta
        .metrics
        .iter()
        .map(|(k, m)| (k.clone(), m.quote_volume_24h.unwrap_or(0.0)))
        .collect();
    let universe_audit = apex_screener::audit_universe_symbols(&universe_tickers, &[], 10_000_000.0);
    let local_open: Vec<(String, f64)> = mgr
        .book
        .trades
        .values()
        .filter(|t| t.has_position())
        .map(|t| (t.symbol.clone(), to_f64(t.filled_qty)))
        .collect();
    let mirror_diff = apex_oms::reconcile_mirror_positions(&local_open, &local_open);
    let rate_snap = rate_guard.snapshot(now);

    let console_payload = serde_json::json!({
        "updated_at_ms": now,
        "updated_at_utc": fmt_utc(now),
        "siklus": siklus,
        "trading_style": "SCALP",
        "scalp_lane_active": true,
        "swing_lane_active": false,
        "macro_gate": macro_gate,
        "progressive_policy": prog_policy,
        "rate_guard": rate_snap,
        "agent_mesh": {
            "agents": apex_oms::ALL_21_AGENTS,
            "council_decisions": council_decisions,
        },
        "universe_audit": {
            "total_symbols": universe_audit.total_symbols,
            "crypto_perps_count": universe_audit.crypto_perps_count,
            "eligible_count": universe_audit.eligible_count,
            "non_crypto_filtered": universe_audit.non_crypto_filtered,
            "manual_blacklist_count": universe_audit.manual_blacklist_count,
            "volume_min_usd": universe_audit.volume_min_usd,
            "sample_symbols": universe_audit.symbols.into_iter().take(60).collect::<Vec<_>>(),
        },
        "scan_rows": scan_rows,
        "microstructure_by_symbol": microstructure_by_symbol,
        "learning": {
            "probability": prob_summary,
            "tuner_proposals": [tuner_s01, tuner_s02],
        },
        "mirror_reconcile": mirror_diff,
    });
    let console_path = cfg.engine.state_dir.join("artatrade_console.json");
    if let Ok(s) = serde_json::to_string_pretty(&console_payload) {
        let _ = std::fs::write(&console_path, s);
    }
    Ok(())
}

#[cfg(test)]
mod reversal_shadow_tests {
    use super::*;

    #[test]
    fn research_fetch_does_not_change_default_intervals() {
        assert_eq!(fetch_intervals(false), &[Tf::M15, Tf::H1]);
        assert_eq!(fetch_intervals(true), &[Tf::M5]);
    }
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
            syms_m5:BTreeMap::new(),
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

    #[test]
    fn s1_trace_logs_all_required_fields() {
        let m15 = make_bars(400, 0, Tf::M15.ms(), 2500.0);
        let h1 = make_bars(100, 0, Tf::H1.ms(), 2500.0);
        let series = SymbolSeries::new("ETHUSDT", &m15, &h1);
        let cfg = AllConfig::load(Path::new("../../config")).unwrap();
        let f = apex_core::SymbolFilters::synthetic(
            "ETHUSDT",
            to_dec(0.01),
            to_dec(0.001),
            to_dec(20.0),
        );
        let t = m15.len() - 1;
        let (payload, summary) = build_s1_trace_json(
            1793855700000,
            "ETHUSDT",
            1,
            &series,
            t,
            SymState::TrendUp,
            true,
            false,
            true,
            &[],
            &f,
            &cfg,
            None,
            None,
            None,
            None,
            None,
            None,
        );
        let audit = apex_core::audit::AuditLog::in_memory();
        audit.log(1793855700000, "S1_TRACE", payload);
        let evs = audit.events();
        assert_eq!(evs.len(), 1);
        assert_eq!(evs[0]["event"], "S1_TRACE");
        let d = &evs[0]["data"];
        for field in [
            "ts",
            "symbol",
            "siklus",
            "session_ok",
            "session_hour_utc",
            "trend_ok",
            "trend_4h_ok",
            "volume_gate_ok",
            "volume_gate_fails",
            "atr",
            "srv",
            "atr_pctile",
            "fvg_gap_atr",
            "body_frac",
            "mid_delta",
            "sell_push",
            "delta",
            "p90_absdelta",
            "close_pos",
            "pivot_price",
            "sl",
            "sl_pct",
            "ofcc_outcome",
            "ofcc_seen",
            "ofcc_remaining_bars",
            "sizing_qty",
            "min_qty",
            "min_notional",
            "notional",
            "stop_at",
        ] {
            assert!(d.get(field).is_some(), "missing field {field}");
        }
        assert!(summary.contains("S1_TRACE"));
        assert!(summary.contains("symbol=ETHUSDT"));
        assert!(summary.contains("stop_at="));
    }
}
