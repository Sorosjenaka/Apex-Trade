//! Filter exchangeInfo per simbol + pembulatan harga/qty (Schema §5).
use crate::num::{ceil_to_step, floor_to_step};
use crate::types::Side;
use rust_decimal::prelude::*;
use serde::{Deserialize, Serialize};
use std::str::FromStr;

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct SymbolFilters {
    pub symbol: String,
    pub tick_size: Decimal,
    pub step_size: Decimal,
    pub min_qty: Decimal,
    pub market_step_size: Decimal,
    pub market_min_qty: Decimal,
    pub market_max_qty: Decimal,
    pub min_notional: Decimal,
    /// PERCENT_PRICE multiplierUp/Down (LIMIT harga harus di dalam band mark × [down, up]).
    pub percent_up: Decimal,
    pub percent_down: Decimal,
}

fn dec_field(v: &serde_json::Value, k: &str) -> Option<Decimal> {
    v.get(k)
        .and_then(|x| x.as_str())
        .and_then(|s| Decimal::from_str(s).ok())
}

impl SymbolFilters {
    /// Parse dari satu elemen `symbols[]` di `/fapi/v1/exchangeInfo`.
    pub fn from_exchange_info_symbol(v: &serde_json::Value) -> Option<SymbolFilters> {
        let symbol = v.get("symbol")?.as_str()?.to_string();
        let mut f = SymbolFilters {
            symbol,
            tick_size: Decimal::ZERO,
            step_size: Decimal::ZERO,
            min_qty: Decimal::ZERO,
            market_step_size: Decimal::ZERO,
            market_min_qty: Decimal::ZERO,
            market_max_qty: Decimal::MAX,
            min_notional: Decimal::ZERO,
            percent_up: Decimal::new(105, 2),
            percent_down: Decimal::new(95, 2),
        };
        for flt in v.get("filters")?.as_array()? {
            match flt.get("filterType").and_then(|x| x.as_str()) {
                Some("PRICE_FILTER") => f.tick_size = dec_field(flt, "tickSize")?,
                Some("LOT_SIZE") => {
                    f.step_size = dec_field(flt, "stepSize")?;
                    f.min_qty = dec_field(flt, "minQty")?;
                }
                Some("MARKET_LOT_SIZE") => {
                    f.market_step_size = dec_field(flt, "stepSize")?;
                    f.market_min_qty = dec_field(flt, "minQty")?;
                    if let Some(m) = dec_field(flt, "maxQty") {
                        f.market_max_qty = m;
                    }
                }
                Some("MIN_NOTIONAL") => f.min_notional = dec_field(flt, "notional")?,
                Some("PERCENT_PRICE") => {
                    if let Some(u) = dec_field(flt, "multiplierUp") {
                        f.percent_up = u;
                    }
                    if let Some(d) = dec_field(flt, "multiplierDown") {
                        f.percent_down = d;
                    }
                }
                _ => {}
            }
        }
        if f.market_step_size.is_zero() {
            f.market_step_size = f.step_size;
            f.market_min_qty = f.min_qty;
        }
        if f.tick_size.is_zero() || f.step_size.is_zero() {
            return None;
        }
        Some(f)
    }

    /// Step yang dipakai sizing: yang lebih kasar dari LOT_SIZE dan MARKET_LOT_SIZE
    /// (SL STOP_MARKET dieksekusi sebagai market).
    pub fn sizing_step(&self) -> Decimal {
        self.step_size.max(self.market_step_size)
    }
    pub fn sizing_min_qty(&self) -> Decimal {
        self.min_qty.max(self.market_min_qty)
    }

    pub fn floor_qty(&self, q: Decimal) -> Decimal {
        floor_to_step(q, self.sizing_step())
    }

    /// Entry limit: long ke bawah, short ke atas.
    pub fn round_entry(&self, side: Side, p: Decimal) -> Decimal {
        match side {
            Side::Long => floor_to_step(p, self.tick_size),
            Side::Short => ceil_to_step(p, self.tick_size),
        }
    }
    /// SL: long ke bawah, short ke atas (arah aman = SL lebih jauh, tidak pernah lebih dekat).
    pub fn round_sl(&self, side: Side, p: Decimal) -> Decimal {
        match side {
            Side::Long => floor_to_step(p, self.tick_size),
            Side::Short => ceil_to_step(p, self.tick_size),
        }
    }
    /// TP: long ke bawah, short ke atas (Schema §5).
    pub fn round_tp(&self, side: Side, p: Decimal) -> Decimal {
        match side {
            Side::Long => floor_to_step(p, self.tick_size),
            Side::Short => ceil_to_step(p, self.tick_size),
        }
    }

    /// PERCENT_PRICE: harga LIMIT harus dalam [mark×down, mark×up].
    pub fn percent_price_ok(&self, price: Decimal, mark: Decimal) -> bool {
        price <= mark * self.percent_up && price >= mark * self.percent_down
    }

    /// Filter sintetis untuk test.
    pub fn synthetic(symbol: &str, tick: Decimal, step: Decimal, min_notional: Decimal) -> Self {
        SymbolFilters {
            symbol: symbol.to_string(),
            tick_size: tick,
            step_size: step,
            min_qty: step,
            market_step_size: step,
            market_min_qty: step,
            market_max_qty: Decimal::MAX,
            min_notional,
            percent_up: Decimal::new(105, 2),
            percent_down: Decimal::new(95, 2),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rust_decimal_macros::dec;

    #[test]
    fn parse_exchange_info_symbol() {
        let v: serde_json::Value = serde_json::json!({
            "symbol": "ETHUSDT",
            "filters": [
                {"filterType":"PRICE_FILTER","tickSize":"0.01","minPrice":"39.86","maxPrice":"306177"},
                {"filterType":"LOT_SIZE","stepSize":"0.001","minQty":"0.001","maxQty":"10000"},
                {"filterType":"MARKET_LOT_SIZE","stepSize":"0.001","minQty":"0.001","maxQty":"2000"},
                {"filterType":"MIN_NOTIONAL","notional":"20"},
                {"filterType":"PERCENT_PRICE","multiplierUp":"1.0500","multiplierDown":"0.9500","multiplierDecimal":"4"}
            ]
        });
        let f = SymbolFilters::from_exchange_info_symbol(&v).unwrap();
        assert_eq!(f.tick_size, dec!(0.01));
        assert_eq!(f.min_notional, dec!(20));
        assert_eq!(f.market_max_qty, dec!(2000));
        assert!(f.percent_price_ok(dec!(3000), dec!(3010)));
        assert!(!f.percent_price_ok(dec!(3200), dec!(3000)));
    }

    #[test]
    fn rounding_directions() {
        let f = SymbolFilters::synthetic("X", dec!(0.01), dec!(0.001), dec!(5));
        assert_eq!(f.round_sl(Side::Long, dec!(97.987)), dec!(97.98));
        assert_eq!(f.round_sl(Side::Short, dec!(102.001)), dec!(102.01));
        assert_eq!(f.round_entry(Side::Long, dec!(100.009)), dec!(100.00));
        assert_eq!(f.round_entry(Side::Short, dec!(100.001)), dec!(100.01));
        assert_eq!(f.floor_qty(dec!(0.4789)), dec!(0.478));
    }
}
