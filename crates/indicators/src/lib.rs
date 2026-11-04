//! Indikator & fitur kausal (nilai di indeks i hanya memakai data ≤ i).
//! Port dari apex `indicators.rs` (EMA, RMA Wilder, RSI, ATR, ADX) + fitur regime dari skrip TradingView:
//! ER Kaufman, Choppiness, Signed Realized Variance (ylUDT58m), Bilateral Filter Trend (yY27v3eD),
//! Decycler High-Low (dOcUNfyv), Distributional Rank (gGTzHW8l), pivot terkonfirmasi, volume profile.
use apex_core::types::{Bar, Ms, Tf};

pub mod regime;
pub use regime::*;

pub fn closes(b: &[Bar]) -> Vec<f64> {
    b.iter().map(|x| x.close).collect()
}

pub fn sma(v: &[f64], n: usize) -> Vec<f64> {
    let mut out = vec![f64::NAN; v.len()];
    if n == 0 {
        return out;
    }
    let mut sum = 0.0;
    for i in 0..v.len() {
        sum += v[i];
        if i >= n {
            sum -= v[i - n];
        }
        if i + 1 >= n {
            out[i] = sum / n as f64;
        }
    }
    out
}

pub fn ema(values: &[f64], period: usize) -> Vec<f64> {
    let mut out = vec![f64::NAN; values.len()];
    if period == 0 {
        return out;
    }
    let alpha = 2.0 / (period as f64 + 1.0);
    let mut last = f64::NAN;
    for (i, &v) in values.iter().enumerate() {
        if !v.is_finite() {
            continue;
        }
        last = if last.is_finite() { alpha * v + (1.0 - alpha) * last } else { v };
        out[i] = last;
    }
    out
}

pub fn rma_wilder(values: &[f64], period: usize) -> Vec<f64> {
    let mut out = vec![f64::NAN; values.len()];
    if period == 0 {
        return out;
    }
    let (mut seed_sum, mut seed_n, mut last) = (0.0, 0usize, f64::NAN);
    for (i, &v) in values.iter().enumerate() {
        if !v.is_finite() {
            continue;
        }
        if !last.is_finite() {
            seed_sum += v;
            seed_n += 1;
            if seed_n < period {
                continue;
            }
            last = seed_sum / period as f64;
        } else {
            last = (last * (period as f64 - 1.0) + v) / period as f64;
        }
        out[i] = last;
    }
    out
}

pub fn rsi_wilder(closes: &[f64], period: usize) -> Vec<f64> {
    let n = closes.len();
    let mut g = vec![f64::NAN; n];
    let mut l = vec![f64::NAN; n];
    for i in 1..n {
        let d = closes[i] - closes[i - 1];
        g[i] = d.max(0.0);
        l[i] = (-d).max(0.0);
    }
    let ag = rma_wilder(&g, period);
    let al = rma_wilder(&l, period);
    (0..n)
        .map(|i| {
            let (a, b) = (ag[i], al[i]);
            if !a.is_finite() || !b.is_finite() || (a == 0.0 && b == 0.0) {
                f64::NAN
            } else if b == 0.0 {
                100.0
            } else {
                100.0 - 100.0 / (1.0 + a / b)
            }
        })
        .collect()
}

pub fn true_range(b: &[Bar]) -> Vec<f64> {
    (0..b.len())
        .map(|i| {
            let pc = if i == 0 { b[i].close } else { b[i - 1].close };
            (b[i].high - b[i].low).max((b[i].high - pc).abs()).max((b[i].low - pc).abs())
        })
        .collect()
}

pub fn atr(b: &[Bar], period: usize) -> Vec<f64> {
    rma_wilder(&true_range(b), period)
}

