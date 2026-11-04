//! Implementasi S1–S5 (logika LONG; short via seri cermin). Semua indeks ≤ t (View terpotong).
//! Penyederhanaan yang jujur ditandai `// SIMPLIFIED:`.
use crate::{RawSignal, StrategyParams, View};
use apex_core::types::{Bar, Tf};

fn fin(x: f64) -> bool {
    x.is_finite()
}

fn sl_ok(entry: f64, sl: f64, p: &StrategyParams) -> bool {
    let d = (entry - sl) / entry;
    sl < entry && d >= p.sl_min_pct && d <= p.sl_max_pct
}

/// OFCC "dorongan jual": delta ≤ −P90(|delta|) dan close di 25% bawah range.
fn sell_push(v: &View, i: usize) -> bool {
    let p90 = v.f15.p90_absdelta[i];
    fin(p90) && v.f15.delta[i] <= -p90 && v.m15[i].close_pos() <= 0.25
}

/// S1 Trend FVG Pullback.
pub fn s1_tfp(v: &View, p: &StrategyParams) -> Option<RawSignal> {
    let t = v.t();
    if t < 3 || !v.trend_ok {
        return None;
    }
    let (atr, srv) = (v.f15.atr[t], v.f15.srv[t]);
    if !fin(atr) || !fin(srv) || srv < p.s1_srv_min {
        return None;
    }
    let (b0, b1, b2) = (&v.m15[t - 2], &v.m15[t - 1], &v.m15[t]);
    let gap = b2.low - b0.high;
    if gap <= 0.0 || gap < p.s1_gap_atr * atr {
        return None;
    }
    if b1.close <= b1.open || b1.range() <= 0.0 || b1.body() / b1.range() < p.s1_body_frac || v.f15.delta[t - 1] <= 0.0 {
        return None;
    }
    let entry = (b2.low + b0.high) / 2.0;
    let mut base = b0.low.min(b1.low).min(b2.low);
    if let Some(pv) = v.piv15_3.iter().rev().find(|x| !x.high && x.price < entry) {
        base = base.min(pv.price);
    }
    let sl = base - p.sl_buffer_atr * atr;
    if !sl_ok(entry, sl, p) {
        return None;
    }
    Some(RawSignal { entry, sl, market: false, ttl: p.s1_ttl, cancel_beyond: Some(b0.high), note: format!("FVG gap={:.3}ATR", gap / atr) })
}

/// Pool likuiditas (pivot) yang belum tersapu sampai indeks `upto` (eksklusif).
fn unswept(v: &View, high: bool, upto: usize, min_age: usize) -> Vec<(usize, f64)> {
    v.piv15_3
        .iter()
        .filter(|x| x.high == high && x.confirmed_idx < upto && upto - x.idx >= min_age)
        .filter(|x| {
            (x.confirmed_idx + 1..upto).all(|k| if high { v.m15[k].high <= x.price } else { v.m15[k].low >= x.price })
        })
        .map(|x| (x.idx, x.price))
        .collect()
}

/// S2 Liquidity Sweep Reclaim.
pub fn s2_lsr(v: &View, p: &StrategyParams) -> Option<RawSignal> {
    let t = v.t();
    if t < 30 || !v.trend_ok {
        return None;
    }
    let atr = v.f15.atr[t];
    if !fin(atr) {
        return None;
    }
    // sweep di s ∈ [t−reclaim_bars+1, t], reclaim pertama di t.
    for s in (t + 1 - p.s2_reclaim_bars.min(t))..=t {
        let pools = unswept(v, false, s, p.s2_pool_age);
        let Some(&(_, pool)) = pools.iter().filter(|(_, px)| v.m15[s].low < *px).max_by(|a, b| a.1.total_cmp(&b.1)) else { continue };
        if v.m15[t].close < pool || (s..t).any(|k| v.m15[k].close >= pool) {
            continue; // bukan reclaim pertama
        }
        let trap = (s..=(s + 1).min(t)).any(|k| {
            let b = &v.m15[k];
            let p95 = v.f15.p95_absdelta[k];
            (fin(p95) && v.f15.delta[k] <= -p95 && b.close_pos() >= 0.6) || (b.close > b.open && v.f15.delta[k] < 0.0)
        });
        if !trap {
            continue;
        }
        let rb = &v.m15[t];
        let entry = (rb.open + rb.close) / 2.0;
        let sweep_low = v.m15[s..=t].iter().map(|b| b.low).fold(f64::MAX, f64::min);
        let sl = sweep_low - p.sl_buffer_atr * atr;
        if !sl_ok(entry, sl, p) {
            return None;
        }
        // Filter target: pool lawan (pivot high belum tersapu) di antara entry dan 1R → SKIP.
        let r1 = entry + (entry - sl);
        if unswept(v, true, t + 1, 0).iter().any(|(_, px)| *px > entry && *px < r1) {
            return None;
        }
        return Some(RawSignal { entry, sl, market: false, ttl: p.s2_ttl, cancel_beyond: None, note: format!("sweep pool={pool:.6}") });
    }
    None
}

