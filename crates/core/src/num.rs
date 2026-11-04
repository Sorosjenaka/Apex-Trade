//! Konversi angka uang/harga/qty. `Decimal` untuk uang; `f64` hanya fitur.
use rust_decimal::prelude::*;

/// Konversi f64 → Decimal (dibatasi 12 desimal agar stabil). Satu-satunya pintu konversi.
pub fn to_dec(x: f64) -> Decimal {
    Decimal::from_f64(x)
        .map(|d| d.round_dp_with_strategy(12, RoundingStrategy::MidpointNearestEven))
        .unwrap_or(Decimal::ZERO)
}

pub fn to_f64(d: Decimal) -> f64 {
    d.to_f64().unwrap_or(f64::NAN)
}

/// floor(x / step) × step. step <= 0 → x.
pub fn floor_to_step(x: Decimal, step: Decimal) -> Decimal {
    if step <= Decimal::ZERO {
        return x;
    }
    ((x / step).floor() * step).normalize()
}

/// ceil(x / step) × step.
pub fn ceil_to_step(x: Decimal, step: Decimal) -> Decimal {
    if step <= Decimal::ZERO {
        return x;
    }
    ((x / step).ceil() * step).normalize()
}

pub fn to_dec_round_down(x: f64, step: Decimal) -> Decimal {
    floor_to_step(to_dec(x), step)
}

/// Format desimal untuk parameter REST Binance (tanpa notasi ilmiah, tanpa trailing zero).
pub fn fmt_dec(d: Decimal) -> String {
    d.normalize().to_string()
}

#[cfg(test)]
mod tests {
    use super::*;
    use rust_decimal_macros::dec;

    #[test]
    fn floor_and_ceil() {
        assert_eq!(floor_to_step(dec!(0.4787), dec!(0.001)), dec!(0.478));
        assert_eq!(ceil_to_step(dec!(0.4781), dec!(0.001)), dec!(0.479));
        assert_eq!(floor_to_step(dec!(97.98), dec!(0.1)), dec!(97.9));
        assert_eq!(fmt_dec(dec!(1.2300)), "1.23");
    }
}
