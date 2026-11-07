//! Uji asap siklus hidup order pada lingkungan Demo Binance (`apex demo-smoke`, WP-9c).
//!
//! Keamanan keras (H1/H3):
//! - Wajib melewati [`crate::safety::authorize`] dan [`crate::safety::check_base`] **tanpa**
//!   memodifikasi gerbang keselamatan yang sudah ada.
//! - Menolak `Mode::Paper`, menolak `Mode::Live`, menolak `arm = false`, dan menolak host apa pun
//!   selain `demo-fapi.binance.com` / `demo-fstream.binance.com`.
//! - Memasang order `LIMIT GTX` (post-only) pada jarak >= 5% di bawah mark price dengan notional
//!   minimum yang lolos `PERCENT_PRICE` serta `MIN_NOTIONAL` sehingga tidak pernah berpotensi terisi.
//! - Selalu menjalankan pembersihan (`cleanup_residual`) untuk membatalkan order/algo sisa dan
//!   meratakan posisi sisa bahkan ketika langkah tengah mengalami error.

use crate::exchange::FuturesExchange;
use crate::mock::MockExchange;
use crate::preflight::{run_preflight, PreflightInput, PreflightReport, PreflightTarget};
use crate::preflight_http::PreflightBackend;
use crate::safety::{authorize, check_base, ExecMode, SafetyError, LIVE_CONFIRM_VALUE};
use crate::types::{AlgoKind, AlgoRequest, ExError, OrderKind, OrderRequest, UserEvent};
use apex_core::filters::SymbolFilters;
use apex_core::num::{ceil_to_step, floor_to_step};
use rust_decimal::Decimal;
use std::collections::BTreeMap;
use std::time::Duration;
use tokio::sync::mpsc;

#[derive(Debug, thiserror::Error)]
pub enum DemoSmokeError {
    #[error("ditolak safety gate: {0}")]
    Safety(#[from] SafetyError),
    #[error("demo-smoke hanya diizinkan pada Mode::Testnet (demo), ditemukan {0:?}")]
    ModeNotDemo(ExecMode),
    #[error("demo-smoke menolak host non-demo: {0}")]
    HostNotDemo(String),
    #[error("preflight demo gagal sebelum smoke dimulai")]
    PreflightFailed(Box<PreflightReport>),
    #[error("tidak ada simbol universe yang mendukung jarak >=5% di dalam PERCENT_PRICE")]
    NoEligibleSymbol,
    #[error("jarak harga limit smoke tidak aman (<5% dari mark)")]
    UnsafeDistance,
    #[error("langkah exchange gagal ({step}): {source}")]
    ExchangeStep {
        step: &'static str,
        #[source]
        source: ExError,
    },
    #[error("query_order tidak menemukan order smoke '{0}'")]
    OrderMissingOnQuery(String),
    #[error("order smoke terisi sebagian/penuh secara tak terduga (qty={0})")]
    UnexpectedFill(Decimal),
    #[error("tidak ada event user-stream yang diterima selama siklus smoke")]
    NoUserStreamEvents,
}

#[derive(Clone, Debug, Default)]
pub struct CleanupSummary {
    pub canceled_orders: usize,
    pub canceled_algos: usize,
    pub flattened_positions: usize,
    pub errors: Vec<String>,
}

#[derive(Clone, Debug)]
pub struct DemoSmokeReport {
    pub symbol: String,
    pub mark_price: Decimal,
    pub limit_price: Decimal,
    pub distance_pct: Decimal,
    pub qty: Decimal,
    pub notional: Decimal,
    pub sl_trigger: Decimal,
    pub order_client_id: String,
    pub algo_id: i64,
    pub user_stream_lines: Vec<String>,
    pub cleanup: CleanupSummary,
}

#[allow(async_fn_in_trait)]
pub trait SmokeEventSource {
    async fn collect_events(&mut self) -> Vec<UserEvent>;
}

pub struct MockEventSource<'a>(pub &'a MockExchange);