pub fn adx(b: &[Bar], period: usize) -> Vec<f64> {
    let n = b.len();
    let mut pdm = vec![f64::NAN; n];
    let mut mdm = vec![f64::NAN; n];
    for i in 1..n {
        let up = b[i].high - b[i - 1].high;
        let dn = b[i - 1].low - b[i].low;
        pdm[i] = if up > dn && up > 0.0 { up } else { 0.0 };
        mdm[i] = if dn > up && dn > 0.0 { dn } else { 0.0 };
    }
    let mut tr = true_range(b);
    if n > 0 {
        tr[0] = f64::NAN;
    }
    let (st, sp, sm) = (rma_wilder(&tr, period), rma_wilder(&pdm, period), rma_wilder(&mdm, period));
    let mut dx = vec![f64::NAN; n];
    for i in 0..n {
        if st[i].is_finite() && st[i] > 0.0 && sp[i].is_finite() && sm[i].is_finite() {
            let p = 100.0 * sp[i] / st[i];
            let m = 100.0 * sm[i] / st[i];
            if p + m > 0.0 {
                dx[i] = 100.0 * (p - m).abs() / (p + m);
            }
        }
    }
    rma_wilder(&dx, period)
}

/// Efficiency Ratio Kaufman: |C_t − C_{t−n}| / Σ|ΔC| (RC-2, MR-01).
pub fn efficiency_ratio(c: &[f64], n: usize) -> Vec<f64> {
    let mut out = vec![f64::NAN; c.len()];
    for i in n..c.len() {
        let path: f64 = (i + 1 - n..=i).map(|k| (c[k] - c[k - 1]).abs()).sum();
        out[i] = if path > 0.0 { (c[i] - c[i - n]).abs() / path } else { 0.0 };
    }
    out
}

/// Choppiness Index (RC-3): 100·log10(ΣTR / (HH−LL)) / log10(n).
pub fn chop(b: &[Bar], n: usize) -> Vec<f64> {
    let tr = true_range(b);
    let mut out = vec![f64::NAN; b.len()];
    for i in n..b.len() {
        let s: f64 = tr[i + 1 - n..=i].iter().sum();
        let hh = b[i + 1 - n..=i].iter().map(|x| x.high).fold(f64::MIN, f64::max);
        let ll = b[i + 1 - n..=i].iter().map(|x| x.low).fold(f64::MAX, f64::min);
        if hh > ll {
            out[i] = 100.0 * (s / (hh - ll)).log10() / (n as f64).log10();
        }
    }
    out
}

/// Persentase rank nilai ke-i terhadap `window` nilai sebelumnya (0..100).
pub fn percent_rank(v: &[f64], window: usize) -> Vec<f64> {
    let mut out = vec![f64::NAN; v.len()];
    for i in window..v.len() {
        if !v[i].is_finite() {
            continue;
        }
        let w = &v[i - window..i];
        let cnt = w.iter().filter(|x| x.is_finite()).count();
        if cnt == 0 {
            continue;
        }
        let le = w.iter().filter(|x| x.is_finite() && **x <= v[i]).count();
        out[i] = 100.0 * le as f64 / cnt as f64;
    }
    out
}

/// Persentil (linear) dari slice.
pub fn percentile(v: &[f64], p: f64) -> f64 {
    let mut s: Vec<f64> = v.iter().copied().filter(|x| x.is_finite()).collect();
    if s.is_empty() {
        return f64::NAN;
    }
    s.sort_by(|a, b| a.total_cmp(b));
    let rank = (p / 100.0) * (s.len() - 1) as f64;
    let lo = rank.floor() as usize;
    let hi = rank.ceil() as usize;
    s[lo] + (s[hi] - s[lo]) * (rank - lo as f64)
}