/// Level kunci untuk vote (c): H/L/C hari UTC sebelumnya + harga pivot swing 15m.
fn key_levels(v: &View) -> Vec<f64> {
    let t = v.t();
    let day = apex_core::time::day_start(v.m15[t].open_time);
    let prev: Vec<&Bar> = v.m15.iter().rev().take(200).filter(|b| b.open_time >= day - 86_400_000 && b.open_time < day).collect();
    let mut lv = Vec::new();
    if !prev.is_empty() {
        lv.push(prev.iter().map(|b| b.high).fold(f64::MIN, f64::max));
        lv.push(prev.iter().map(|b| b.low).fold(f64::MAX, f64::min));
        lv.push(prev[0].close); // rev → elemen pertama = bar terakhir hari itu
    }
    lv.extend(v.piv15_3.iter().rev().take(20).map(|x| x.price));
    lv
}

/// S3 Structure Break Retest.
pub fn s3_sbr(v: &View, p: &StrategyParams) -> Option<RawSignal> {
    let t = v.t();
    let h = v.h()?;
    if t < 40 || !v.trend_ok || v.f1.dec[h] != 1 {
        return None;
    }
    let atr = v.f15.atr[t];
    if !fin(atr) {
        return None;
    }
    let rt = &v.m15[t];
    for k in (t.saturating_sub(p.s3_retest_bars))..t {
        // pivot high 5/5 terkonfirmasi sebelum k, belum ditembus close sebelum k
        let Some(pv) = v.piv15_5.iter().rev().find(|x| x.high && x.confirmed_idx < k) else { continue };
        let level = pv.price;
        if v.m15[k].close <= level || (pv.confirmed_idx + 1..k).any(|j| v.m15[j].close > level) {
            continue;
        }
        let (ak, vs) = (v.f15.atr[k], v.f15.vol_sma20[k]);
        if !fin(ak) || !fin(vs) || vs <= 0.0 {
            continue;
        }
        // SIMPLIFIED: SBPO = (close−break)/ATR × RVOL (bukan port 1:1 skrip tertutup).
        let sbpo = (v.m15[k].close - level) / ak * (v.m15[k].volume / vs);
        if sbpo < p.s3_sbpo_min {
            continue;
        }
        let is_retest = |b: &Bar| b.low <= level + 0.25 * atr && b.close >= level && b.close > b.open;
        if !is_retest(rt) || (k + 1..t).any(|j| is_retest(&v.m15[j])) {
            continue;
        }
        let mut votes = 0;
        if v.f15.pressure[t] >= 60.0 {
            votes += 1; // SIMPLIFIED Pressure Radar = percent-rank delta 3 bar
        }
        if v.f15.delta[t] + v.f15.delta[t - 1] + v.f15.delta[t - 2] > 0.0 {
            votes += 1;
        }
        if key_levels(v).iter().filter(|x| (*x - level).abs() <= 0.6 * atr && (**x - level).abs() > 0.0).count() >= 2 {
            votes += 1;
        }
        if fin(v.f15.mso[t]) && v.f15.mso[t] > v.f15.mso_sig[t] {
            votes += 1; // SIMPLIFIED MSO proxy
        }
        if votes < 2 {
            return None;
        }
        let low = v.m15[k + 1..=t].iter().map(|b| b.low).fold(f64::MAX, f64::min);
        let sl = low - p.sl_buffer_atr * atr;
        let entry = if p.s3_market { rt.close } else { (rt.open + rt.close) / 2.0 };
        if !sl_ok(entry, sl, p) {
            return None;
        }
        return Some(RawSignal { entry, sl, market: p.s3_market, ttl: p.s3_ttl, cancel_beyond: None, note: format!("break={level:.6} votes={votes} sbpo={sbpo:.2}") });
    }
    None
}