impl<'a> SmokeEventSource for MockEventSource<'a> {
    async fn collect_events(&mut self) -> Vec<UserEvent> {
        self.0.drain_events()
    }
}

pub struct ChannelEventSource {
    pub rx: mpsc::UnboundedReceiver<UserEvent>,
    pub wait_ms: u64,
}

impl SmokeEventSource for ChannelEventSource {
    async fn collect_events(&mut self) -> Vec<UserEvent> {
        let mut out = Vec::new();
        let deadline = tokio::time::Instant::now() + Duration::from_millis(self.wait_ms);
        while let Ok(Some(ev)) = tokio::time::timeout_at(deadline, self.rx.recv()).await {
            out.push(ev);
            while let Ok(extra) = self.rx.try_recv() {
                out.push(extra);
            }
            if !out.is_empty() {
                break;
            }
        }
        out
    }
}

#[derive(Clone, Debug)]
struct SmokePlan {
    symbol: String,
    mark: Decimal,
    limit_price: Decimal,
    distance_pct: Decimal,
    qty: Decimal,
    notional: Decimal,
    sl_trigger: Decimal,
}

fn verify_demo_gates(inp: &PreflightInput) -> Result<ExecMode, DemoSmokeError> {
    let lc = inp.env.live_confirm_matches.then_some(LIVE_CONFIRM_VALUE);
    let mode = authorize(&inp.engine, lc)?;
    if mode != ExecMode::Testnet {
        return Err(DemoSmokeError::ModeNotDemo(mode));
    }
    check_base(mode, &inp.engine.testnet_rest_base)?;
    inp.engine
        .validate_ws_urls()
        .map_err(DemoSmokeError::HostNotDemo)?;
    let rest_l = inp.engine.testnet_rest_base.trim().to_ascii_lowercase();
    let ws_l = inp.engine.testnet_ws_base.trim().to_ascii_lowercase();
    if !rest_l.contains("demo-fapi.binance.com")
        || rest_l.contains("binancefuture.com")
        || !ws_l.contains("demo-fstream.binance.com")
    {
        return Err(DemoSmokeError::HostNotDemo(format!(
            "rest='{}' ws='{}' (wajib demo-fapi.binance.com & demo-fstream.binance.com)",
            inp.engine.testnet_rest_base, inp.engine.testnet_ws_base
        )));
    }
    Ok(mode)
}

fn build_smoke_plan(
    universe: &[String],
    filters: &BTreeMap<String, SymbolFilters>,
    marks: &BTreeMap<String, Decimal>,
) -> Result<SmokePlan, DemoSmokeError> {
    let mut candidates: Vec<(&SymbolFilters, Decimal)> = universe
        .iter()
        .filter_map(|s| Some((filters.get(s)?, *marks.get(s)?)))
        .filter(|(f, m)| *m > Decimal::ZERO && f.percent_down <= Decimal::new(94, 2))
        .collect();
    // Pilih simbol dengan min_notional terkecil agar eksposur notional uji sekecil mungkin.
    candidates.sort_by_key(|(f, _)| f.min_notional);
    let (flt, mark) = candidates
        .into_iter()
        .next()
        .ok_or(DemoSmokeError::NoEligibleSymbol)?;
    // 6% di bawah mark price -> jarak >= 5% dan tetap lolos PERCENT_PRICE (percent_down <= 0.94).
    let limit_price = floor_to_step(mark * Decimal::new(94, 2), flt.tick_size);
    if limit_price <= Decimal::ZERO || !flt.percent_price_ok(limit_price, mark) {
        return Err(DemoSmokeError::NoEligibleSymbol);
    }
    let dist_pct = (mark - limit_price) / mark;
    if dist_pct < Decimal::new(5, 2) {
        return Err(DemoSmokeError::UnsafeDistance);
    }
    let target_notional = (flt.min_notional * Decimal::new(105, 2)).max(Decimal::from(6));
    let qty =
        ceil_to_step(target_notional / limit_price, flt.sizing_step()).max(flt.sizing_min_qty());
    let notional = qty * limit_price;
    let sl_trigger = floor_to_step(mark * Decimal::new(80, 2), flt.tick_size);
    Ok(SmokePlan {
        symbol: flt.symbol.clone(),
        mark,
        limit_price,
        distance_pct: dist_pct,
        qty,
        notional,
        sl_trigger,
    })
}

