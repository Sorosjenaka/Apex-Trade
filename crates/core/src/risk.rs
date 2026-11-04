//! Gate risk akun (Rules §4.2; GR-5..GR-7, GR-10; FR-014..FR-016, FR-024).
//! Urutan cek TETAP: ARM → regime → SL range → max posisi → cooldown → stop harian → equity floor → spread → sizing.
//! Kode yang sama dipakai backtest dan engine live.
use crate::config::RiskConfig;
use crate::sizing::{size_order, Sizing, SizingInput};
use crate::time::utc_day;
use crate::types::{Ms, SkipReason};
use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum ExitKind {
    /// Exit oleh SL algo atau oleh flatten FR-019 → menambah sl_count + cooldown.
    StopLoss,
    TakeProfit,
    Manual,
    Liquidation,
    TimeStop,
    EndOfData,
}

impl ExitKind {
    pub fn counts_as_sl(self) -> bool {
        matches!(self, ExitKind::StopLoss | ExitKind::Liquidation)
    }
}

/// State risk harian yang dipersist.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, Default)]
pub struct RiskState {
    pub day: i64,
    pub daily_sl_count: u32,
    pub daily_realized_net: f64,
    /// Entry baru ditolak sampai ts ini (fill SL + 60 menit). Global, semua simbol & strategi.
    pub cooldown_until: Ms,
    pub last_sl_fill_ms: Option<Ms>,
}

/// Konteks untuk satu keputusan entry.
#[derive(Clone, Debug)]
pub struct GateContext {
    pub now: Ms,
    pub armed: bool,
    /// Regime global + screener simbol mengizinkan sisi sinyal.
    pub regime_ok: bool,
    pub regime_side_ok: bool,
    /// max posisi dari regime (≤ 2). Regime hanya boleh MENGURANGI.
    pub regime_max_pos: usize,
    pub open_positions: usize,
    pub pending_entries: usize,
    pub symbol_busy: bool,
    pub equity: f64,
    pub spread_bps: Option<f64>,
}

impl RiskState {
    pub fn new(now: Ms) -> Self {
        RiskState { day: utc_day(now), ..Default::default() }
    }

    /// Reset counter harian di 00:00:00.000 UTC.
    pub fn roll_day(&mut self, now: Ms) {
        let d = utc_day(now);
        if d != self.day {
            self.day = d;
            self.daily_sl_count = 0;
            self.daily_realized_net = 0.0;
        }
    }

    /// Catat exit. `pnl_net` = PnL realized neto (fee+funding). SL → cooldown dari waktu fill SL.
    pub fn record_exit(&mut self, kind: ExitKind, pnl_net: f64, fill_ts: Ms, cfg: &RiskConfig) {
        self.roll_day(fill_ts);
        self.daily_realized_net += pnl_net;
        if kind.counts_as_sl() {
            self.daily_sl_count += 1;
            self.last_sl_fill_ms = Some(fill_ts);
            self.cooldown_until = self.cooldown_until.max(fill_ts + cfg.cooldown_ms);
        }
    }

    pub fn daily_stop_active(&self, now: Ms, cfg: &RiskConfig) -> bool {
        if utc_day(now) != self.day {
            return false;
        }
        self.daily_sl_count >= cfg.daily_sl_limit || self.daily_realized_net <= -cfg.daily_loss_limit_usd + 1e-12
    }

    pub fn in_cooldown(&self, now: Ms) -> bool {
        now < self.cooldown_until
    }

    /// Gate akun tanpa sizing (dipakai juga untuk mencatat funnel). Gagal pertama → SKIP.
    pub fn check_gates(&mut self, ctx: &GateContext, sl_pct: f64, cfg: &RiskConfig) -> Result<(), SkipReason> {
        self.roll_day(ctx.now);
        if !ctx.armed {
            return Err(SkipReason::NotArmed);
        }
        if !ctx.regime_ok {
            return Err(SkipReason::Regime);
        }
        if !ctx.regime_side_ok {
            return Err(SkipReason::RegimeSide);
        }
        if !(cfg.sl_min_pct - 1e-12..=cfg.sl_max_pct + 1e-12).contains(&sl_pct) {
            return Err(SkipReason::SlRange);
        }
        let cap = cfg.max_positions.min(ctx.regime_max_pos);
        if ctx.open_positions + ctx.pending_entries >= cap {
            return Err(SkipReason::MaxPositions);
        }
        if ctx.symbol_busy {
            return Err(SkipReason::SymbolBusy);
        }
        if self.in_cooldown(ctx.now) {
            return Err(SkipReason::Cooldown);
        }
        if self.daily_stop_active(ctx.now, cfg) {
            return Err(SkipReason::DailyStop);
        }
        if ctx.equity < cfg.equity_floor_usd {
            return Err(SkipReason::EquityFloor);
        }
        if let Some(s) = ctx.spread_bps {
            if s > cfg.max_spread_bps {
                return Err(SkipReason::Spread);
            }
        }
        Ok(())
    }