/// S4 Wyckoff Spring/LPS. SIMPLIFIED: range = jendela 1h 10–60 bar sideways (ER ≤ 0,35, tinggi ≥ 4%, tidak ada close di atas AR);
/// SC/AR/ST tidak dilabeli satu per satu.
pub fn s4_wsl(v: &View, p: &StrategyParams) -> Option<RawSignal> {
    let t = v.t();
    let h = v.h()?;
    if !v.trend_ok || h < p.s4_max_range + 25 {
        return None;
    }
    let atr15 = v.f15.atr[t];
    let lps = &v.m15[t];
    if !fin(atr15) || sell_push(v, t) {
        return None;
    }
    let sos_close_ms = |j: usize| v.h1[j].open_time + Tf::H1.ms();
    // SOS dalam ≤ 24 bar 15m sebelum t (6 jam).
    for j in (h.saturating_sub(6)..=h).rev() {
        let sb = &v.h1[j];
        let (a1, vs) = (v.f1.atr[j], v.f1.vol_sma20[j.saturating_sub(1)]);
        if !fin(a1) || !fin(vs) || sb.delta() <= 0.0 || sb.volume < 1.5 * vs {
            continue;
        }
        let first15 = v.m15.partition_point(|b| b.open_time < sos_close_ms(j));
        if first15 > t || t - first15 >= p.s4_lps_bars {
            continue;
        }
        // cari range sebelum SOS
        let mut found = None;
        for len in p.s4_min_range..=p.s4_max_range {
            let rg = &v.h1[j - len..j];
            let ar = rg.iter().map(|b| b.high).fold(f64::MIN, f64::max);
            let lo = rg.iter().map(|b| b.low).fold(f64::MAX, f64::min);
            if (ar - lo) / lo < p.s4_min_height || rg.iter().any(|b| b.close > ar) {
                continue;
            }
            let c: Vec<f64> = rg.iter().map(|b| b.close).collect();
            let path: f64 = c.windows(2).map(|w| (w[1] - w[0]).abs()).sum();
            let er = if path > 0.0 { (c[c.len() - 1] - c[0]).abs() / path } else { 1.0 };
            if er <= 0.35 {
                found = Some((len, ar, lo));
                break;
            }
        }
        let Some((len, ar, lo)) = found else { continue };
        if sb.close <= ar + 0.2 * a1 {
            continue;
        }
        let (_poc, vah, val) = apex_indicators::volume_profile(&v.h1[j - len..j], 24, 0.7)?;
        let in_zone = |b: &Bar| (b.low >= ar - 0.3 * a1 && b.low <= ar + 0.3 * a1) || b.low <= vah.max(ar);
        if !in_zone(lps) || (first15..t).any(|k| in_zone(&v.m15[k])) {
            continue; // hanya LPS pertama
        }
        let entry = ar.max(vah);
        let c1 = lps.low - p.sl_buffer_atr * atr15;
        let c2 = val - p.sl_buffer_atr * a1;
        let sl = [c1, c2].into_iter().filter(|x| *x < lps.low).fold(f64::MIN, f64::max);
        if !sl_ok(entry, sl, p) {
            return None;
        }
        if ar + (ar - lo) < entry + 2.0 * (entry - sl) {
            return None; // proyeksi range < 2R
        }
        return Some(RawSignal { entry, sl, market: false, ttl: p.s4_ttl, cancel_beyond: None, note: format!("range {len}h AR={ar:.6}") });
    }
    None
}