/// Bersihkan sisa order terbuka, algo terbuka, dan ratakan posisi sisa pada simbol smoke.
pub async fn cleanup_residual<E: FuturesExchange>(
    exchange: &E,
    symbol: &str,
    tag: &str,
) -> CleanupSummary {
    let mut sum = CleanupSummary::default();
    match exchange.open_orders().await {
        Ok(orders) => {
            for o in orders.into_iter().filter(|o| o.symbol == symbol) {
                match exchange.cancel_order(symbol, &o.client_id).await {
                    Ok(()) => sum.canceled_orders += 1,
                    Err(e) => sum
                        .errors
                        .push(format!("cancel_order {}: {e}", o.client_id)),
                }
            }
        }
        Err(e) => sum.errors.push(format!("open_orders: {e}")),
    }
    match exchange.open_algo_orders().await {
        Ok(algos) => {
            for a in algos.into_iter().filter(|a| a.symbol == symbol) {
                match exchange.cancel_algo(symbol, a.algo_id).await {
                    Ok(()) => sum.canceled_algos += 1,
                    Err(e) => sum.errors.push(format!("cancel_algo {}: {e}", a.algo_id)),
                }
            }
        }
        Err(e) => sum.errors.push(format!("open_algo_orders: {e}")),
    }
    match exchange.positions().await {
        Ok(positions) => {
            for (idx, p) in positions
                .into_iter()
                .filter(|p| p.symbol == symbol && !p.amt.is_zero())
                .enumerate()
            {
                let side = if p.amt > Decimal::ZERO { "SELL" } else { "BUY" };
                let req = OrderRequest {
                    symbol: symbol.to_string(),
                    side: side.into(),
                    kind: OrderKind::Market,
                    qty: p.amt.abs(),
                    reduce_only: true,
                    client_id: format!("apxSMKFLT-{tag}-{idx}"),
                };
                match exchange.place_order(&req).await {
                    Ok(_) => sum.flattened_positions += 1,
                    Err(e) => sum.errors.push(format!("flatten {symbol}: {e}")),
                }
            }
        }
        Err(e) => sum.errors.push(format!("positions: {e}")),
    }
    sum
}

async fn run_smoke_steps<E: FuturesExchange, S: SmokeEventSource>(
    exchange: &E,
    events: &mut S,
    plan: &SmokePlan,
    tag: &str,
) -> Result<(String, i64, Vec<String>), DemoSmokeError> {
    let cid = format!("apxSMK-{tag}");
    let acid = format!("apxSMKA-{tag}");
    let order_req = OrderRequest {
        symbol: plan.symbol.clone(),
        side: "BUY".into(),
        kind: OrderKind::LimitGtx {
            price: plan.limit_price,
        },
        qty: plan.qty,
        reduce_only: false,
        client_id: cid.clone(),
    };
    let ack =
        exchange
            .place_order(&order_req)
            .await
            .map_err(|source| DemoSmokeError::ExchangeStep {
                step: "place_order",
                source,
            })?;
    if !ack.executed_qty.is_zero() {
        return Err(DemoSmokeError::UnexpectedFill(ack.executed_qty));
    }
    let queried = exchange
        .query_order(&plan.symbol, &cid)
        .await
        .map_err(|source| DemoSmokeError::ExchangeStep {
            step: "query_order",
            source,
        })?
        .ok_or_else(|| DemoSmokeError::OrderMissingOnQuery(cid.clone()))?;
    if !queried.executed_qty.is_zero() {
        return Err(DemoSmokeError::UnexpectedFill(queried.executed_qty));
    }
    exchange
        .cancel_order(&plan.symbol, &cid)
        .await
        .map_err(|source| DemoSmokeError::ExchangeStep {
            step: "cancel_order",
            source,
        })?;
    let algo_req = AlgoRequest {
        symbol: plan.symbol.clone(),
        side: "SELL".into(),
        kind: AlgoKind::StopMarket,
        trigger: plan.sl_trigger,
        close_position: false,
        qty: Some(plan.qty),
        client_algo_id: acid,
    };
    let algo_ack =
        exchange
            .place_algo(&algo_req)
            .await
            .map_err(|source| DemoSmokeError::ExchangeStep {
                step: "place_algo",
                source,
            })?;
    exchange
        .cancel_algo(&plan.symbol, algo_ack.algo_id)
        .await
        .map_err(|source| DemoSmokeError::ExchangeStep {
            step: "cancel_algo",
            source,
        })?;
    let evs = events.collect_events().await;
    if evs.is_empty() {
        return Err(DemoSmokeError::NoUserStreamEvents);
    }
    let lines = evs.iter().map(format_user_event_line).collect();
    Ok((cid, algo_ack.algo_id, lines))
}