/// Agregasi bar ke TF lebih tinggi, selaras 00:00 UTC. Hanya grup LENGKAP yang dikeluarkan.
pub fn aggregate(b: &[Bar], from: Tf, to: Tf) -> Vec<Bar> {
    let k = (to.ms() / from.ms()) as usize;
    let mut out = Vec::new();
    let mut i = 0;
    while i < b.len() {
        let start: Ms = b[i].open_time - b[i].open_time.rem_euclid(to.ms());
        if b[i].open_time != start || i + k > b.len() || b[i + k - 1].open_time != start + to.ms() - from.ms() {
            i += 1;
            continue;
        }
        let g = &b[i..i + k];
        out.push(Bar {
            open_time: start,
            open: g[0].open,
            high: g.iter().map(|x| x.high).fold(f64::MIN, f64::max),
            low: g.iter().map(|x| x.low).fold(f64::MAX, f64::min),
            close: g[k - 1].close,
            volume: g.iter().map(|x| x.volume).sum(),
            quote_volume: g.iter().map(|x| x.quote_volume).sum(),
            trades: g.iter().map(|x| x.trades).sum(),
            taker_buy_base: g.iter().map(|x| x.taker_buy_base).sum(),
        });
        i += k;
    }
    out
}

/// Pivot terkonfirmasi. `idx` = bar pivot; `confirmed_idx` = idx + right (baru boleh dipakai di close bar itu).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Pivot {
    pub idx: usize,
    pub confirmed_idx: usize,
    pub price: f64,
    pub high: bool,
}

pub fn pivots(b: &[Bar], left: usize, right: usize) -> Vec<Pivot> {
    let mut out = Vec::new();
    if b.len() < left + right + 1 {
        return out;
    }
    for i in left..b.len() - right {
        let h = b[i].high;
        let l = b[i].low;
        let is_h = (i - left..i).all(|k| b[k].high < h) && (i + 1..=i + right).all(|k| b[k].high <= h);
        let is_l = (i - left..i).all(|k| b[k].low > l) && (i + 1..=i + right).all(|k| b[k].low >= l);
        if is_h {
            out.push(Pivot { idx: i, confirmed_idx: i + right, price: h, high: true });
        }
        if is_l {
            out.push(Pivot { idx: i, confirmed_idx: i + right, price: l, high: false });
        }
    }
    out
}

/// Volume profile range tertutup: (POC, VAH, VAL) dengan `rows` baris dan value area `va` (0,7).
pub fn volume_profile(b: &[Bar], rows: usize, va: f64) -> Option<(f64, f64, f64)> {
    if b.is_empty() || rows == 0 {
        return None;
    }
    let hi = b.iter().map(|x| x.high).fold(f64::MIN, f64::max);
    let lo = b.iter().map(|x| x.low).fold(f64::MAX, f64::min);
    if hi <= lo {
        return None;
    }
    let h = (hi - lo) / rows as f64;
    let mut vol = vec![0.0; rows];
    for x in b {
        let a = (((x.low - lo) / h).floor() as usize).min(rows - 1);
        let z = (((x.high - lo) / h).floor() as usize).min(rows - 1);
        let per = x.volume / (z - a + 1) as f64;
        for v in vol.iter_mut().take(z + 1).skip(a) {
            *v += per;
        }
    }
    let poc = (0..rows).max_by(|&i, &j| vol[i].total_cmp(&vol[j]))?;
    let total: f64 = vol.iter().sum();
    let (mut lo_i, mut hi_i, mut acc) = (poc, poc, vol[poc]);
    while acc < va * total && (lo_i > 0 || hi_i < rows - 1) {
        let dn = if lo_i > 0 { vol[lo_i - 1] } else { -1.0 };
        let up = if hi_i < rows - 1 { vol[hi_i + 1] } else { -1.0 };
        if up >= dn {
            hi_i += 1;
            acc += up;
        } else {
            lo_i -= 1;
            acc += dn;
        }
    }
    Some((lo + (poc as f64 + 0.5) * h, lo + (hi_i as f64 + 1.0) * h, lo + lo_i as f64 * h))
}

#[cfg(test)]
mod tests {
    use super::*;