/// Proxy Magnet: pool pivot high terkonfirmasi, berbobot volume dan digabung
/// dalam 0,3 ATR. Pool kedaluwarsa setelah 200 bar tanpa pivot baru.
/// Ini adaptasi dari riset lokal, bukan parity penuh indikator TradingView.
fn opposing_magnet_blocks(v: &View, entry: f64, one_r: f64) -> bool {
    struct Pool { price: f64, weight: f64, refreshed: usize }
    let t = v.t();
    let mut pools: Vec<Pool> = Vec::new();
    for pivot in v.piv15_3.iter().filter(|x| x.high && x.confirmed_idx <= t && t - x.confirmed_idx <= 200) {
        let now = pivot.confirmed_idx;
        pools.retain(|p| now.saturating_sub(p.refreshed) <= 200);
        let atr = v.f15.atr[now];
        if !fin(atr) || atr <= 0.0 { continue; }
        let avg = v.f15.vol_sma20[pivot.idx];
        let volume = v.m15[pivot.idx].volume;
        let weight = if fin(avg) && avg > 0.0 && fin(volume) && volume > 0.0 { (volume / avg).min(4.0) } else { 1.0 };
        if let Some(pool) = pools.iter_mut().find(|p| (p.price - pivot.price).abs() <= 0.3 * atr) {
            pool.price = (pool.price * pool.weight + pivot.price * weight) / (pool.weight + weight);
            pool.weight += weight;
            pool.refreshed = now;
        } else {
            pools.push(Pool { price: pivot.price, weight, refreshed: now });
            if pools.len() > 32 { pools.remove(0); }
        }
    }
    pools.iter().any(|p| t.saturating_sub(p.refreshed) <= 200 && p.price >= entry && p.price <= one_r)
}

/// Compute the real-price 1R target before transforming a short into mirror space.
fn one_r_in_side_space(side: apex_core::types::Side, entry: f64, sl: f64) -> Option<f64> {
    match side {
        apex_core::types::Side::Long => Some(entry + (entry - sl)),
        apex_core::types::Side::Short => {
            let target = 2.0 / entry - 1.0 / sl;
            if target > 0.0 && target.is_finite() { Some(1.0 / target) } else { None }
        }
    }
}

/// S5 Regime Momentum Pullback. kNN (varian K) belum diimplementasi.
pub fn s5_rmp(v: &View, p: &StrategyParams) -> Option<RawSignal> {
    let t = v.t();
    let h = v.h()?;
    if !v.trend_ok || t < 30 || h < 210 {
        return None;
    }
    let f = &v.f1;
    if !(f.dist_rank[h] >= p.s5_distrank_min && f.apexwave[h] >= p.s5_apexwave_min && f.ema20[h] > f.ema50[h] && f.ema50[h] > f.ema200[h] && f.ema50[h] > f.ema50[h - 5]) {
        return None;
    }
    let hi = v.piv1h_3.iter().rev().find(|x| x.high)?;
    let lo = v.piv1h_3.iter().rev().find(|x| !x.high && x.idx < hi.idx)?;
    let d = hi.price - lo.price;
    let a1 = f.atr[hi.idx];
    if !fin(a1) || d < p.s5_impulse_atr * a1 {
        return None;
    }
    let start = v.m15.partition_point(|b| b.open_time < v.h1[hi.idx].open_time + Tf::H1.ms());
    if start + 2 > t {
        return None;
    }
    let pb = &v.m15[start..t];
    let pb_low = pb.iter().map(|b| b.low).fold(f64::MAX, f64::min);
    if pb_low > hi.price - 0.5 * d || pb_low < hi.price - 0.786 * d {
        return None;
    }
    let rsi_min = v.f15.rsi[start..t].iter().copied().filter(|x| fin(*x)).fold(f64::MAX, f64::min);
    if !(35.0..=50.0).contains(&rsi_min) {
        return None;
    }
    let (b, pb_prev) = (&v.m15[t], &v.m15[t - 1]);
    let (e, ep) = (v.f15.ema21[t], v.f15.ema21[t - 1]);
    let ba = v.f15.body_avg20[t];
    if !(b.close > e && pb_prev.close <= ep && b.close > b.open && fin(ba) && b.body() >= 1.2 * ba) {
        return None;
    }
    let atr = v.f15.atr[t];
    let entry = (b.open + b.close) / 2.0;
    let mut sl = pb_low - p.sl_buffer_atr * atr;
    if (entry - sl) / entry < p.sl_min_pct {
        let alt = hi.price - 0.786 * d - p.sl_buffer_atr * atr;
        if sl_ok(entry, alt, p) {
            sl = alt;
        } else {
            return None;
        }
    }
    if !sl_ok(entry, sl, p) {
        return None;
    }
    let one_r = one_r_in_side_space(v.side, entry, sl)?;
    if opposing_magnet_blocks(v, entry, one_r) {
        return None;
    }
    Some(RawSignal { entry, sl, market: false, ttl: p.s5_ttl, cancel_beyond: None, note: format!("impulse {:.1}ATR", d / a1) })
}

