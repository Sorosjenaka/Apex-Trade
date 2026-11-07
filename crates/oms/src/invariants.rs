//! Pemeriksaan invarian aturan risiko & bursa (`R1`..`R8`) untuk audit `RULE_VIOLATION` (`WP-8`).
//!
//! Fungsi [`check_rule_invariants`] dipanggil setiap `tick()` dan saat `boot()` oleh `OrderManager`
//! untuk mendeteksi pelanggaran aturan emas (`Rules.md` GR-1..GR-9) secara independen dari jalur gate
//! sebelum order dikirim. Setiap pelanggaran dideduplikasi per `(rule, trade_id)` agar log audit tidak
//! dibanjiri event berulang pada siklus polling berikutnya.

use std::collections::{BTreeMap, BTreeSet};

use apex_core::config::RiskConfig;
use apex_core::num::to_dec;
use apex_core::types::{Ms, Side};
use rust_decimal::Decimal;
use serde::{Deserialize, Serialize};

use crate::manager::{Book, ManagedTrade, TradeState};
use crate::safety::ExecMode;
use crate::types::{AlgoAck, PositionInfo};

/// Toleransi pembulatan `stepSize` atas risiko nominal maksimum (`0,5%`).
pub const RISK_NOMINAL_EPSILON: Decimal = Decimal::from_parts(5, 0, 0, false, 3);
/// Toleransi pembulatan tick pada jarak SL (`0,01%` = 1 bps).
pub const SL_BAND_EPSILON: Decimal = Decimal::from_parts(1, 0, 0, false, 4);
/// Batas GR-8 konfirmasi SL di bursa (`2000 ms`).
pub const MAX_UNPROTECTED_SL_MS: i64 = 2_000;
/// Masa tenggang default adopsi/reconcile posisi yatim (`15000 ms` = 1 siklus poll).
pub const DEFAULT_RECONCILE_GRACE_MS: i64 = 15_000;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct RuleViolation {
    pub rule: &'static str,
    pub trade_id: String,
    pub detail: String,
}

#[derive(Clone, Debug)]
pub struct ExchangeInvariantState {
    pub mode: ExecMode,
    pub now_ms: Ms,
    pub kill_engaged: bool,
    pub kill_since_ms: Option<Ms>,
    pub positions: Vec<PositionInfo>,
    pub algos: Vec<AlgoAck>,
    pub go_strategies: BTreeSet<String>,
    pub reconcile_grace_ms: i64,
    pub verify_symbol_config: bool,
    pub unprotected_first_seen: BTreeMap<String, Ms>,
    pub orphan_first_seen: BTreeMap<String, Ms>,
    pub emitted_dedup: BTreeSet<(String, String)>,
}

impl Default for ExchangeInvariantState {
    fn default() -> Self {
        Self::new(ExecMode::Paper)
    }
}

impl ExchangeInvariantState {
    pub fn new(mode: ExecMode) -> Self {
        Self {
            mode,
            now_ms: 0,
            kill_engaged: false,
            kill_since_ms: None,
            positions: Vec::new(),
            algos: Vec::new(),
            go_strategies: BTreeSet::new(),
            reconcile_grace_ms: DEFAULT_RECONCILE_GRACE_MS,
            verify_symbol_config: mode != ExecMode::Paper,
            unprotected_first_seen: BTreeMap::new(),
            orphan_first_seen: BTreeMap::new(),
            emitted_dedup: BTreeSet::new(),
        }
    }
}

fn push_dedup(
    out: &mut Vec<RuleViolation>,
    dedup: &mut BTreeSet<(String, String)>,
    rule: &'static str,
    trade_id: impl Into<String>,
    detail: String,
) {
    let tid = trade_id.into();
    if dedup.insert((rule.to_string(), tid.clone())) {
        out.push(RuleViolation {
            rule,
            trade_id: tid,
            detail,
        });
    }
}

fn check_trade_rules(
    t: &ManagedTrade,
    book: &Book,
    risk_cfg: &RiskConfig,
    ex: &ExchangeInvariantState,
    last_closed_ms: Option<Ms>,
    out: &mut Vec<RuleViolation>,
    dedup: &mut BTreeSet<(String, String)>,
) {
    if t.adopted {
        return;
    }
    let max_risk = to_dec(risk_cfg.risk_usd) * (Decimal::ONE + RISK_NOMINAL_EPSILON);
    if t.planned_risk > max_risk {
        push_dedup(
            out,
            dedup,
            "R1",
            &t.client_id,
            format!(
                "planned_risk {} > batas {}",
                t.planned_risk.round_dp(4),
                max_risk.round_dp(4)
            ),
        );
    }
    if t.entry_price > Decimal::ZERO {
        let sl_pct = (t.entry_price - t.sl).abs() / t.entry_price;
        let lo = to_dec(risk_cfg.sl_min_pct) - SL_BAND_EPSILON;
        let hi = to_dec(risk_cfg.sl_max_pct) + SL_BAND_EPSILON;
        if sl_pct < lo || sl_pct > hi {
            push_dedup(
                out,
                dedup,
                "R2",
                &t.client_id,
                format!(
                    "sl_pct {} di luar pita [{}, {}]",
                    sl_pct.round_dp(5),
                    lo.round_dp(5),
                    hi.round_dp(5)
                ),
            );
        }
    }
    let on_kill = ex.kill_engaged
        && ex.kill_since_ms.is_none_or(|ks| t.created_ms >= ks)
        && matches!(
            t.state,
            TradeState::PendingEntry | TradeState::UnknownSubmit
        );
    let on_cd = book
        .risk
        .last_sl_fill_ms
        .is_some_and(|sl_ts| t.created_ms > sl_ts && t.created_ms < book.risk.cooldown_until);
    let on_dstop = book.risk.daily_stop_active(t.created_ms, risk_cfg)
        && last_closed_ms.is_none_or(|lc| t.created_ms > lc);
    if on_kill || on_cd || on_dstop {
        push_dedup(
            out,
            dedup,
            "R4",
            &t.client_id,
            format!(
                "entry saat guard aktif (kill={on_kill}, cooldown={on_cd}, daily_stop={on_dstop})"
            ),
        );
    }
    if ex.mode != ExecMode::Paper && !ex.go_strategies.contains(&t.strategy) {
        push_dedup(
            out,
            dedup,
            "R8",
            &t.client_id,
            format!(
                "strategi {} belum berstatus GO pada mode {:?}",
                t.strategy, ex.mode
            ),
        );
    }
}