fn format_user_event_line(ev: &UserEvent) -> String {
    match ev {
        UserEvent::Order(o) => format!(
            "USER_STREAM ORDER symbol={} cid={} side={} type={} status={:?} exec={} cum={}",
            o.symbol, o.client_id, o.side, o.orig_type, o.status, o.exec_type, o.cum_qty
        ),
        UserEvent::Algo {
            symbol,
            algo_id,
            client_algo_id,
            status,
            ..
        } => format!(
            "USER_STREAM ALGO symbol={symbol} algo_id={algo_id} caid={client_algo_id} status={status}"
        ),
        UserEvent::Account { positions, ts } => {
            format!("USER_STREAM ACCOUNT positions={} ts={ts}", positions.len())
        }
        UserEvent::ListenKeyExpired => "USER_STREAM LISTEN_KEY_EXPIRED".into(),
    }
}

pub async fn run_demo_smoke<B, E, S>(
    inp: &PreflightInput,
    preflight_backend: &B,
    exchange: &E,
    events: &mut S,
    run_tag: &str,
) -> Result<DemoSmokeReport, DemoSmokeError>
where
    B: PreflightBackend,
    E: FuturesExchange,
    S: SmokeEventSource,
{
    let _mode = verify_demo_gates(inp)?;
    let mut demo_inp = inp.clone();
    demo_inp.target = PreflightTarget::Demo;
    let preflight = run_preflight(&demo_inp, preflight_backend).await;
    if preflight.has_failure() {
        return Err(DemoSmokeError::PreflightFailed(Box::new(preflight)));
    }
    let (filters, marks) = preflight_backend
        .exchange_filters_and_marks()
        .await
        .map_err(|msg| DemoSmokeError::ExchangeStep {
            step: "exchange_filters_and_marks",
            source: ExError::Network(msg),
        })?;
    let plan = build_smoke_plan(&demo_inp.universe, &filters, &marks)?;
    let tag: String = run_tag
        .chars()
        .filter(|c| c.is_ascii_alphanumeric())
        .take(12)
        .collect();
    let safe_tag = if tag.is_empty() { "1".to_string() } else { tag };
    let step_res = run_smoke_steps(exchange, events, &plan, &safe_tag).await;
    let cleanup = cleanup_residual(exchange, &plan.symbol, &safe_tag).await;
    let (cid, algo_id, lines) = step_res?;
    Ok(DemoSmokeReport {
        symbol: plan.symbol,
        mark_price: plan.mark,
        limit_price: plan.limit_price,
        distance_pct: plan.distance_pct,
        qty: plan.qty,
        notional: plan.notional,
        sl_trigger: plan.sl_trigger,
        order_client_id: cid,
        algo_id,
        user_stream_lines: lines,
        cleanup,
    })
}