    /// Gate lengkap + sizing. Mengembalikan sizing final atau alasan SKIP.
    pub fn evaluate_entry(&mut self, ctx: &GateContext, sizing: &SizingInput, cfg: &RiskConfig) -> Result<Sizing, SkipReason> {
        let e = crate::num::to_f64(sizing.filters.round_entry(sizing.side, sizing.entry));
        let s = crate::num::to_f64(sizing.filters.round_sl(sizing.side, sizing.sl));
        let sl_pct = if e > 0.0 { (e - s).abs() / e } else { 0.0 };
        self.check_gates(ctx, sl_pct, cfg)?;
        size_order(sizing)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::filters::SymbolFilters;
    use crate::sizing::EntryKind;
    use crate::time::parse_ymd;
    use crate::types::Side;
    use rust_decimal_macros::dec;

    fn ctx(now: Ms) -> GateContext {
        GateContext {
            now,
            armed: true,
            regime_ok: true,
            regime_side_ok: true,
            regime_max_pos: 2,
            open_positions: 0,
            pending_entries: 0,
            symbol_busy: false,
            equity: 60.0,
            spread_bps: Some(1.0),
        }
    }

    #[test]
    fn max_positions_counts_pending() {
        let c = RiskConfig::default();
        let t = parse_ymd("2026-01-02").unwrap();
        let mut r = RiskState::new(t);
        let mut x = ctx(t);
        x.open_positions = 1;
        x.pending_entries = 1;
        assert_eq!(r.check_gates(&x, 0.03, &c), Err(SkipReason::MaxPositions));
        x.pending_entries = 0;
        assert!(r.check_gates(&x, 0.03, &c).is_ok());
        x.regime_max_pos = 1;
        assert_eq!(r.check_gates(&x, 0.03, &c), Err(SkipReason::MaxPositions));
        x.regime_max_pos = 5; // regime tidak bisa menaikkan di atas 2
        x.open_positions = 2;
        assert_eq!(r.check_gates(&x, 0.03, &c), Err(SkipReason::MaxPositions));
    }

    #[test]
    fn cooldown_one_hour_global_after_sl() {
        let c = RiskConfig::default();
        let t = parse_ymd("2026-01-02").unwrap() + 3_600_000;
        let mut r = RiskState::new(t);
        r.record_exit(ExitKind::StopLoss, -1.0, t, &c);
        assert_eq!(r.check_gates(&ctx(t + 59 * 60_000), 0.03, &c), Err(SkipReason::Cooldown));
        assert!(r.check_gates(&ctx(t + 3_600_000), 0.03, &c).is_ok());
        // TP tidak memicu cooldown
        let mut r2 = RiskState::new(t);
        r2.record_exit(ExitKind::TakeProfit, 2.0, t, &c);
        assert!(r2.check_gates(&ctx(t + 1), 0.03, &c).is_ok());
    }

    #[test]
    fn daily_stop_after_six_sl_resets_at_utc_midnight() {
        let c = RiskConfig::default();
        let d0 = parse_ymd("2026-01-02").unwrap();
        let mut r = RiskState::new(d0);
        for i in 0..6 {
            // pnl kecil supaya yang memicu adalah jumlah SL, bukan -$6
            r.record_exit(ExitKind::StopLoss, -0.5, d0 + i * 1000, &c);
        }
        let after = d0 + 5 * 3_600_000;
        assert_eq!(r.check_gates(&ctx(after), 0.03, &c), Err(SkipReason::DailyStop));
        // 23:59:59.999 masih stop
        let end = d0 + 86_400_000 - 1;
        assert_eq!(r.check_gates(&ctx(end), 0.03, &c), Err(SkipReason::DailyStop));
        // 00:00 UTC hari berikutnya reset
        assert!(r.check_gates(&ctx(d0 + 86_400_000), 0.03, &c).is_ok());
        assert_eq!(r.daily_sl_count, 0);
    }

    #[test]
    fn daily_stop_on_minus_six_dollars() {
        let c = RiskConfig::default();
        let d0 = parse_ymd("2026-01-02").unwrap();
        let mut r = RiskState::new(d0);
        r.record_exit(ExitKind::Manual, -3.0, d0 + 10, &c);
        r.record_exit(ExitKind::Manual, -3.0, d0 + 20, &c);
        assert_eq!(r.daily_sl_count, 0);
        assert_eq!(r.check_gates(&ctx(d0 + 30), 0.03, &c), Err(SkipReason::DailyStop));
    }

    #[test]
    fn gate_order_is_fixed() {
        let c = RiskConfig::default();
        let t = parse_ymd("2026-01-02").unwrap();
        let mut r = RiskState::new(t);
        let mut x = ctx(t);
        x.armed = false;
        x.regime_ok = false;
        assert_eq!(r.check_gates(&x, 0.01, &c), Err(SkipReason::NotArmed));
        x.armed = true;
        assert_eq!(r.check_gates(&x, 0.01, &c), Err(SkipReason::Regime));
        x.regime_ok = true;
        assert_eq!(r.check_gates(&x, 0.01, &c), Err(SkipReason::SlRange));
        x.equity = 39.0;
        assert_eq!(r.check_gates(&x, 0.03, &c), Err(SkipReason::EquityFloor));
        x.equity = 60.0;
        x.spread_bps = Some(4.0);
        assert_eq!(r.check_gates(&x, 0.03, &c), Err(SkipReason::Spread));
    }

    #[test]
    fn evaluate_entry_sizes() {
        let c = RiskConfig::default();
        let t = parse_ymd("2026-01-02").unwrap();
        let mut r = RiskState::new(t);
        let f = SymbolFilters::synthetic("X", dec!(0.01), dec!(0.001), dec!(5));
        let s = r
            .evaluate_entry(&ctx(t), &SizingInput { side: Side::Long, entry: dec!(100), sl: dec!(98), kind: EntryKind::LimitMaker, filters: &f, risk: &c }, &c)
            .unwrap();
        assert_eq!(s.qty, dec!(0.478));
    }
}