fn check_single_exchange_position(
    p: &PositionInfo,
    book: &Book,
    risk_cfg: &RiskConfig,
    ex: &mut ExchangeInvariantState,
    out: &mut Vec<RuleViolation>,
    dedup: &mut BTreeSet<(String, String)>,
) {
    let Some(side) = p.side() else { return };
    let exit_side = match side {
        Side::Long => "SELL",
        Side::Short => "BUY",
    };
    let has_sl = ex
        .algos
        .iter()
        .any(|a| a.symbol == p.symbol && a.side == exit_side && a.order_type == "STOP_MARKET");
    let matched = book
        .trades
        .values()
        .find(|t| t.symbol == p.symbol && t.has_position());
    let tid = matched
        .map(|t| t.client_id.as_str())
        .unwrap_or(p.symbol.as_str());

    if has_sl {
        ex.unprotected_first_seen.remove(&p.symbol);
    } else {
        let base_ts = matched
            .and_then(|t| t.fill_confirmed_at.or(t.filled_ms))
            .unwrap_or_else(|| {
                *ex.unprotected_first_seen
                    .entry(p.symbol.clone())
                    .or_insert(ex.now_ms)
            });
        let elapsed = ex.now_ms.saturating_sub(base_ts);
        if elapsed > MAX_UNPROTECTED_SL_MS {
            push_dedup(
                out,
                dedup,
                "R5",
                tid,
                format!("posisi {} tanpa SL {} ms", p.symbol, elapsed),
            );
        }
    }

    if matched.is_some() {
        ex.orphan_first_seen.remove(&p.symbol);
    } else {
        let first = *ex
            .orphan_first_seen
            .entry(p.symbol.clone())
            .or_insert(ex.now_ms);
        let elapsed = ex.now_ms.saturating_sub(first);
        if elapsed > ex.reconcile_grace_ms {
            push_dedup(
                out,
                dedup,
                "R6",
                &p.symbol,
                format!("posisi yatim {} {} ms", p.symbol, elapsed),
            );
        }
    }

    if ex.verify_symbol_config || ex.mode != ExecMode::Paper || book.prepared.contains(&p.symbol) {
        let margin_ok = matches!(
            p.margin_type.to_ascii_uppercase().as_str(),
            "CROSSED" | "CROSS"
        );
        let lev_ok = p.leverage == 0 || p.leverage == risk_cfg.leverage;
        if !margin_ok || !lev_ok {
            push_dedup(
                out,
                dedup,
                "R7",
                tid,
                format!(
                    "margin={} lev={} (wajib CROSSED/{})",
                    p.margin_type, p.leverage, risk_cfg.leverage
                ),
            );
        }
    }
}

/// Evaluasi seluruh aturan invarian `R1`..`R8` dengan deduplikasi per `(rule, trade_id)`.
pub fn check_rule_invariants(
    book: &Book,
    risk_cfg: &RiskConfig,
    exchange_state: &mut ExchangeInvariantState,
) -> Vec<RuleViolation> {
    let mut out = Vec::new();
    let mut dedup = std::mem::take(&mut exchange_state.emitted_dedup);
    let last_closed_ms = book.closed.iter().filter_map(|c| c.closed_ms).max();

    for t in book.trades.values().filter(|t| t.is_active()) {
        check_trade_rules(
            t,
            book,
            risk_cfg,
            exchange_state,
            last_closed_ms,
            &mut out,
            &mut dedup,
        );
    }

    let open_cnt = book.trades.values().filter(|t| t.has_position()).count();
    let active_non_orphan = book
        .trades
        .values()
        .filter(|t| t.is_active() && !t.adopted)
        .count();
    if open_cnt > risk_cfg.max_positions || active_non_orphan > risk_cfg.max_positions {
        let key = book.trades.keys().cloned().collect::<Vec<_>>().join("+");
        push_dedup(
            &mut out,
            &mut dedup,
            "R3",
            if key.is_empty() { "BOOK".into() } else { key },
            format!(
                "posisi aktif > {} (open={open_cnt}, total={active_non_orphan})",
                risk_cfg.max_positions
            ),
        );
    }

    let active_pos: Vec<PositionInfo> = exchange_state
        .positions
        .iter()
        .filter(|p| !p.amt.is_zero())
        .cloned()
        .collect();
    let active_syms: BTreeSet<String> = active_pos.iter().map(|p| p.symbol.clone()).collect();
    exchange_state
        .unprotected_first_seen
        .retain(|k, _| active_syms.contains(k));
    exchange_state
        .orphan_first_seen
        .retain(|k, _| active_syms.contains(k));
    for p in &active_pos {
        check_single_exchange_position(p, book, risk_cfg, exchange_state, &mut out, &mut dedup);
    }

    exchange_state.emitted_dedup = dedup;
    out
}
