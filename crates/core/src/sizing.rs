//! Sizing risk $1 dengan margin variabel (Rules §4.1; GR-1, GR-2, GR-3; FR-011..FR-013).
//! notional ≈ $1 / jarak SL; margin = notional / 25. Tidak ada sizing acak, tidak ada margin tetap.
use crate::config::RiskConfig;
use crate::filters::SymbolFilters;
use crate::num::to_dec;
use crate::types::{Side, SkipReason};
use rust_decimal::prelude::*;
use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum EntryKind {
    /// LIMIT GTX post-only → fee maker.
    LimitMaker,
    /// MARKET → fee taker.
    Market,
}

#[derive(Clone, Debug)]
pub struct SizingInput<'a> {
    pub side: Side,
    /// Harga entry mentah (akan dibulatkan tick).
    pub entry: Decimal,
    /// Harga SL mentah (akan dibulatkan tick ke arah aman).
    pub sl: Decimal,
    pub kind: EntryKind,
    pub filters: &'a SymbolFilters,
    pub risk: &'a RiskConfig,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Sizing {
    pub side: Side,
    pub entry: Decimal,
    pub sl: Decimal,
    pub tp: Decimal,
    pub sl_pct: Decimal,
    pub loss_unit: Decimal,
    pub qty: Decimal,
    pub notional: Decimal,
    pub margin: Decimal,
    /// risk terencana neto (fee masuk + keluar + slip SL), harus ∈ [0,80; 1,00].
    pub risk: Decimal,
}