    pub fn synth(n: usize, seed: u64) -> Vec<Bar> {
        let mut s = seed;
        let mut p = 100.0;
        (0..n)
            .map(|i| {
                s ^= s << 13;
                s ^= s >> 7;
                s ^= s << 17;
                let r = ((s >> 11) as f64 / (1u64 << 53) as f64 - 0.5) * 0.02;
                let o = p;
                p *= 1.0 + r;
                let v = 1000.0 + (s % 500) as f64;
                Bar { open_time: i as i64 * 3_600_000, open: o, high: o.max(p) * 1.002, low: o.min(p) * 0.998, close: p, volume: v, quote_volume: v * p, trades: 100, taker_buy_base: v * 0.5 }
            })
            .collect()
    }

    #[test]
    fn rsi_seed_and_flat() {
        let prices: Vec<f64> = (1..=50).map(|x| x as f64).collect();
        let r = rsi_wilder(&prices, 14);
        assert!(r[..14].iter().all(|x| x.is_nan()));
        assert!(r[14..].iter().all(|x| (*x - 100.0).abs() < 1e-12));
        assert!(rsi_wilder(&[1.0; 50], 14).iter().all(|x| x.is_nan()));
    }

    #[test]
    fn er_trend_is_one() {
        let c: Vec<f64> = (0..40).map(|x| x as f64).collect();
        assert!((efficiency_ratio(&c, 20)[30] - 1.0).abs() < 1e-12);
    }

    /// Prefix test (NFR-18): nilai indikator di i dari data terpotong == dari data penuh.
    #[test]
    fn indicators_are_causal_prefix() {
        let b = synth(600, 7);
        let c = closes(&b);
        let full = (atr(&b, 14), adx(&b, 14), rsi_wilder(&c, 14), chop(&b, 14), efficiency_ratio(&c, 20), srv(&c, 30, 3).0, bilateral_state(&b, 60, 12.0, 1.5, 1.75), decycler_state(&b, 115, 14, 0.1, 3, 0.05, 0.5, 0.45));
        for cut in [150usize, 333, 480, 599] {
            let p = &b[..=cut];
            let pc = &c[..=cut];
            let eqn = |a: f64, z: f64| (a.is_nan() && z.is_nan()) || (a - z).abs() < 1e-9;
            assert!(eqn(atr(p, 14)[cut], full.0[cut]));
            assert!(eqn(adx(p, 14)[cut], full.1[cut]));
            assert!(eqn(rsi_wilder(pc, 14)[cut], full.2[cut]));
            assert!(eqn(chop(p, 14)[cut], full.3[cut]));
            assert!(eqn(efficiency_ratio(pc, 20)[cut], full.4[cut]));
            assert!(eqn(srv(pc, 30, 3).0[cut], full.5[cut]));
            assert_eq!(bilateral_state(p, 60, 12.0, 1.5, 1.75)[cut], full.6[cut]);
            assert_eq!(decycler_state(p, 115, 14, 0.1, 3, 0.05, 0.5, 0.45)[cut], full.7[cut]);
        }
    }

    #[test]
    fn aggregate_4h_aligned() {
        let b = synth(30, 3);
        let a = aggregate(&b[1..], Tf::H1, Tf::H4);
        assert!(a.iter().all(|x| x.open_time % Tf::H4.ms() == 0));
        assert_eq!(a.len(), 6); // bar 4..28 (grup pertama tidak lengkap dibuang)
    }

    #[test]
    fn pivots_confirmed_later() {
        let b = synth(200, 11);
        for p in pivots(&b, 3, 3) {
            assert_eq!(p.confirmed_idx, p.idx + 3);
        }
    }

    #[test]
    fn vp_va_contains_poc() {
        let b = synth(100, 5);
        let (poc, vah, val) = volume_profile(&b, 24, 0.7).unwrap();
        assert!(val <= poc && poc <= vah);
    }
}
