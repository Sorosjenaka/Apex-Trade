//! OrderManager: state machine entry → proteksi → exit, reconcile, kill switch, persist, audit.
//!
//! Siklus trade: `PendingEntry` / `UnknownSubmit` → (fill) → `Open` (SL+TP algo terpasang) → `Closing` → `Closed`.
//! `Unprotected` = posisi ada tapi SL gagal dipasang DAN flatten gagal → kill switch aktif,
//! posisi TETAP di buku dan dicoba ulang setiap `tick()`.
use crate::exchange::FuturesExchange;
use crate::killswitch::KillSwitch;
use crate::notifier::{NoopNotifier, Notifier};
use crate::safety::ExecMode;
use crate::store::{StateStore, StoreError};
use crate::types::*;
use apex_core::audit::AuditLog;
use apex_core::config::{EngineConfig, OrphanPolicy, RiskConfig};
use apex_core::filters::SymbolFilters;
use apex_core::num::{fmt_dec, to_dec, to_f64};
use apex_core::risk::{ExitKind, RiskState};
use apex_core::sizing::{EntryKind, Sizing};
use apex_core::types::{Ms, Side};
use rust_decimal::Decimal;
use serde::{Deserialize, Serialize};
use serde_json::json;
use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;

/// Jadwal backoff kumulatif (ms sejak `unknown_since_ms`) untuk re-query `UnknownSubmit`:
/// langkah jeda `[1s, 2s, 4s, 8s, 15s]` → kumulatif `[1s, 3s, 7s, 15s, 30s]`.
pub const UNKNOWN_CUM_BACKOFF_MS: [i64; 5] = [1_000, 3_000, 7_000, 15_000, 30_000];

/// Jadwal backoff antar percobaan pemasangan SL algo (ms) sesuai GR-8.
pub const SL_RETRY_BACKOFF_MS: [u64; 4] = [100, 250, 500, 1_000];

fn default_true() -> bool {
    true
}