/// FR-011/FR-012/FR-013. Gagal → `SkipReason::SlRange` atau `SkipReason::Size`.
pub fn size_order(inp: &SizingInput) -> Result<Sizing, SkipReason> {
    let f = inp.filters;
    let r = inp.risk;
    let entry = f.round_entry(inp.side, inp.entry);
    let sl = f.round_sl(inp.side, inp.sl);
    if entry <= Decimal::ZERO || sl <= Decimal::ZERO {
        return Err(SkipReason::Size);
    }
    let dist = (entry - sl).abs();
    // SL harus di sisi yang benar.
    let correct_side = match inp.side {
        Side::Long => sl < entry,
        Side::Short => sl > entry,
    };
    if !correct_side {
        return Err(SkipReason::SlRange);
    }
    let sl_pct = dist / entry;
    if sl_pct < to_dec(r.sl_min_pct) || sl_pct > to_dec(r.sl_max_pct) {
        return Err(SkipReason::SlRange);
    }
    let fee_in = match inp.kind {
        EntryKind::LimitMaker => to_dec(r.fee_maker),
        EntryKind::Market => to_dec(r.fee_taker),
    };
    let fee_out = to_dec(r.fee_taker);
    let slip = to_dec(r.slip_sl);
    let sgn = match inp.side {
        Side::Long => Decimal::ONE,
        Side::Short => -Decimal::ONE,
    };
    let stop_fill = sl * (Decimal::ONE - sgn * slip);
    let loss_unit = sgn * (entry - stop_fill) + entry * fee_in + stop_fill * fee_out;
    if loss_unit <= Decimal::ZERO {
        return Err(SkipReason::Size);
    }
    let qty = f.floor_qty(to_dec(r.risk_usd) / loss_unit);
    let notional = qty * entry;
    let risk = qty * loss_unit;
    if qty < f.sizing_min_qty()
        || qty <= Decimal::ZERO
        || notional < f.min_notional
        || risk < to_dec(r.min_risk_usd)
        || risk > to_dec(r.risk_usd)
    {
        return Err(SkipReason::Size);
    }
    let lev = Decimal::from(r.leverage);
    let tp_raw = entry + sgn * to_dec(r.rr) * dist;
    let tp = f.round_tp(inp.side, tp_raw);
    Ok(Sizing {
        side: inp.side,
        entry,
        sl,
        tp,
        sl_pct,
        loss_unit,
        qty,
        notional,
        margin: notional / lev,
        risk,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use rust_decimal_macros::dec;

    fn f(tick: Decimal, step: Decimal) -> SymbolFilters {
        SymbolFilters::synthetic("T", tick, step, dec!(5))
    }
    fn run(side: Side, e: Decimal, s: Decimal, step: Decimal) -> Result<Sizing, SkipReason> {
        let fl = f(dec!(0.001), step);
        let rc = RiskConfig::default();
        size_order(&SizingInput { side, entry: e, sl: s, kind: EntryKind::LimitMaker, filters: &fl, risk: &rc })
    }
    fn approx(a: Decimal, b: f64, tol: f64) -> bool {
        (a.to_f64().unwrap() - b).abs() < tol
    }

    #[test]
    fn u01_sl_2pct() {
        let z = run(Side::Long, dec!(100), dec!(98), dec!(0.001)).unwrap();
        assert_eq!(z.qty, dec!(0.478));
        assert_eq!(z.notional, dec!(47.800));
        assert!(approx(z.margin, 1.912, 1e-9));
        assert!(approx(z.risk, 0.9983, 1e-4));
        assert_eq!(z.tp, dec!(104));
    }

    #[test]
    fn u02_sl_4pct() {
        let z = run(Side::Long, dec!(100), dec!(96), dec!(0.001)).unwrap();
        assert_eq!(z.qty, dec!(0.244));
        assert!(approx(z.notional, 24.40, 1e-9));
        assert!(approx(z.margin, 0.976, 1e-9));
        assert!(approx(z.risk, 0.9973, 1e-4));
    }

    #[test]
    fn u03_eth() {
        let fl = SymbolFilters::synthetic("ETHUSDT", dec!(0.01), dec!(0.001), dec!(20));
        let rc = RiskConfig::default();
        let z = size_order(&SizingInput { side: Side::Long, entry: dec!(3000), sl: dec!(2925), kind: EntryKind::LimitMaker, filters: &fl, risk: &rc }).unwrap();
        assert_eq!(z.qty, dec!(0.012));
        assert!(approx(z.notional, 36.0, 1e-9));
        assert!(approx(z.margin, 1.44, 1e-9));
        assert!(approx(z.risk, 0.9318, 1e-3));
    }

    #[test]
    fn u04_u05_sl_range() {
        assert_eq!(run(Side::Long, dec!(100), dec!(98.01), dec!(0.001)), Err(SkipReason::SlRange));
        assert_eq!(run(Side::Long, dec!(100), dec!(95.99), dec!(0.001)), Err(SkipReason::SlRange));
        assert_eq!(run(Side::Short, dec!(100), dec!(101.99), dec!(0.001)), Err(SkipReason::SlRange));
        // SL di sisi salah
        assert_eq!(run(Side::Long, dec!(100), dec!(103), dec!(0.001)), Err(SkipReason::SlRange));
    }

    #[test]
    fn u06_step_rounding_skip() {
        let r = run(Side::Long, dec!(717), dec!(691.905), dec!(0.01));
        assert_eq!(r, Err(SkipReason::Size));
    }

    #[test]
    fn u07_margin_variable() {
        let a = run(Side::Long, dec!(100), dec!(98), dec!(0.001)).unwrap();
        let b = run(Side::Long, dec!(100), dec!(97), dec!(0.001)).unwrap();
        assert_ne!(a.margin, b.margin);
        assert!(a.margin > b.margin);
    }

    #[test]
    fn short_mirror() {
        let z = run(Side::Short, dec!(100), dec!(102), dec!(0.001)).unwrap();
        assert!(z.qty > dec!(0.47) && z.qty < dec!(0.49));
        assert_eq!(z.tp, dec!(96));
    }

    #[test]
    fn market_entry_uses_taker_fee() {
        let fl = f(dec!(0.001), dec!(0.001));
        let rc = RiskConfig::default();
        let m = size_order(&SizingInput { side: Side::Long, entry: dec!(100), sl: dec!(98), kind: EntryKind::Market, filters: &fl, risk: &rc }).unwrap();
        let l = run(Side::Long, dec!(100), dec!(98), dec!(0.001)).unwrap();
        assert!(m.loss_unit > l.loss_unit);
    }

    /// U-08: properti 10.000 kasus — semua yang lolos: 0,80 ≤ risk ≤ 1,00 dan margin = notional/25.
    #[test]
    fn u08_property_10000() {
        let mut s: u64 = 0x9E3779B97F4A7C15;
        let mut next = || {
            s ^= s << 13;
            s ^= s >> 7;
            s ^= s << 17;
            (s >> 11) as f64 / (1u64 << 53) as f64
        };
        let rc = RiskConfig::default();
        let steps = [dec!(0.001), dec!(0.01), dec!(0.1), dec!(1), dec!(10)];
        let mut passed = 0;
        for _ in 0..10_000 {
            let price = 0.001 + next() * 5000.0;
            let slp = 0.015 + next() * 0.03;
            let step = steps[(next() * 5.0) as usize % 5];
            let side = if next() < 0.5 { Side::Long } else { Side::Short };
            let entry = to_dec(price);
            let sl = to_dec(price * (1.0 - side.sign() * slp));
            let fl = SymbolFilters::synthetic("P", dec!(0.0000001), step, dec!(5));
            if let Ok(z) = size_order(&SizingInput { side, entry, sl, kind: EntryKind::LimitMaker, filters: &fl, risk: &rc }) {
                passed += 1;
                assert!(z.risk >= dec!(0.80) && z.risk <= dec!(1.00), "risk {}", z.risk);
                assert_eq!(z.margin, z.notional / dec!(25));
                assert!(z.sl_pct >= dec!(0.02) && z.sl_pct <= dec!(0.04));
            }
        }
        assert!(passed > 1000);
    }
}
