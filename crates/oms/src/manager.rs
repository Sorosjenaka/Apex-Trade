//! OrderManager: state machine entry → proteksi → exit, reconcile, kill switch, persist, audit.
//!
//! Siklus trade: `PendingEntry` → (fill) → `Open` (SL+TP algo terpasang) → `Closing` → `Closed`.
//! `Unprotected` = posisi ada tapi SL gagal dipasang DAN flatten gagal → kill switch aktif,
//! posisi TETAP di buku dan dicoba ulang setiap `tick()`.
use crate::exchange::FuturesExchange;
use crate::killswitch::KillSwitch;
use crate::safety::ExecMode;
use crate::store::StateStore;
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

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum TradeState {
    PendingEntry,
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
        matches!(self.state, TradeState::Open | TradeState::Unprotected | TradeState::Closing)
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
    store: StateStore,
    pub audit: AuditLog,
    pub kill: KillSwitch,
    filters: BTreeMap<String, SymbolFilters>,
    /// jeda backoff rate-limit maksimum (ms)
    pub max_backoff_ms: u64,
}

macro_rules! rl {
    ($self:ident, $call:expr) => {{
        let mut n = 0u32;
        loop {
            match $call.await {
                Err(ExError::RateLimited { retry_after_ms, banned: false }) if n < 4 => {
                    n += 1;
                    let wait = retry_after_ms.max(50 * (1 << n)).min($self.max_backoff_ms);
                    $self.audit.log(apex_core::time::now_ms(), "RATE_LIMIT_BACKOFF", json!({"wait_ms": wait, "attempt": n}));
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
    pub fn new(ex: E, mode: ExecMode, cfg: EngineConfig, risk_cfg: RiskConfig, store: StateStore, audit: AuditLog, filters: BTreeMap<String, SymbolFilters>) -> anyhow::Result<Self> {
        if mode != ExecMode::Paper && !cfg.arm {
            anyhow::bail!("ARM=false: OrderManager non-paper ditolak");
        }
        if mode == ExecMode::Paper && ex.name() != "mock" {
            anyhow::bail!("mode paper hanya boleh memakai MockExchange (tanpa panggilan exchange)");
        }
        let book: Book = store.load()?;
        let kill = KillSwitch::new(&cfg.kill_file);
        Ok(OrderManager { armed: cfg.arm, ex, mode, cfg, risk_cfg, book, store, audit, kill, filters, max_backoff_ms: 30_000 })
    }

    fn save(&self) {
        if let Err(e) = self.store.save(&self.book) {
            self.audit.log(apex_core::time::now_ms(), "STATE_SAVE_ERROR", json!({"err": e.to_string()}));
        }
    }

    pub fn active(&self) -> impl Iterator<Item = &ManagedTrade> {
        self.book.trades.values().filter(|t| t.is_active())
    }
    pub fn open_count(&self) -> usize {
        self.active().filter(|t| t.has_position()).count()
    }
    pub fn pending_count(&self) -> usize {
        self.active().filter(|t| t.state == TradeState::PendingEntry).count()
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
        self.audit.log(now, "BOOT", json!({"mode": format!("{:?}", self.mode), "exchange": self.ex.name(), "armed": self.armed}));
        self.reconcile(now).await
    }

    /// CROSSED + leverage 25x per simbol (sekali, diingat di state).
    pub async fn prepare_symbol(&mut self, sym: &str) -> Result<(), OmsError> {
        if self.book.prepared.contains(sym) {
            return Ok(());
        }
        rl!(self, self.ex.set_margin_crossed(sym))?;
        let lev = rl!(self, self.ex.set_leverage(sym, self.risk_cfg.leverage))?;
        if lev != self.risk_cfg.leverage {
            return Err(OmsError::Other(format!("leverage {sym} = {lev}, bukan {}", self.risk_cfg.leverage)));
        }
        self.book.prepared.insert(sym.into());
        self.save();
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
        self.prepare_symbol(&it.symbol).await?;
        let sz = &it.sizing;
        let cid = self.next_id(it.strategy.get(..2).unwrap_or("XX"), &it.symbol, now);
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
        // persist SEBELUM kirim → crash setelah kirim tetap bisa di-reconcile via clientOrderId
        self.book.trades.insert(cid.clone(), t);
        self.save();
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
        self.audit.log(now, "ENTRY_SUBMIT", json!({"cid": cid, "symbol": it.symbol, "side": req.side, "kind": format!("{:?}", req.kind), "qty": fmt_dec(sz.qty), "sl": fmt_dec(sz.sl), "tp": fmt_dec(sz.tp), "risk": fmt_dec(sz.risk)}));
        let res = rl!(self, self.ex.place_order(&req));
        let ack = match res {
            Ok(a) => Some(a),
            Err(ExError::Unknown(m)) => {
                self.audit.log(now, "ENTRY_UNKNOWN", json!({"cid": cid, "msg": m}));
                rl!(self, self.ex.query_order(&it.symbol, &cid))?
            }
            Err(e) => {
                self.audit.log(now, "ENTRY_REJECTED", json!({"cid": cid, "err": e.to_string()}));
                self.book.trades.remove(&cid);
                self.save();
                return Err(e.into());
            }
        };
        match ack {
            None => {
                self.audit.log(now, "ENTRY_NOT_FOUND", json!({"cid": cid}));
                self.book.trades.remove(&cid);
                self.save();
                Err(OmsError::Other("order tidak ditemukan setelah outcome unknown".into()))
            }
            Some(a) => {
                self.apply_entry_status(&cid, a.status, a.executed_qty, a.avg_price, Decimal::ZERO, a.update_ms.max(now)).await;
                Ok(cid)
            }
        }
    }

    /// Terapkan status entry (dari ACK REST atau event stream). Idempoten via cum qty.
    async fn apply_entry_status(&mut self, cid: &str, status: OrderStatus, cum: Decimal, avg: Decimal, fee: Decimal, ts: Ms) {
        let Some(t) = self.book.trades.get_mut(cid) else { return };
        if !t.is_active() {
            return;
        }
        t.entry_final |= status.is_final();
        let mut newly_filled = false;
        if cum > t.filled_qty {
            t.filled_qty = cum;
            if !avg.is_zero() {
                t.avg_fill = avg;
            }
            if fee.is_zero() {
                let rate = if t.entry_kind == EntryKind::Market { self.risk_cfg.fee_taker } else { self.risk_cfg.fee_maker };
                t.fees = t.avg_fill * t.filled_qty * to_dec(rate);
            } else {
                t.fees += fee;
            }
            if t.state == TradeState::PendingEntry {
                t.state = TradeState::Open;
                t.filled_ms = Some(ts);
                newly_filled = true;
            }
        }
        let cancelled_empty = matches!(status, OrderStatus::Canceled | OrderStatus::Expired | OrderStatus::Rejected) && t.filled_qty.is_zero();
        if cancelled_empty {
            let t = self.book.trades.remove(cid).unwrap();
            self.audit.log(ts, "ENTRY_CANCELLED", json!({"cid": cid, "symbol": t.symbol, "status": format!("{status:?}")}));
            self.save();
            return;
        }
        if newly_filled {
            let (s, q, p) = (t.symbol.clone(), t.filled_qty, t.avg_fill);
            self.audit.log(ts, "ENTRY_FILLED", json!({"cid": cid, "symbol": s, "qty": fmt_dec(q), "avg": fmt_dec(p), "status": format!("{status:?}")}));
        }
        self.save();
        if self.book.trades.get(cid).is_some_and(|t| t.has_position() && t.sl_algo.is_none()) {
            self.protect(cid, ts).await;
        }
    }

    fn algo_req(&self, t: &ManagedTrade, kind: AlgoKind, trigger: Decimal, tag: &str) -> AlgoRequest {
        let cp = self.cfg.protective_close_position;
        AlgoRequest {
            symbol: t.symbol.clone(),
            side: exit_str(t.side).into(),
            kind,
            trigger,
            close_position: cp,
            qty: if cp { None } else { Some(t.filled_qty) },
            client_algo_id: format!("{tag}{}", &t.client_id[3.min(t.client_id.len())..]).chars().take(36).collect(),
        }
    }

    /// Pasang SL (wajib) dan TP. SL gagal semua retry → flatten. Flatten gagal → Unprotected + kill.
    pub async fn protect(&mut self, cid: &str, now: Ms) {
        let Some(t) = self.book.trades.get(cid).cloned() else { return };
        if !t.has_position() || t.state == TradeState::Closing {
            return;
        }
        if t.sl_algo.is_none() {
            let req = self.algo_req(&t, AlgoKind::StopMarket, t.sl, "sl");
            let mut placed = None;
            let mut would_trigger = false;
            let mut last = String::new();
            for _ in 0..self.cfg.protection_retries.max(1) {
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
            }
            match placed {
                Some(id) => {
                    let t = self.book.trades.get_mut(cid).unwrap();
                    t.sl_algo = Some(id);
                    t.state = TradeState::Open;
                    t.last_error = None;
                    self.audit.log(now, "SL_PLACED", json!({"cid": cid, "algo_id": id, "trigger": fmt_dec(req.trigger), "close_position": req.close_position}));
                    self.save();
                }
                None => {
                    self.audit.log(now, "SL_FAILED", json!({"cid": cid, "err": last, "would_trigger": would_trigger}));
                    // FR-019: tidak boleh ada posisi tanpa SL → flatten
                    if let Err(e) = self.close_trade(cid, ExitKind::StopLoss, now).await {
                        if let Some(t) = self.book.trades.get_mut(cid) {
                            t.state = TradeState::Unprotected;
                            t.last_error = Some(format!("SL gagal: {last}; flatten gagal: {e}"));
                            t.protect_attempts += 1;
                        }
                        let _ = self.kill.engage(&format!("UNPROTECTED {cid}"));
                        self.audit.log(now, "CRITICAL_UNPROTECTED", json!({"cid": cid, "err": e.to_string()}));
                        self.save();
                    }
                    return;
                }
            }
        }
        let t = self.book.trades.get(cid).cloned().unwrap();
        if t.tp_algo.is_none() && t.state == TradeState::Open {
            let req = self.algo_req(&t, AlgoKind::TakeProfitMarket, t.tp, "tp");
            match rl!(self, self.ex.place_algo(&req)) {
                Ok(a) => {
                    self.book.trades.get_mut(cid).unwrap().tp_algo = Some(a.algo_id);
                    self.audit.log(now, "TP_PLACED", json!({"cid": cid, "algo_id": a.algo_id, "trigger": fmt_dec(req.trigger)}));
                }
                Err(e) => self.audit.log(now, "TP_FAILED", json!({"cid": cid, "err": e.to_string()})),
            }
            self.save();
        }
    }

    /// Tutup posisi dengan MARKET reduceOnly. Gagal → posisi TETAP di buku (state tidak dihapus).
    pub async fn close_trade(&mut self, cid: &str, kind: ExitKind, now: Ms) -> Result<(), OmsError> {
        let Some(t) = self.book.trades.get(cid).cloned() else { return Err(OmsError::Other("trade tidak ada".into())) };
        if t.state == TradeState::PendingEntry {
            return self.cancel_entry(cid, now).await;
        }
        let close_cid = self.next_id("CL", &t.symbol, now);
        let req = OrderRequest { symbol: t.symbol.clone(), side: exit_str(t.side).into(), kind: OrderKind::Market, qty: t.filled_qty, reduce_only: true, client_id: close_cid.clone() };
        if let Some(tr) = self.book.trades.get_mut(cid) {
            tr.exit_hint = Some(kind);
            tr.close_attempts += 1;
        }
        self.audit.log(now, "CLOSE_SUBMIT", json!({"cid": cid, "close_cid": close_cid, "kind": format!("{kind:?}")}));
        let res = match rl!(self, self.ex.place_order(&req)) {
            Err(ExError::Unknown(_)) => rl!(self, self.ex.query_order(&t.symbol, &close_cid)).map(|o| o.unwrap_or(OrderAck { symbol: t.symbol.clone(), order_id: 0, client_id: close_cid.clone(), status: OrderStatus::Rejected, side: req.side.clone(), orig_qty: req.qty, executed_qty: Decimal::ZERO, avg_price: Decimal::ZERO, reduce_only: true, update_ms: now })),
            r => r,
        };
        match res {
            Ok(a) if a.status == OrderStatus::Filled => {
                self.finish_close(cid, kind, a.avg_price, Decimal::ZERO, a.update_ms.max(now)).await;
                Ok(())
            }
            Ok(a) if a.status == OrderStatus::Rejected || a.status == OrderStatus::Expired || a.status == OrderStatus::Canceled => {
                let e = OmsError::Other(format!("close {close_cid} status {:?}", a.status));
                self.mark_close_failed(cid, &e.to_string(), now);
                Err(e)
            }
            Ok(_) => {
                // ACK (NEW): tunggu event FILLED dari user stream
                if let Some(tr) = self.book.trades.get_mut(cid) {
                    tr.state = TradeState::Closing;
                }
                self.save();
                Ok(())
            }
            Err(e) => {
                self.mark_close_failed(cid, &e.to_string(), now);
                Err(e.into())
            }
        }
    }

    fn mark_close_failed(&mut self, cid: &str, err: &str, now: Ms) {
        if let Some(tr) = self.book.trades.get_mut(cid) {
            tr.last_error = Some(format!("close gagal: {err}"));
        }
        self.audit.log(now, "CLOSE_FAILED", json!({"cid": cid, "err": err}));
        self.save();
    }

    /// Evaluate an already closed bar; cancellation is reconciled against the exchange.
    pub async fn on_closed_price(&mut self, symbol: &str, close: Decimal, close_ms: Ms) -> Result<(), OmsError> {
        let ids: Vec<String> = self.book.trades.values().filter(|t| {
            t.symbol == symbol && t.is_active() && t.entry_kind == EntryKind::LimitMaker && !t.entry_final && close_ms > t.created_ms
                && t.cancel_beyond.is_some_and(|level| match t.side {
                    Side::Long => close < level,
                    Side::Short => close > level,
                })
        }).map(|t| t.client_id.clone()).collect();
        for cid in ids {
            self.audit.log(close_ms, "CANCEL_BEYOND", json!({"cid": cid, "symbol": symbol, "close": fmt_dec(close)}));
            self.cancel_entry(&cid, close_ms).await?;
        }
        Ok(())
    }

    async fn cancel_entry(&mut self, cid: &str, now: Ms) -> Result<(), OmsError> {
        let Some(t) = self.book.trades.get(cid).cloned() else { return Ok(()) };
        let r = rl!(self, self.ex.cancel_order(&t.symbol, cid));
        // apa pun hasilnya, cek status sebenarnya (bisa sudah terisi)
        let q = rl!(self, self.ex.query_order(&t.symbol, cid))?;
        match q {
            Some(a) if a.executed_qty > Decimal::ZERO => {
                // Box::pin memutus rekursi async (apply → protect → close → cancel → apply)
                Box::pin(self.apply_entry_status(cid, a.status, a.executed_qty, a.avg_price, Decimal::ZERO, now)).await;
            }
            Some(a) if !a.status.is_final() => {
                r?;
            }
            None => {
                return Err(OmsError::Other("status entry tidak ditemukan setelah cancel; trade tetap disimpan".into()));
            }
            Some(_) if t.filled_qty > Decimal::ZERO => {
                return Err(OmsError::Other("cancel status tidak memastikan sisa entry; posisi tetap disimpan".into()));
            }
            Some(_) => {
                self.book.trades.remove(cid);
                self.audit.log(now, "ENTRY_CANCELLED", json!({"cid": cid, "symbol": t.symbol, "reason": "cancel"}));
                self.save();
            }
        }
        Ok(())
    }

    async fn finish_close(&mut self, cid: &str, kind: ExitKind, exit_px: Decimal, fee: Decimal, ts: Ms) {
        let Some(mut t) = self.book.trades.remove(cid) else { return };
        let px = if exit_px.is_zero() { t.sl } else { exit_px };
        let exit_fee = if fee.is_zero() { px * t.filled_qty * to_dec(self.risk_cfg.fee_taker) } else { fee };
        t.fees += exit_fee;
        let s = to_dec(t.side.sign());
        let gross = s * (px - t.avg_fill) * t.filled_qty;
        let pnl = to_f64(gross - t.fees);
        t.state = TradeState::Closed;
        t.closed_ms = Some(ts);
        t.exit_price = Some(px);
        t.exit_kind = Some(kind);
        t.pnl_net = Some(pnl);
        self.book.risk.record_exit(kind, pnl, ts, &self.risk_cfg);
        for id in [t.sl_algo, t.tp_algo].into_iter().flatten() {
            let _ = rl!(self, self.ex.cancel_algo(&t.symbol, id));
        }
        self.audit.log(ts, "TRADE_CLOSED", json!({"cid": cid, "symbol": t.symbol, "kind": format!("{kind:?}"), "exit": fmt_dec(px), "pnl_net": pnl, "daily_sl": self.book.risk.daily_sl_count, "cooldown_until": self.book.risk.cooldown_until}));
        self.book.closed.push(t);
        self.save();
    }

    /// Event user-data stream = sumber kebenaran.
    pub async fn on_event(&mut self, ev: UserEvent, now: Ms) {
        match ev {
            UserEvent::Order(o) => {
                if self.book.trades.get(&o.client_id).is_some_and(|t| t.is_active() && t.client_id == o.client_id && !o.reduce_only) {
                    self.apply_entry_status(&o.client_id.clone(), o.status, o.cum_qty, o.avg_price, o.fee, o.ts).await;
                    return;
                }
                let is_exit = o.reduce_only || o.close_position || o.orig_type == "STOP_MARKET" || o.orig_type == "TAKE_PROFIT_MARKET";
                if is_exit && o.status == OrderStatus::Filled {
                    let found = self.active().find(|t| t.symbol == o.symbol && t.has_position()).map(|t| t.client_id.clone());
                    if let Some(cid) = found {
                        let hint = self.book.trades[&cid].exit_hint;
                        let kind = match o.orig_type.as_str() {
                            "STOP_MARKET" => ExitKind::StopLoss,
                            "TAKE_PROFIT_MARKET" => ExitKind::TakeProfit,
                            _ => hint.unwrap_or(ExitKind::Manual),
                        };
                        self.finish_close(&cid, kind, o.avg_price, o.fee, o.ts).await;
                    }
                }
            }
            UserEvent::Algo { symbol, algo_id, status, ts, .. } => {
                let found = self.active().find(|t| t.symbol == symbol && (t.sl_algo == Some(algo_id) || t.tp_algo == Some(algo_id))).map(|t| t.client_id.clone());
                let Some(cid) = found else { return };
                let t = self.book.trades.get_mut(&cid).unwrap();
                let is_sl = t.sl_algo == Some(algo_id);
                match status.as_str() {
                    "TRIGGERED" | "TRIGGERING" => t.exit_hint = Some(if is_sl { ExitKind::StopLoss } else { ExitKind::TakeProfit }),
                    "CANCELED" | "EXPIRED" | "REJECTED" => {
                        // dibatalkan bukan oleh kita → proteksi hilang, pasang ulang di tick()
                        if is_sl {
                            t.sl_algo = None;
                        } else {
                            t.tp_algo = None;
                        }
                        self.audit.log(ts, "ALGO_LOST", json!({"cid": cid, "algo_id": algo_id, "status": status}));
                    }
                    _ => {}
                }
                self.save();
            }
            UserEvent::Account { positions, ts } => {
                for (sym, amt, _) in positions {
                    if amt.is_zero() {
                        if let Some(t) = self.book.trades.values_mut().find(|t| t.symbol == sym && t.has_position()) {
                            t.flat_seen_ms.get_or_insert(ts);
                        }
                    }
                }
                let _ = now;
            }
            UserEvent::ListenKeyExpired => self.audit.log(now, "LISTEN_KEY_EXPIRED", json!({})),
        }
    }

    /// Loop periodik: roll hari, kill switch, proteksi ulang, TTL pending, finalisasi flat.
    pub async fn tick(&mut self, now: Ms) {
        self.book.risk.roll_day(now);
        let killed = self.kill.is_engaged();
        let flatten = killed && (self.cfg.flatten_on_kill || self.kill.reason().is_some_and(|r| r.contains("FLATTEN")));
        let ids: Vec<String> = self.active().map(|t| t.client_id.clone()).collect();
        for cid in ids {
            let Some(t) = self.book.trades.get(&cid).cloned() else { continue };
            match t.state {
                TradeState::PendingEntry if killed || now >= t.expires_ms => {
                    let _ = self.cancel_entry(&cid, now).await;
                    if !killed {
                        self.audit.log(now, "ENTRY_EXPIRED", json!({"cid": cid}));
                    }
                }
                TradeState::Open | TradeState::Unprotected if t.flat_seen_ms.is_some_and(|f| now - f >= 5_000) => {
                    let k = t.exit_hint.unwrap_or(ExitKind::Manual);
                    self.finish_close(&cid, k, Decimal::ZERO, Decimal::ZERO, now).await;
                }
                TradeState::Open | TradeState::Unprotected => {
                    if flatten {
                        let _ = self.close_trade(&cid, ExitKind::Manual, now).await;
                    } else if t.sl_algo.is_none() || t.tp_algo.is_none() || t.state == TradeState::Unprotected {
                        self.protect(&cid, now).await;
                    } else if t.last_error.as_deref().is_some_and(|e| e.starts_with("close gagal")) {
                        let k = t.exit_hint.unwrap_or(ExitKind::Manual);
                        let _ = self.close_trade(&cid, k, now).await;
                    }
                }
                TradeState::Closing if t.flat_seen_ms.is_some_and(|f| now - f >= 5_000) => {
                    let k = t.exit_hint.unwrap_or(ExitKind::Manual);
                    self.finish_close(&cid, k, Decimal::ZERO, Decimal::ZERO, now).await;
                }
                _ => {}
            }
        }
        self.save();
    }

    /// Kill switch: tulis file, batalkan pending, opsional flatten semua.
    pub async fn engage_kill(&mut self, reason: &str, flatten: bool, now: Ms) {
        let _ = self.kill.engage(reason);
        self.audit.log(now, "KILL_ENGAGED", json!({"reason": reason, "flatten": flatten}));
        let ids: Vec<String> = self.active().map(|t| t.client_id.clone()).collect();
        for cid in ids {
            let st = self.book.trades[&cid].state;
            if st == TradeState::PendingEntry {
                let _ = self.cancel_entry(&cid, now).await;
            } else if flatten {
                let _ = self.close_trade(&cid, ExitKind::Manual, now).await;
            }
        }
    }

    fn fallback_sl(&self, sym: &str, side: Side, entry: Decimal) -> Decimal {
        let pct = to_dec(self.risk_cfg.sl_max_pct);
        let raw = if side == Side::Long { entry * (Decimal::ONE - pct) } else { entry * (Decimal::ONE + pct) };
        match self.filters.get(sym) {
            Some(f) => f.round_sl(side, raw),
            None => raw.round_dp(8),
        }
    }

    /// Reconcile buku lokal ↔ exchange (boot & periodik).
    pub async fn reconcile(&mut self, now: Ms) -> Result<(), OmsError> {
        let positions = rl!(self, self.ex.positions())?;
        let algos = rl!(self, self.ex.open_algo_orders())?;
        // 1. pending entry lokal → cek status sebenarnya
        let pend: Vec<(String, String)> = self.active().filter(|t| t.state == TradeState::PendingEntry).map(|t| (t.client_id.clone(), t.symbol.clone())).collect();
        for (cid, sym) in pend {
            match rl!(self, self.ex.query_order(&sym, &cid))? {
                Some(a) => self.apply_entry_status(&cid, a.status, a.executed_qty, a.avg_price, Decimal::ZERO, now).await,
                None => {
                    self.book.trades.remove(&cid);
                    self.audit.log(now, "RECONCILE_DROP_PENDING", json!({"cid": cid}));
                }
            }
        }
        // 2. posisi exchange
        for p in &positions {
            let Some(side) = p.side() else { continue };
            let cid = match self.cid_on(&p.symbol) {
                Some(c) if self.book.trades[&c].has_position() => c,
                _ => {
                    // posisi yatim (tidak dikenal buku lokal)
                    self.audit.log(now, "ORPHAN_POSITION", json!({"symbol": p.symbol, "amt": fmt_dec(p.amt), "entry": fmt_dec(p.entry_price), "policy": format!("{:?}", self.cfg.orphan_policy)}));
                    let cid = self.next_id("OR", &p.symbol, now);
                    let sl = self.fallback_sl(&p.symbol, side, p.entry_price);
                    let tp = if side == Side::Long { p.entry_price + (p.entry_price - sl) * to_dec(self.risk_cfg.rr) } else { p.entry_price - (sl - p.entry_price) * to_dec(self.risk_cfg.rr) };
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
            {
                let t = self.book.trades.get_mut(&cid).unwrap();
                t.filled_qty = p.amt.abs();
                let ex_side = exit_str(side);
                // cocokkan algo yang benar-benar ada di exchange
                let sl_live = algos.iter().find(|a| a.symbol == p.symbol && a.side == ex_side && a.order_type == "STOP_MARKET").map(|a| a.algo_id);
                let tp_live = algos.iter().find(|a| a.symbol == p.symbol && a.side == ex_side && a.order_type == "TAKE_PROFIT_MARKET").map(|a| a.algo_id);
                t.sl_algo = sl_live;
                t.tp_algo = tp_live;
            }
            let t = self.book.trades[&cid].clone();
            if t.adopted && self.cfg.orphan_policy == OrphanPolicy::Flatten && t.sl_algo.is_none() {
                let _ = self.close_trade(&cid, ExitKind::Manual, now).await;
            } else if t.sl_algo.is_none() || t.tp_algo.is_none() {
                self.protect(&cid, now).await;
            }
        }
        // 3. trade lokal yang posisinya sudah tidak ada di exchange → tutup di buku
        let gone: Vec<String> = self.active().filter(|t| t.has_position() && !positions.iter().any(|p| p.symbol == t.symbol && !p.amt.is_zero())).map(|t| t.client_id.clone()).collect();
        for cid in gone {
            let k = self.book.trades[&cid].exit_hint.unwrap_or(ExitKind::Manual);
            self.audit.log(now, "RECONCILE_FLAT", json!({"cid": cid}));
            self.finish_close(&cid, k, Decimal::ZERO, Decimal::ZERO, now).await;
        }
        // 4. algo yatim di simbol tanpa posisi → batalkan
        for a in &algos {
            if !positions.iter().any(|p| p.symbol == a.symbol && !p.amt.is_zero()) {
                let _ = rl!(self, self.ex.cancel_algo(&a.symbol, a.algo_id));
                self.audit.log(now, "ORPHAN_ALGO_CANCELLED", json!({"algo_id": a.algo_id, "symbol": a.symbol}));
            }
        }
        self.save();
        Ok(())
    }
}