#[cfg(test)]
mod tests {
    use super::{one_r_in_side_space, opposing_magnet_blocks};
    use crate::*;
    use apex_core::types::Bar;
    use apex_screener::SymState;

    fn synth(n: usize, seed: u64, tf_ms: i64, drift: f64) -> Vec<Bar> {
        let mut s = seed;
        let mut p = 50.0;
        (0..n)
            .map(|i| {
                s ^= s << 13;
                s ^= s >> 7;
                s ^= s << 17;
                let u = (s >> 11) as f64 / (1u64 << 53) as f64;
                let r = (u - 0.5) * 0.02 + drift + if i % 97 < 4 { 0.01 } else { 0.0 };
                let o = p;
                p *= 1.0 + r;
                let v = 1000.0 + (s % 900) as f64;
                let tb = v * (0.3 + 0.4 * ((s >> 20) % 100) as f64 / 100.0);
                Bar { open_time: i as i64 * tf_ms, open: o, high: o.max(p) * (1.0 + u * 0.006), low: o.min(p) * (1.0 - (1.0 - u) * 0.006), close: p, volume: v, quote_volume: v * p, trades: 100, taker_buy_base: tb }
            })
            .collect()
    }

    fn series(n15: usize) -> (Vec<Bar>, Vec<Bar>) {
        let m15 = synth(n15, 42, 900_000, 0.0004);
        let h1 = apex_indicators::aggregate(&m15, apex_core::types::Tf::M15, apex_core::types::Tf::H1);
        (m15, h1)
    }

    fn all_signals(sym: &SymbolSeries, from: usize, to: usize) -> Vec<(usize, Signal)> {
        let p = StrategyParams::default();
        let mut out = vec![];
        for t in from..to {
            for st in [SymState::TrendUp, SymState::TrendDown] {
                for s in evaluate(sym, t, st, &StrategyId::ALL, &p) {
                    out.push((t, s));
                }
            }
        }
        out
    }

    /// No-look-ahead: sinyal di t dari data terpotong (≤ t) == dari data penuh, dan
    /// mutasi data masa depan (> t) tidak mengubah sinyal ≤ t.
    #[test]
    fn strategies_no_lookahead_prefix_and_future_mutation() {
        let (m15, h1) = series(6000);
        let full = SymbolSeries::new("TUSDT", &m15, &h1);
        let cut = 4800;
        let sigs_full: Vec<_> = all_signals(&full, 3000, cut + 1);
        // prefix: hanya data sampai close bar `cut`
        let h_cut: Vec<Bar> = h1.iter().copied().filter(|b| b.open_time + 3_600_000 <= m15[cut].open_time + 900_000).collect();
        let pre = SymbolSeries::new("TUSDT", &m15[..=cut], &h_cut);
        let sigs_pre = all_signals(&pre, 3000, cut + 1);
        assert_eq!(sigs_full, sigs_pre);
        // future mutation
        let mut m2 = m15.clone();
        for b in m2.iter_mut().skip(cut + 1) {
            b.close *= 1.3;
            b.high *= 1.4;
            b.low *= 0.7;
            b.volume *= 5.0;
        }
        let h2 = apex_indicators::aggregate(&m2, apex_core::types::Tf::M15, apex_core::types::Tf::H1);
        let mutated = SymbolSeries::new("TUSDT", &m2, &h2);
        assert_eq!(all_signals(&mutated, 3000, cut + 1), sigs_full);
    }