pub fn strategy_has_backtest(code: &str) -> bool {
    let u = code.trim().to_ascii_uppercase();
    !matches!(
        u.as_str(),
        "S01" | "S-01" | "S01_SWEEP" | "SWEEP" | "S02" | "S-02" | "S02_SNIPER" | "SNIPER"
    )
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum TradeState {
    PendingEntry,
    UnknownSubmit,
    Open,
    Unprotected,
    Closing,
    Closed,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ManagedTrade {
    pub client_id: String,
    pub symbol: String,
    pub strategy: String,
    pub side: Side,
    pub entry_kind: EntryKind,
    pub planned_qty: Decimal,
    pub filled_qty: Decimal,
    pub entry_price: Decimal,
    pub avg_fill: Decimal,
    pub sl: Decimal,
    pub tp: Decimal,
    pub planned_risk: Decimal,
    pub state: TradeState,
    pub sl_algo: Option<i64>,
    pub tp_algo: Option<i64>,
    pub created_ms: Ms,
    pub expires_ms: Ms,
    #[serde(default)]
    pub cancel_beyond: Option<Decimal>,
    #[serde(default)]
    pub entry_final: bool,
    #[serde(default)]
    pub knn_features: Option<[f64; 5]>,
    #[serde(default)]
    pub knn_signal_ms: Option<Ms>,
    #[serde(default)]
    pub unknown_since_ms: Option<Ms>,
    #[serde(default)]
    pub unknown_attempts: u8,
    #[serde(default)]
    pub fill_confirmed_at: Option<Ms>,
    #[serde(default = "default_true")]
    pub validated: bool,
    pub filled_ms: Option<Ms>,
    pub closed_ms: Option<Ms>,
    pub exit_price: Option<Decimal>,
    pub exit_kind: Option<ExitKind>,
    pub exit_hint: Option<ExitKind>,
    pub fees: Decimal,
    pub pnl_net: Option<f64>,
    pub close_attempts: u32,
    pub protect_attempts: u32,
    pub flat_seen_ms: Option<Ms>,
    pub last_error: Option<String>,
    pub adopted: bool,
}

impl ManagedTrade {
    pub fn is_active(&self) -> bool {
        self.state != TradeState::Closed
    }
    pub fn has_position(&self) -> bool {
        matches!(
            self.state,
            TradeState::Open | TradeState::Unprotected | TradeState::Closing
        )
    }
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct Book {
    pub trades: BTreeMap<String, ManagedTrade>,
    pub closed: Vec<ManagedTrade>,
    pub risk: RiskState,
    pub seq: u64,
    pub prepared: BTreeSet<String>,
    pub one_way_ok: bool,
}

#[derive(Clone, Debug)]
pub struct EntryIntent {
    pub symbol: String,
    pub strategy: String,
    pub sizing: Sizing,
    pub kind: EntryKind,
    pub ttl_ms: i64,
    pub cancel_beyond: Option<Decimal>,
    pub knn_features: Option<[f64; 5]>,
    pub knn_signal_ms: Option<Ms>,
}

#[derive(Debug, thiserror::Error)]
pub enum OmsError {
    #[error("kill switch aktif")]
    Killed,
    #[error("ditolak safety: {0}")]
    Safety(String),
    #[error("simbol {0} sedang dipakai")]
    SymbolBusy(String),
    #[error("exchange: {0}")]
    Exchange(#[from] ExError),
    #[error("state store: {0}")]
    Store(#[from] StoreError),
    #[error("{0}")]
    Other(String),
}

pub struct OrderManager<E: FuturesExchange> {
    pub ex: E,
    pub mode: ExecMode,
    armed: bool,
    pub cfg: EngineConfig,
    pub risk_cfg: RiskConfig,
    pub book: Book,
    pub store: StateStore,
    pub audit: AuditLog,
    pub kill: KillSwitch,
    pub notifier: Arc<dyn Notifier>,
    filters: BTreeMap<String, SymbolFilters>,
    /// jeda backoff rate-limit maksimum (ms)
    pub max_backoff_ms: u64,
    /// penghitung kegagalan simpan berturut-turut pada transisi pasca-entry (>=3 memicu kill switch)
    pub consecutive_save_failures: u32,
}

macro_rules! rl {
    ($self:ident, $call:expr) => {{
        let mut n = 0u32;
        loop {
            match $call.await {
                Err(ExError::RateLimited {
                    retry_after_ms,
                    banned: false,
                }) if n < 4 => {
                    n += 1;
                    let wait = retry_after_ms.max(50 * (1 << n)).min($self.max_backoff_ms);
                    $self.audit.log(
                        apex_core::time::now_ms(),
                        "RATE_LIMIT_BACKOFF",
                        json!({"wait_ms": wait, "attempt": n}),
                    );
                    tokio::time::sleep(std::time::Duration::from_millis(wait)).await;
                }
                r => break r,
            }
        }
    }};
}

fn side_str(s: Side) -> &'static str {
    s.entry_order_side()
}
fn exit_str(s: Side) -> &'static str {
    s.exit_order_side()
}

impl<E: FuturesExchange> OrderManager<E> {
    /// `armed` = cfg.arm. Paper selalu boleh (simulasi); testnet/live wajib armed (dicek lagi di sini).
    pub fn new(
        ex: E,
        mode: ExecMode,
        cfg: EngineConfig,
        risk_cfg: RiskConfig,
        store: StateStore,
        audit: AuditLog,
        filters: BTreeMap<String, SymbolFilters>,
    ) -> anyhow::Result<Self> {
        if mode != ExecMode::Paper && !cfg.arm {
            anyhow::bail!("ARM=false: OrderManager non-paper ditolak");
        }
        if mode == ExecMode::Paper && ex.name() != "mock" {
            anyhow::bail!("mode paper hanya boleh memakai MockExchange (tanpa panggilan exchange)");
        }
        let loaded = store.load_with_recovery::<Book>()?;
        if loaded.recovered_from_bak {
            let p = store
                .path()
                .map(|x| x.display().to_string())
                .unwrap_or_default();
            audit.log(
                apex_core::time::now_ms(),
                "STATE_RECOVERED_FROM_BAK",
                json!({"path": p}),
            );
        }
        let book = loaded.value;
        let kill = KillSwitch::new(&cfg.kill_file);
        Ok(OrderManager {
            armed: cfg.arm,
            ex,
            mode,
            cfg,
            risk_cfg,
            book,
            store,
            audit,
            kill,
            notifier: Arc::new(NoopNotifier),
            filters,
            max_backoff_ms: 30_000,
            consecutive_save_failures: 0,
        })
    }

    pub fn with_notifier(mut self, notifier: Arc<dyn Notifier>) -> Self {
        self.notifier = notifier;
        self
    }

    pub fn set_notifier(&mut self, notifier: Arc<dyn Notifier>) {
        self.notifier = notifier;
    }

    pub fn save(&mut self) -> Result<(), StoreError> {
        match self.store.save(&self.book) {
            Ok(()) => {
                self.consecutive_save_failures = 0;
                Ok(())
            }
            Err(e) => {
                self.consecutive_save_failures = self.consecutive_save_failures.saturating_add(1);
                let ts = apex_core::time::now_ms();
                let msg = format!(
                    "gagal menyimpan state OMS (berturut-turut={}): {e}",
                    self.consecutive_save_failures
                );
                self.audit.log(
                    ts,
                    "STATE_SAVE_ERROR",
                    json!({"err": e.to_string(), "consecutive_failures": self.consecutive_save_failures}),
                );
                self.notifier
                    .notify("CRITICAL", "STATE_SAVE_ERROR", &msg, ts);
                Err(e)
            }
        }
    }

    async fn save_or_kill(&mut self, now: Ms) {
        if self.save().is_err() && self.consecutive_save_failures >= 3 && !self.kill.is_engaged() {
            Box::pin(self.engage_kill("STATE_SAVE_FAILED", false, now)).await;
        }
    }

    pub fn active(&self) -> impl Iterator<Item = &ManagedTrade> {
        self.book.trades.values().filter(|t| t.is_active())
    }
    pub fn open_count(&self) -> usize {
        self.active().filter(|t| t.has_position()).count()
    }
    /// Menghitung `PendingEntry` dan `UnknownSubmit` secara konservatif terhadap `max_positions`.
    pub fn pending_count(&self) -> usize {
        self.active()
            .filter(|t| {
                matches!(
                    t.state,
                    TradeState::PendingEntry | TradeState::UnknownSubmit
                )
            })
            .count()
    }
    pub fn unknown_count(&self) -> usize {
        self.active()
            .filter(|t| t.state == TradeState::UnknownSubmit)
            .count()
    }
    pub fn symbol_busy(&self, s: &str) -> bool {
        self.active().any(|t| t.symbol == s)
    }
    pub fn trade_on(&self, s: &str) -> Option<&ManagedTrade> {
        self.active().find(|t| t.symbol == s)
    }
    fn cid_on(&self, s: &str) -> Option<String> {
        self.trade_on(s).map(|t| t.client_id.clone())
    }

    fn next_id(&mut self, prefix: &str, sym: &str, now: Ms) -> String {
        self.book.seq += 1;
        make_client_id(prefix, sym, now, self.book.seq)
    }

    /// Boot: one-way mode + reconcile.
    pub async fn boot(&mut self, now: Ms) -> Result<(), OmsError> {
        if !self.book.one_way_ok {
            rl!(self, self.ex.set_one_way())?;
            self.book.one_way_ok = true;
        }
        self.audit.log(
            now,
            "BOOT",
            json!({"mode": format!("{:?}", self.mode), "exchange": self.ex.name(), "armed": self.armed}),
        );
        self.notifier.notify(
            "INFO",
            "BOOT",
            &format!("OMS boot mode={:?} armed={}", self.mode, self.armed),
            now,
        );
        self.reconcile(now).await
    }

    pub fn shutdown(&mut self, now: Ms) {
        self.audit
            .log(now, "SHUTDOWN", json!({"mode": format!("{:?}", self.mode)}));
        self.notifier
            .notify("INFO", "SHUTDOWN", "OMS shutdown bersih", now);
        let _ = self.save();
    }

    /// CROSSED + leverage 25x per simbol (sekali, diingat di state).
    pub async fn prepare_symbol(&mut self, sym: &str) -> Result<(), OmsError> {
        if self.book.prepared.contains(sym) {
            return Ok(());
        }
        rl!(self, self.ex.set_margin_crossed(sym))?;
        let lev = rl!(self, self.ex.set_leverage(sym, self.risk_cfg.leverage))?;
        if lev != self.risk_cfg.leverage {
            return Err(OmsError::Other(format!(
                "leverage {sym} = {lev}, bukan {}",
                self.risk_cfg.leverage
            )));
        }
        self.book.prepared.insert(sym.into());
        self.save_or_kill(apex_core::time::now_ms()).await;
        Ok(())
    }

    pub async fn submit(&mut self, it: EntryIntent, now: Ms) -> Result<String, OmsError> {
        if self.kill.is_engaged() {
            return Err(OmsError::Killed);
        }
        if self.mode != ExecMode::Paper && !self.armed {
            return Err(OmsError::Safety("ARM=false".into()));
        }
        if self.symbol_busy(&it.symbol) {
            return Err(OmsError::SymbolBusy(it.symbol));
        }
        let sz = &it.sizing;
        let cid = self.next_id(it.strategy.get(..2).unwrap_or("XX"), &it.symbol, now);
        let validated = strategy_has_backtest(&it.strategy);
        let t = ManagedTrade {
            client_id: cid.clone(),
            symbol: it.symbol.clone(),
            strategy: it.strategy.clone(),
            side: sz.side,
            entry_kind: it.kind,
            planned_qty: sz.qty,
            filled_qty: Decimal::ZERO,
            entry_price: sz.entry,
            avg_fill: Decimal::ZERO,
            sl: sz.sl,
            tp: sz.tp,
            planned_risk: sz.risk,
            state: TradeState::PendingEntry,
            sl_algo: None,
            tp_algo: None,
            created_ms: now,
            expires_ms: now + it.ttl_ms,
            cancel_beyond: it.cancel_beyond,
            entry_final: false,
            knn_features: it.knn_features,
            knn_signal_ms: it.knn_signal_ms,
            unknown_since_ms: None,
            unknown_attempts: 0,
            fill_confirmed_at: None,
            validated,
            filled_ms: None,
            closed_ms: None,
            exit_price: None,
            exit_kind: None,
            exit_hint: None,
            fees: Decimal::ZERO,
            pnl_net: None,
            close_attempts: 0,
            protect_attempts: 0,
            flat_seen_ms: None,
            last_error: None,
            adopted: false,
        };
        // Persist SEBELUM kirim → bila persist gagal, batalkan di memori tanpa menyentuh bursa.
        self.book.trades.insert(cid.clone(), t);
        if let Err(e) = self.save() {
            self.book.trades.remove(&cid);
            self.audit.log(
                now,
                "ENTRY_ABORTED_NO_PERSIST",
                json!({"cid": cid, "symbol": it.symbol, "err": e.to_string()}),
            );
            return Err(OmsError::Store(e));
        }
        if let Err(e) = self.prepare_symbol(&it.symbol).await {
            self.book.trades.remove(&cid);
            self.save_or_kill(now).await;
            return Err(e);
        }
        let sz = &it.sizing;
        let req = OrderRequest {
            symbol: it.symbol.clone(),
            side: side_str(sz.side).into(),
            kind: match it.kind {
                EntryKind::Market => OrderKind::Market,
                EntryKind::LimitMaker => OrderKind::LimitGtx { price: sz.entry },
            },
            qty: sz.qty,
            reduce_only: false,
            client_id: cid.clone(),
        };
        self.audit.log(
            now,
            "ENTRY_SUBMIT",
            json!({
                "cid": cid,
                "symbol": it.symbol,
                "strategy": it.strategy,
                "validated": validated,
                "side": req.side,
                "kind": format!("{:?}", req.kind),
                "qty": fmt_dec(sz.qty),
                "sl": fmt_dec(sz.sl),
                "tp": fmt_dec(sz.tp),
                "risk": fmt_dec(sz.risk),
            }),
        );
        match rl!(self, self.ex.place_order(&req)) {
            Ok(a) => {
                self.apply_entry_status(
                    &cid,
                    a.status,
                    a.executed_qty,
                    a.avg_price,
                    Decimal::ZERO,
                    a.update_ms.max(now),
                )
                .await;
                Ok(cid)
            }
            Err(ExError::Unknown(m)) => {
                self.audit
                    .log(now, "ENTRY_UNKNOWN", json!({"cid": cid, "msg": m}));
                match rl!(self, self.ex.query_order(&it.symbol, &cid)) {
                    Ok(Some(a)) => {
                        self.apply_entry_status(
                            &cid,
                            a.status,
                            a.executed_qty,
                            a.avg_price,
                            Decimal::ZERO,
                            a.update_ms.max(now),
                        )
                        .await;
                        Ok(cid)
                    }
                    Ok(None) | Err(ExError::Unknown(_)) => {
                        if let Some(tr) = self.book.trades.get_mut(&cid) {
                            tr.state = TradeState::UnknownSubmit;
                            tr.unknown_since_ms = Some(now);
                            tr.unknown_attempts = 0;
                        }
                        self.audit.log(
                            now,
                            "ENTRY_UNKNOWN_QUEUED",
                            json!({"cid": cid, "symbol": it.symbol}),
                        );
                        self.save_or_kill(now).await;
                        Ok(cid)
                    }
                    Err(e) => {
                        self.audit.log(
                            now,
                            "ENTRY_REJECTED",
                            json!({"cid": cid, "err": e.to_string()}),
                        );
                        self.book.trades.remove(&cid);
                        self.save_or_kill(now).await;
                        Err(e.into())
                    }
                }
            }
            Err(e) => {
                self.audit.log(
                    now,
                    "ENTRY_REJECTED",
                    json!({"cid": cid, "err": e.to_string()}),
                );
                self.book.trades.remove(&cid);
                self.save_or_kill(now).await;
                Err(e.into())
            }
        }
    }

    /// Terapkan status entry (dari ACK REST, re-query `UnknownSubmit`, atau event stream). Idempoten via cum qty.
    async fn apply_entry_status(
        &mut self,
        cid: &str,
        status: OrderStatus,
        cum: Decimal,
        avg: Decimal,
        fee: Decimal,
        ts: Ms,
    ) {
        let Some(t) = self.book.trades.get_mut(cid) else {
            return;
        };
        if !t.is_active() {
            return;
        }
        if t.state == TradeState::UnknownSubmit && cum.is_zero() && !status.is_final() {
            t.state = TradeState::PendingEntry;
        }
        t.entry_final |= status.is_final();
        let mut newly_filled = false;
        if cum > t.filled_qty {
            t.filled_qty = cum;
            if !avg.is_zero() {
                t.avg_fill = avg;
            }
            if fee.is_zero() {
                let rate = if t.entry_kind == EntryKind::Market {
                    self.risk_cfg.fee_taker
                } else {
                    self.risk_cfg.fee_maker
                };
                t.fees = t.avg_fill * t.filled_qty * to_dec(rate);
            } else {
                t.fees += fee;
            }
            if matches!(
                t.state,
                TradeState::PendingEntry | TradeState::UnknownSubmit
            ) {
                t.state = TradeState::Open;
                t.filled_ms = Some(ts);
                t.fill_confirmed_at.get_or_insert(ts);
                newly_filled = true;
            }
        }
        let cancelled_empty = matches!(
            status,
            OrderStatus::Canceled | OrderStatus::Expired | OrderStatus::Rejected
        ) && t.filled_qty.is_zero();
        if cancelled_empty {
            if let Some(removed) = self.book.trades.remove(cid) {
                self.audit.log(
                    ts,
                    "ENTRY_CANCELLED",
                    json!({"cid": cid, "symbol": removed.symbol, "status": format!("{status:?}")}),
                );
            }
            self.save_or_kill(ts).await;
            return;
        }
        if newly_filled {
            let (s, q, p) = (t.symbol.clone(), t.filled_qty, t.avg_fill);
            self.audit.log(
                ts,
                "ENTRY_FILLED",
                json!({"cid": cid, "symbol": s, "qty": fmt_dec(q), "avg": fmt_dec(p), "status": format!("{status:?}")}),
            );
        }
        self.save_or_kill(ts).await;
        if self
            .book
            .trades
            .get(cid)
            .is_some_and(|tr| tr.has_position() && tr.sl_algo.is_none())
        {
            self.protect(cid, ts).await;
        }
    }

    /// Kendalikan re-query order berstatus `UnknownSubmit` dengan jadwal backoff `[1s, 2s, 4s, 8s, 15s]`.
    pub async fn tick_unknown(&mut self, now: Ms) {
        let unknowns: Vec<(String, String, Ms, u8)> = self
            .active()
            .filter(|t| t.state == TradeState::UnknownSubmit)
            .map(|t| {
                (
                    t.client_id.clone(),
                    t.symbol.clone(),
                    t.unknown_since_ms.unwrap_or(t.created_ms),
                    t.unknown_attempts,
                )
            })
            .collect();
        for (cid, sym, since, attempts) in unknowns {
            let idx = (attempts as usize).min(UNKNOWN_CUM_BACKOFF_MS.len() - 1);
            if (attempts as usize) < UNKNOWN_CUM_BACKOFF_MS.len()
                && now.saturating_sub(since) < UNKNOWN_CUM_BACKOFF_MS[idx]
            {
                continue;
            }
            let next_attempt = attempts.saturating_add(1);
            if let Some(tr) = self.book.trades.get_mut(&cid) {
                tr.unknown_attempts = next_attempt;
            }
            match rl!(self, self.ex.query_order(&sym, &cid)) {
                Ok(Some(a)) => {
                    self.audit.log(
                        now,
                        "UNKNOWN_RESOLVED_BY_QUERY",
                        json!({"cid": cid, "symbol": sym, "attempt": next_attempt, "status": format!("{:?}", a.status)}),
                    );
                    self.apply_entry_status(
                        &cid,
                        a.status,
                        a.executed_qty,
                        a.avg_price,
                        Decimal::ZERO,
                        a.update_ms.max(now),
                    )
                    .await;
                }
                Ok(None) | Err(_) => {
                    if (next_attempt as usize) >= UNKNOWN_CUM_BACKOFF_MS.len() {
                        self.resolve_unknown_final(&cid, &sym, now).await;
                    } else {
                        self.save_or_kill(now).await;
                    }
                }
            }
        }
    }

    async fn resolve_unknown_final(&mut self, cid: &str, sym: &str, now: Ms) {
        if let Ok(open_ords) = rl!(self, self.ex.open_orders()) {
            if let Some(o) = open_ords
                .into_iter()
                .find(|o| o.client_id == cid || (o.symbol == sym && !o.reduce_only))
            {
                self.apply_entry_status(
                    cid,
                    o.status,
                    o.executed_qty,
                    o.avg_price,
                    Decimal::ZERO,
                    o.update_ms.max(now),
                )
                .await;
                return;
            }
        }
        if let Ok(positions) = rl!(self, self.ex.positions()) {
            if let Some(p) = positions
                .into_iter()
                .find(|p| p.symbol == sym && !p.amt.is_zero())
            {
                self.adopt_unknown_position(cid, &p, now).await;
                return;
            }
        }
        self.book.trades.remove(cid);
        self.audit.log(
            now,
            "ENTRY_NOT_FOUND_FINAL",
            json!({"cid": cid, "symbol": sym, "attempts": UNKNOWN_CUM_BACKOFF_MS.len()}),
        );
        self.save_or_kill(now).await;
    }

    async fn adopt_unknown_position(&mut self, cid: &str, p: &PositionInfo, now: Ms) {
        if let Some(tr) = self.book.trades.get_mut(cid) {
            tr.state = TradeState::Open;
            tr.filled_qty = p.amt.abs();
            if !p.entry_price.is_zero() {
                tr.avg_fill = p.entry_price;
            } else {
                tr.avg_fill = tr.entry_price;
            }
            tr.entry_final = true;
            tr.filled_ms = Some(now);
            tr.fill_confirmed_at.get_or_insert(now);
            tr.adopted = true;
        }
        self.audit.log(
            now,
            "UNKNOWN_ADOPTED_POSITION",
            json!({"cid": cid, "symbol": p.symbol, "amt": fmt_dec(p.amt), "entry": fmt_dec(p.entry_price)}),
        );
        self.save_or_kill(now).await;
        self.protect(cid, now).await;
    }

    fn algo_req(
        &self,
        t: &ManagedTrade,
        kind: AlgoKind,
        trigger: Decimal,
        tag: &str,
    ) -> AlgoRequest {
        let cp = self.cfg.protective_close_position;
        AlgoRequest {
            symbol: t.symbol.clone(),
            side: exit_str(t.side).into(),
            kind,
            trigger,
            close_position: cp,
            qty: if cp { None } else { Some(t.filled_qty) },
            client_algo_id: format!("{tag}{}", &t.client_id[3.min(t.client_id.len())..])
                .chars()
                .take(36)
                .collect(),
        }
    }

    /// Pasang SL (wajib, GR-8: konfirmasi <=2 dtk, >2 dtk audit `SL_MISSING` + notifikasi,
    /// >5 dtk atau retry habis → flatten `reduceOnly` + `engage_kill`) dan TP.
    pub async fn protect(&mut self, cid: &str, now: Ms) {
        let Some(t) = self.book.trades.get(cid).cloned() else {
            return;
        };
        if !t.has_position() || t.state == TradeState::Closing {
            return;
        }
        if t.sl_algo.is_none() {
            let req = self.algo_req(&t, AlgoKind::StopMarket, t.sl, "sl");
            let max_retries = self.cfg.protection_retries.max(1);
            let timeout_ms = self.cfg.protection_timeout_ms.max(100);
            let confirm_base = t.fill_confirmed_at.or(t.filled_ms).unwrap_or(now);
            let started = std::time::Instant::now();
            let mut placed = None;
            let mut would_trigger = false;
            let mut last = String::new();
            for attempt in 0..max_retries {
                match rl!(self, self.ex.place_algo(&req)) {
                    Ok(a) => {
                        placed = Some(a.algo_id);
                        break;
                    }
                    Err(e) if e.would_trigger() => {
                        would_trigger = true;
                        last = e.to_string();
                        break;
                    }
                    Err(e) => last = e.to_string(),
                }
                if attempt + 1 < max_retries {
                    let elapsed = started.elapsed().as_millis() as u64;
                    if elapsed >= timeout_ms {
                        break;
                    }
                    let step = SL_RETRY_BACKOFF_MS
                        [(attempt as usize).min(SL_RETRY_BACKOFF_MS.len() - 1)]
                    .min(timeout_ms - elapsed);
                    tokio::time::sleep(std::time::Duration::from_millis(step)).await;
                }
            }
            let elapsed_confirm_ms =
                now.saturating_sub(confirm_base).max(0) + (started.elapsed().as_millis() as i64);
            if elapsed_confirm_ms > 2_000 {
                let msg = format!(
                    "SL {} belum terkonfirmasi dalam <=2000 ms (elapsed={} ms)",
                    t.symbol, elapsed_confirm_ms
                );
                self.audit.log(
                    now,
                    "SL_MISSING",
                    json!({"cid": cid, "symbol": t.symbol, "elapsed_ms": elapsed_confirm_ms, "placed": placed.is_some()}),
                );
                self.notifier.notify("WARN", "SL_MISSING", &msg, now);
            }
            if elapsed_confirm_ms > 5_000 {
                if let Some(id) = placed.take() {
                    let _ = rl!(self, self.ex.cancel_algo(&t.symbol, id));
                }
                if last.is_empty() {
                    last = format!("melebihi batas GR-8 5000 ms ({elapsed_confirm_ms} ms)");
                }
            }
            match placed {
                Some(id) => {
                    if let Some(tr) = self.book.trades.get_mut(cid) {
                        tr.sl_algo = Some(id);
                        tr.state = TradeState::Open;
                        tr.last_error = None;
                    }
                    self.audit.log(
                        now,
                        "SL_PLACED",
                        json!({"cid": cid, "algo_id": id, "trigger": fmt_dec(req.trigger), "close_position": req.close_position, "elapsed_ms": elapsed_confirm_ms}),
                    );
                    self.save_or_kill(now).await;
                }
                None => {
                    self.audit.log(
                        now,
                        "SL_FAILED",
                        json!({"cid": cid, "err": last, "would_trigger": would_trigger, "elapsed_ms": elapsed_confirm_ms}),
                    );
                    // GR-8 / FR-019: gagal pasang SL atau >5 dtk → flatten reduceOnly + engage_kill
                    if let Err(e) = self.close_trade(cid, ExitKind::StopLoss, now).await {
                        if let Some(tr) = self.book.trades.get_mut(cid) {
                            tr.state = TradeState::Unprotected;
                            tr.last_error = Some(format!("SL gagal: {last}; flatten gagal: {e}"));
                            tr.protect_attempts += 1;
                        }
                        let msg =
                            format!("UNPROTECTED {cid}: SL gagal ({last}) & flatten gagal ({e})");
                        self.audit.log(
                            now,
                            "CRITICAL_UNPROTECTED",
                            json!({"cid": cid, "err": e.to_string()}),
                        );
                        self.notifier
                            .notify("CRITICAL", "CRITICAL_UNPROTECTED", &msg, now);
                        self.engage_kill(&format!("UNPROTECTED {cid}"), false, now)
                            .await;
                        self.save_or_kill(now).await;
                    } else {
                        self.engage_kill(&format!("SL_PROTECTION_FAILED {cid}"), false, now)
                            .await;
                    }
                    return;
                }
            }
        }
        let Some(t) = self.book.trades.get(cid).cloned() else {
            return;
        };
        if t.tp_algo.is_none() && t.state == TradeState::Open {
            let req = self.algo_req(&t, AlgoKind::TakeProfitMarket, t.tp, "tp");
            match rl!(self, self.ex.place_algo(&req)) {
                Ok(a) => {
                    if let Some(tr) = self.book.trades.get_mut(cid) {
                        tr.tp_algo = Some(a.algo_id);
                    }
                    self.audit.log(
                        now,
                        "TP_PLACED",
                        json!({"cid": cid, "algo_id": a.algo_id, "trigger": fmt_dec(req.trigger)}),
                    );
                }
                Err(e) => {
                    self.audit
                        .log(now, "TP_FAILED", json!({"cid": cid, "err": e.to_string()}))
                }
            }
            self.save_or_kill(now).await;
        }
    }

    /// Tutup posisi dengan MARKET reduceOnly. Gagal → posisi TETAP di buku (state tidak dihapus).
    pub async fn close_trade(
        &mut self,
        cid: &str,
        kind: ExitKind,
        now: Ms,
    ) -> Result<(), OmsError> {
        let Some(t) = self.book.trades.get(cid).cloned() else {
            return Err(OmsError::Other("trade tidak ada".into()));
        };
        if matches!(
            t.state,
            TradeState::PendingEntry | TradeState::UnknownSubmit
        ) {
            return self.cancel_entry(cid, now).await;
        }
        let close_cid = self.next_id("CL", &t.symbol, now);
        let req = OrderRequest {
            symbol: t.symbol.clone(),
            side: exit_str(t.side).into(),
            kind: OrderKind::Market,
            qty: t.filled_qty,
            reduce_only: true,
            client_id: close_cid.clone(),
        };
        if let Some(tr) = self.book.trades.get_mut(cid) {
            tr.exit_hint = Some(kind);
            tr.close_attempts += 1;
        }
        self.audit.log(
            now,
            "CLOSE_SUBMIT",
            json!({"cid": cid, "close_cid": close_cid, "kind": format!("{kind:?}")}),
        );
        let res = match rl!(self, self.ex.place_order(&req)) {
            Err(ExError::Unknown(_)) => {
                rl!(self, self.ex.query_order(&t.symbol, &close_cid)).map(|o| {
                    o.unwrap_or(OrderAck {
                        symbol: t.symbol.clone(),
                        order_id: 0,
                        client_id: close_cid.clone(),
                        status: OrderStatus::Rejected,
                        side: req.side.clone(),
                        orig_qty: req.qty,
                        executed_qty: Decimal::ZERO,
                        avg_price: Decimal::ZERO,
                        reduce_only: true,
                        update_ms: now,
                    })
                })
            }
            r => r,
        };
        match res {
            Ok(a) if a.status == OrderStatus::Filled => {
                self.finish_close(cid, kind, a.avg_price, Decimal::ZERO, a.update_ms.max(now))
                    .await;
                Ok(())
            }
            Ok(a)
                if a.status == OrderStatus::Rejected
                    || a.status == OrderStatus::Expired
                    || a.status == OrderStatus::Canceled =>
            {
                let e = OmsError::Other(format!("close {close_cid} status {:?}", a.status));
                self.mark_close_failed(cid, &e.to_string(), now).await;
                Err(e)
            }
            Ok(_) => {
                // ACK (NEW): tunggu event FILLED dari user stream
                if let Some(tr) = self.book.trades.get_mut(cid) {
                    tr.state = TradeState::Closing;
                }
                self.save_or_kill(now).await;
                Ok(())
            }
            Err(e) => {
                self.mark_close_failed(cid, &e.to_string(), now).await;
                Err(e.into())
            }
        }
    }

    async fn mark_close_failed(&mut self, cid: &str, err: &str, now: Ms) {
        if let Some(tr) = self.book.trades.get_mut(cid) {
            tr.last_error = Some(format!("close gagal: {err}"));
        }
        self.audit
            .log(now, "CLOSE_FAILED", json!({"cid": cid, "err": err}));
        self.save_or_kill(now).await;
    }

    /// Evaluate an already closed bar; cancellation is reconciled against the exchange.
    pub async fn on_closed_price(
        &mut self,
        symbol: &str,
        close: Decimal,
        close_ms: Ms,
    ) -> Result<(), OmsError> {
        let ids: Vec<String> = self
            .book
            .trades
            .values()
            .filter(|t| {
                t.symbol == symbol
                    && t.is_active()
                    && t.entry_kind == EntryKind::LimitMaker
                    && !t.entry_final
                    && close_ms > t.created_ms
                    && t.cancel_beyond.is_some_and(|level| match t.side {
                        Side::Long => close < level,
                        Side::Short => close > level,
                    })
            })
            .map(|t| t.client_id.clone())
            .collect();
        for cid in ids {
            self.audit.log(
                close_ms,
                "CANCEL_BEYOND",
                json!({"cid": cid, "symbol": symbol, "close": fmt_dec(close)}),
            );
            self.cancel_entry(&cid, close_ms).await?;
        }
        Ok(())
    }

    async fn cancel_entry(&mut self, cid: &str, now: Ms) -> Result<(), OmsError> {
        let Some(t) = self.book.trades.get(cid).cloned() else {
            return Ok(());
        };
        let r = rl!(self, self.ex.cancel_order(&t.symbol, cid));
        // apa pun hasilnya, cek status sebenarnya (bisa sudah terisi)
        let q = rl!(self, self.ex.query_order(&t.symbol, cid))?;
        match q {
            Some(a) if a.executed_qty > Decimal::ZERO => {
                // Box::pin memutus rekursi async (apply → protect → close → cancel → apply)
                Box::pin(self.apply_entry_status(
                    cid,
                    a.status,
                    a.executed_qty,
                    a.avg_price,
                    Decimal::ZERO,
                    now,
                ))
                .await;
            }
            Some(a) if !a.status.is_final() => {
                r?;
            }
            None => {
                return Err(OmsError::Other(
                    "status entry tidak ditemukan setelah cancel; trade tetap disimpan".into(),
                ));
            }
            Some(_) if t.filled_qty > Decimal::ZERO => {
                return Err(OmsError::Other(
                    "cancel status tidak memastikan sisa entry; posisi tetap disimpan".into(),
                ));
            }
            Some(_) => {
                self.book.trades.remove(cid);
                self.audit.log(
                    now,
                    "ENTRY_CANCELLED",
                    json!({"cid": cid, "symbol": t.symbol, "reason": "cancel"}),
                );
                self.save_or_kill(now).await;
            }
        }
        Ok(())
    }

    async fn finish_close(
        &mut self,
        cid: &str,
        kind: ExitKind,
        exit_px: Decimal,
        fee: Decimal,
        ts: Ms,
    ) {
        let Some(mut t) = self.book.trades.remove(cid) else {
            return;
        };
        let px = if exit_px.is_zero() { t.sl } else { exit_px };
        let exit_fee = if fee.is_zero() {
            px * t.filled_qty * to_dec(self.risk_cfg.fee_taker)
        } else {
            fee
        };
        t.fees += exit_fee;
        let s = to_dec(t.side.sign());
        let gross = s * (px - t.avg_fill) * t.filled_qty;
        let pnl = to_f64(gross - t.fees);
        t.state = TradeState::Closed;
        t.closed_ms = Some(ts);
        t.exit_price = Some(px);
        t.exit_kind = Some(kind);
        t.pnl_net = Some(pnl);
        let was_daily_stop = self.book.risk.daily_stop_active(ts, &self.risk_cfg);
        self.book.risk.record_exit(kind, pnl, ts, &self.risk_cfg);
        let now_daily_stop = self.book.risk.daily_stop_active(ts, &self.risk_cfg);
        if !was_daily_stop && now_daily_stop {
            let msg = format!(
                "Daily stop aktif: daily_sl={} realized_net={:.2}",
                self.book.risk.daily_sl_count, self.book.risk.daily_realized_net
            );
            self.audit.log(
                ts,
                "DAILY_STOP",
                json!({"daily_sl": self.book.risk.daily_sl_count, "daily_realized_net": self.book.risk.daily_realized_net}),
            );
            self.notifier.notify("WARN", "DAILY_STOP", &msg, ts);
        }
        for id in [t.sl_algo, t.tp_algo].into_iter().flatten() {
            let _ = rl!(self, self.ex.cancel_algo(&t.symbol, id));
        }
        self.audit.log(
            ts,
            "TRADE_CLOSED",
            json!({"cid": cid, "symbol": t.symbol, "kind": format!("{kind:?}"), "exit": fmt_dec(px), "pnl_net": pnl, "daily_sl": self.book.risk.daily_sl_count, "cooldown_until": self.book.risk.cooldown_until}),
        );
        self.book.closed.push(t);
        self.save_or_kill(ts).await;
    }

    /// Event user-data stream = sumber kebenaran.
    pub async fn on_event(&mut self, ev: UserEvent, now: Ms) {
        match ev {
            UserEvent::Order(o) => {
                if self
                    .book
                    .trades
                    .get(&o.client_id)
                    .is_some_and(|t| t.is_active() && t.client_id == o.client_id && !o.reduce_only)
                {
                    self.apply_entry_status(
                        &o.client_id.clone(),
                        o.status,
                        o.cum_qty,
                        o.avg_price,
                        o.fee,
                        o.ts,
                    )
                    .await;
                    return;
                }
                let is_exit = o.reduce_only
                    || o.close_position
                    || o.orig_type == "STOP_MARKET"
                    || o.orig_type == "TAKE_PROFIT_MARKET";
                if is_exit && o.status == OrderStatus::Filled {
                    let found = self
                        .active()
                        .find(|t| t.symbol == o.symbol && t.has_position())
                        .map(|t| t.client_id.clone());
                    if let Some(cid) = found {
                        let hint = self.book.trades.get(&cid).and_then(|t| t.exit_hint);
                        let kind = match o.orig_type.as_str() {
                            "STOP_MARKET" => ExitKind::StopLoss,
                            "TAKE_PROFIT_MARKET" => ExitKind::TakeProfit,
                            _ => hint.unwrap_or(ExitKind::Manual),
                        };
                        self.finish_close(&cid, kind, o.avg_price, o.fee, o.ts)
                            .await;
                    }
                }
            }
            UserEvent::Algo {
                symbol,
                algo_id,
                status,
                ts,
                ..
            } => {
                let found = self
                    .active()
                    .find(|t| {
                        t.symbol == symbol
                            && (t.sl_algo == Some(algo_id) || t.tp_algo == Some(algo_id))
                    })
                    .map(|t| t.client_id.clone());
                let Some(cid) = found else { return };
                let Some(t) = self.book.trades.get_mut(&cid) else {
                    return;
                };
                let is_sl = t.sl_algo == Some(algo_id);
                match status.as_str() {
                    "TRIGGERED" | "TRIGGERING" => {
                        t.exit_hint = Some(if is_sl {
                            ExitKind::StopLoss
                        } else {
                            ExitKind::TakeProfit
                        })
                    }
                    "CANCELED" | "EXPIRED" | "REJECTED" => {
                        // dibatalkan bukan oleh kita → proteksi hilang, pasang ulang di tick()
                        if is_sl {
                            t.sl_algo = None;
                        } else {
                            t.tp_algo = None;
                        }
                        self.audit.log(
                            ts,
                            "ALGO_LOST",
                            json!({"cid": cid, "algo_id": algo_id, "status": status}),
                        );
                    }
                    _ => {}
                }
                self.save_or_kill(ts).await;
            }
            UserEvent::Account { positions, ts } => {
                for (sym, amt, _) in positions {
                    if amt.is_zero() {
                        if let Some(t) = self
                            .book
                            .trades
                            .values_mut()
                            .find(|t| t.symbol == sym && t.has_position())
                        {
                            t.flat_seen_ms.get_or_insert(ts);
                        }
                    }
                }
                let _ = now;
            }
            UserEvent::ListenKeyExpired => self.audit.log(now, "LISTEN_KEY_EXPIRED", json!({})),
        }
    }

    /// Loop periodik: roll hari, re-query `UnknownSubmit`, kill switch, proteksi ulang, TTL pending, finalisasi flat.
    pub async fn tick(&mut self, now: Ms) {
        self.book.risk.roll_day(now);
        self.tick_unknown(now).await;
        let killed = self.kill.is_engaged();
        let flatten = killed
            && (self.cfg.flatten_on_kill
                || self.kill.reason().is_some_and(|r| r.contains("FLATTEN")));
        let ids: Vec<String> = self.active().map(|t| t.client_id.clone()).collect();
        for cid in ids {
            let Some(t) = self.book.trades.get(&cid).cloned() else {
                continue;
            };
            match t.state {
                TradeState::PendingEntry if killed || now >= t.expires_ms => {
                    let _ = self.cancel_entry(&cid, now).await;
                    if !killed {
                        self.audit.log(now, "ENTRY_EXPIRED", json!({"cid": cid}));
                    }
                }
                TradeState::Open | TradeState::Unprotected
                    if t.flat_seen_ms.is_some_and(|f| now - f >= 5_000) =>
                {
                    let k = t.exit_hint.unwrap_or(ExitKind::Manual);
                    self.finish_close(&cid, k, Decimal::ZERO, Decimal::ZERO, now)
                        .await;
                }
                TradeState::Open | TradeState::Unprotected => {
                    if flatten {
                        let _ = self.close_trade(&cid, ExitKind::Manual, now).await;
                    } else if t.sl_algo.is_none()
                        || t.tp_algo.is_none()
                        || t.state == TradeState::Unprotected
                    {
                        self.protect(&cid, now).await;
                    } else if t
                        .last_error
                        .as_deref()
                        .is_some_and(|e| e.starts_with("close gagal"))
                    {
                        let k = t.exit_hint.unwrap_or(ExitKind::Manual);
                        let _ = self.close_trade(&cid, k, now).await;
                    }
                }
                TradeState::Closing if t.flat_seen_ms.is_some_and(|f| now - f >= 5_000) => {
                    let k = t.exit_hint.unwrap_or(ExitKind::Manual);
                    self.finish_close(&cid, k, Decimal::ZERO, Decimal::ZERO, now)
                        .await;
                }
                _ => {}
            }
        }
        self.save_or_kill(now).await;
    }

    /// Kill switch: tulis file, batalkan pending, opsional flatten semua, dan kirim notifikasi.
    pub async fn engage_kill(&mut self, reason: &str, flatten: bool, now: Ms) {
        let _ = self.kill.engage(reason);
        self.audit.log(
            now,
            "KILL_ENGAGED",
            json!({"reason": reason, "flatten": flatten}),
        );
        self.notifier.notify(
            "CRITICAL",
            "KILL_ENGAGED",
            &format!("Kill switch aktif: {reason} (flatten={flatten})"),
            now,
        );
        let ids: Vec<String> = self.active().map(|t| t.client_id.clone()).collect();
        for cid in ids {
            let Some(st) = self.book.trades.get(&cid).map(|t| t.state) else {
                continue;
            };
            if st == TradeState::PendingEntry {
                let _ = self.cancel_entry(&cid, now).await;
            } else if flatten && matches!(st, TradeState::Open | TradeState::Unprotected) {
                let _ = self.close_trade(&cid, ExitKind::Manual, now).await;
            }
        }
    }

    fn fallback_sl(&self, sym: &str, side: Side, entry: Decimal) -> Decimal {
        let pct = to_dec(self.risk_cfg.sl_max_pct);
        let raw = if side == Side::Long {
            entry * (Decimal::ONE - pct)
        } else {
            entry * (Decimal::ONE + pct)
        };
        match self.filters.get(sym) {
            Some(f) => f.round_sl(side, raw),
            None => raw.round_dp(8),
        }
    }

    /// Reconcile buku lokal ↔ exchange (boot & periodik).
    pub async fn reconcile(&mut self, now: Ms) -> Result<(), OmsError> {
        let positions = rl!(self, self.ex.positions())?;
        let algos = rl!(self, self.ex.open_algo_orders())?;
        // 1a. pending entry lokal → cek status sebenarnya
        let pend: Vec<(String, String)> = self
            .active()
            .filter(|t| t.state == TradeState::PendingEntry)
            .map(|t| (t.client_id.clone(), t.symbol.clone()))
            .collect();
        for (cid, sym) in pend {
            match rl!(self, self.ex.query_order(&sym, &cid))? {
                Some(a) => {
                    self.apply_entry_status(
                        &cid,
                        a.status,
                        a.executed_qty,
                        a.avg_price,
                        Decimal::ZERO,
                        now,
                    )
                    .await
                }
                None => {
                    self.book.trades.remove(&cid);
                    self.audit
                        .log(now, "RECONCILE_DROP_PENDING", json!({"cid": cid}));
                }
            }
        }
        // 1b. UnknownSubmit lokal → perlakukan sama dengan tick_unknown (jangan langsung dihapus bila belum kedaluwarsa)
        let unk: Vec<(String, String, Ms, u8)> = self
            .active()
            .filter(|t| t.state == TradeState::UnknownSubmit)
            .map(|t| {
                (
                    t.client_id.clone(),
                    t.symbol.clone(),
                    t.unknown_since_ms.unwrap_or(t.created_ms),
                    t.unknown_attempts,
                )
            })
            .collect();
        for (cid, sym, since, attempts) in unk {
            match rl!(self, self.ex.query_order(&sym, &cid)) {
                Ok(Some(a)) => {
                    self.apply_entry_status(
                        &cid,
                        a.status,
                        a.executed_qty,
                        a.avg_price,
                        Decimal::ZERO,
                        a.update_ms.max(now),
                    )
                    .await;
                }
                Ok(None) | Err(_) => {
                    if let Some(p) = positions
                        .iter()
                        .find(|p| p.symbol == sym && !p.amt.is_zero())
                    {
                        self.adopt_unknown_position(&cid, p, now).await;
                    } else if (attempts as usize) >= UNKNOWN_CUM_BACKOFF_MS.len()
                        || now.saturating_sub(since) >= 30_000
                    {
                        self.resolve_unknown_final(&cid, &sym, now).await;
                    }
                }
            }
        }
        // 2. posisi exchange
        for p in &positions {
            let Some(side) = p.side() else { continue };
            let cid = match self.cid_on(&p.symbol) {
                Some(c) if self.book.trades.get(&c).is_some_and(|t| t.has_position()) => c,
                _ => {
                    // posisi yatim (tidak dikenal buku lokal)
                    self.audit.log(
                        now,
                        "ORPHAN_POSITION",
                        json!({"symbol": p.symbol, "amt": fmt_dec(p.amt), "entry": fmt_dec(p.entry_price), "policy": format!("{:?}", self.cfg.orphan_policy)}),
                    );
                    self.notifier.notify(
                        "WARN",
                        "RECONCILE_ORPHAN",
                        &format!(
                            "Posisi yatim ditemukan saat reconcile: {} amt={} entry={}",
                            p.symbol,
                            fmt_dec(p.amt),
                            fmt_dec(p.entry_price)
                        ),
                        now,
                    );
                    let cid = self.next_id("OR", &p.symbol, now);
                    let sl = self.fallback_sl(&p.symbol, side, p.entry_price);
                    let tp = if side == Side::Long {
                        p.entry_price + (p.entry_price - sl) * to_dec(self.risk_cfg.rr)
                    } else {
                        p.entry_price - (sl - p.entry_price) * to_dec(self.risk_cfg.rr)
                    };
                    let t = ManagedTrade {
                        client_id: cid.clone(),
                        symbol: p.symbol.clone(),
                        strategy: "ORPHAN".into(),
                        side,
                        entry_kind: EntryKind::Market,
                        planned_qty: p.amt.abs(),
                        filled_qty: p.amt.abs(),
                        entry_price: p.entry_price,
                        avg_fill: p.entry_price,
                        sl,
                        tp,
                        planned_risk: Decimal::ZERO,
                        state: TradeState::Open,
                        sl_algo: None,
                        tp_algo: None,
                        created_ms: now,
                        expires_ms: now,
                        cancel_beyond: None,
                        entry_final: true,
                        knn_features: None,
                        knn_signal_ms: None,
                        unknown_since_ms: None,
                        unknown_attempts: 0,
                        fill_confirmed_at: Some(now),
                        validated: false,
                        filled_ms: Some(now),
                        closed_ms: None,
                        exit_price: None,
                        exit_kind: None,
                        exit_hint: None,
                        fees: Decimal::ZERO,
                        pnl_net: None,
                        close_attempts: 0,
                        protect_attempts: 0,
                        flat_seen_ms: None,
                        last_error: None,
                        adopted: true,
                    };
                    self.book.trades.insert(cid.clone(), t);
                    cid
                }
            };
            if let Some(t) = self.book.trades.get_mut(&cid) {
                t.filled_qty = p.amt.abs();
                let ex_side = exit_str(side);
                let sl_live = algos
                    .iter()
                    .find(|a| {
                        a.symbol == p.symbol && a.side == ex_side && a.order_type == "STOP_MARKET"
                    })
                    .map(|a| a.algo_id);
                let tp_live = algos
                    .iter()
                    .find(|a| {
                        a.symbol == p.symbol
                            && a.side == ex_side
                            && a.order_type == "TAKE_PROFIT_MARKET"
                    })
                    .map(|a| a.algo_id);
                t.sl_algo = sl_live;
                t.tp_algo = tp_live;
            }
            let Some(t) = self.book.trades.get(&cid).cloned() else {
                continue;
            };
            if t.adopted && self.cfg.orphan_policy == OrphanPolicy::Flatten && t.sl_algo.is_none() {
                let _ = self.close_trade(&cid, ExitKind::Manual, now).await;
            } else if t.sl_algo.is_none() || t.tp_algo.is_none() {
                self.protect(&cid, now).await;
            }
        }
        // 3. trade lokal yang posisinya sudah tidak ada di exchange → tutup di buku
        let gone: Vec<String> = self
            .active()
            .filter(|t| {
                t.has_position()
                    && !positions
                        .iter()
                        .any(|p| p.symbol == t.symbol && !p.amt.is_zero())
            })
            .map(|t| t.client_id.clone())
            .collect();
        for cid in gone {
            let k = self
                .book
                .trades
                .get(&cid)
                .and_then(|t| t.exit_hint)
                .unwrap_or(ExitKind::Manual);
            self.audit.log(now, "RECONCILE_FLAT", json!({"cid": cid}));
            self.finish_close(&cid, k, Decimal::ZERO, Decimal::ZERO, now)
                .await;
        }
        // 4. algo yatim di simbol tanpa posisi → batalkan
        for a in &algos {
            if !positions
                .iter()
                .any(|p| p.symbol == a.symbol && !p.amt.is_zero())
            {
                let _ = rl!(self, self.ex.cancel_algo(&a.symbol, a.algo_id));
                self.audit.log(
                    now,
                    "ORPHAN_ALGO_CANCELLED",
                    json!({"algo_id": a.algo_id, "symbol": a.symbol}),
                );
            }
        }
        self.save_or_kill(now).await;
        Ok(())
    }
}