    #[test]
    fn deterministic_and_well_formed() {
        let (m15, h1) = series(5000);
        let a = SymbolSeries::new("TUSDT", &m15, &h1);
        let s1 = all_signals(&a, 2000, 5000);
        let s2 = all_signals(&a, 2000, 5000);
        assert_eq!(s1, s2);
        assert!(!s1.is_empty(), "sintetis harus menghasilkan ≥1 sinyal");
        for (_, s) in &s1 {
            match s.side {
                apex_core::types::Side::Long => assert!(s.sl < s.entry),
                apex_core::types::Side::Short => assert!(s.sl > s.entry),
            }
            assert!(s.sl_pct() > 0.0195 && s.sl_pct() < 0.0405, "{s:?}");
        }
    }

    #[test]
    fn regime_gate_blocks_everything() {
        let (m15, h1) = series(4000);
        let a = SymbolSeries::new("TUSDT", &m15, &h1);
        let p = StrategyParams::default();
        for t in 1000..4000 {
            assert!(evaluate(&a, t, SymState::Ranging, &StrategyId::ALL, &p).is_empty());
            assert!(evaluate(&a, t, SymState::Blocked, &StrategyId::ALL, &p).is_empty());
        }
    }

    #[test]
    fn magnet_requires_confirmed_opposing_pool_and_expires() {
        let mut bars: Vec<Bar> = (0..800).map(|i| Bar {
            open_time: i * 900_000, open:100.0, high:101.0, low:99.0, close:100.0,
            volume:100.0, quote_volume:10000.0, trades:100, taker_buy_base:50.0,
        }).collect();
        bars[510].high = 105.0;
        let h = apex_indicators::aggregate(&bars, apex_core::types::Tf::M15, apex_core::types::Tf::H1);
        let series = SymbolSeries::new("ETHUSDT", &bars, &h);
        let state = SymState::TrendUp;
        assert!(!opposing_magnet_blocks(&make_view(&series, &series.long, 512, state), 100.0, 106.0));
        assert!(opposing_magnet_blocks(&make_view(&series, &series.long, 513, state), 100.0, 106.0));
        assert!(!opposing_magnet_blocks(&make_view(&series, &series.long, 513, state), 100.0, 104.0));
        assert!(!opposing_magnet_blocks(&make_view(&series, &series.long, 714, state), 100.0, 106.0));

        // A heavier first pivot shifts a merged pool below 105.1.
        bars[510].volume = 300.0;
        bars[520].high = 105.2;
        let h = apex_indicators::aggregate(&bars, apex_core::types::Tf::M15, apex_core::types::Tf::H1);
        let weighted = SymbolSeries::new("ETHUSDT", &bars, &h);
        assert!(opposing_magnet_blocks(&make_view(&weighted, &weighted.long, 523, state), 100.0, 105.1));
    }

    #[test]
    fn short_magnet_target_uses_real_price_one_r() {
        let target = one_r_in_side_space(apex_core::types::Side::Short, 1.0 / 100.0, 1.0 / 103.0).unwrap();
        assert!((1.0 / target - 97.0).abs() < 1e-10);
    }

    #[test]
    fn mirror_is_involution() {
        let b = Bar { open_time: 0, open: 2.0, high: 4.0, low: 1.0, close: 3.0, volume: 10.0, quote_volume: 0.0, trades: 1, taker_buy_base: 7.0 };
        let m = mirror_bar(&b);
        assert!(m.high >= m.low && (m.delta() + b.delta()).abs() < 1e-12);
        let mm = mirror_bar(&m);
        assert!((mm.high - b.high).abs() < 1e-12 && (mm.low - b.low).abs() < 1e-12);
    }
}
