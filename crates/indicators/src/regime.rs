//! Fitur regime yang di-port dari skrip Pine (versi bar tertutup, kausal).
use crate::{atr, ema, percentile};
use apex_core::types::Bar;

/// Signed Realized Variance (ylUDT58m): EMA(sign(r)·r²)/EMA(r²), lalu EMA(post).
/// Mengembalikan (rv_trend, state ±1 dengan flip threshold 0,10).
pub fn srv(c: &[f64], len: usize, post: usize) -> (Vec<f64>, Vec<i8>) {
    let n = c.len();
    let mut sq = vec![0.0; n];
    let mut ssq = vec![0.0; n];
    for i in 1..n {
        let r = if c[i] > 0.0 && c[i - 1] > 0.0 { (c[i] / c[i - 1]).ln() } else { 0.0 };
        sq[i] = r * r;
        ssq[i] = r.signum() * r * r;
    }
    let se = ema(&ssq, len);
    let te = ema(&sq, len);
    let raw: Vec<f64> = (0..n).map(|i| if te[i] > 0.0 { (se[i] / te[i]).clamp(-1.0, 1.0) } else { 0.0 }).collect();
    let rv = if post > 1 { ema(&raw, post) } else { raw };
    let mut st = vec![0i8; n];
    let mut s = 0i8;
    for i in 0..n {
        if s == 0 {
            s = if rv[i] >= 0.0 { 1 } else { -1 };
        } else if s != 1 && rv[i] > 0.10 {
            s = 1;
        } else if s != -1 && rv[i] < -0.10 {
            s = -1;
        }
        st[i] = s;
    }
    (rv, st)
}

/// Bilateral Filter Trend (yY27v3eD) state ±1/0. Preset Smooth = (60, 12, 1.5, 1.75). Source = close, spike rejection on.
pub fn bilateral_state(b: &[Bar], window: usize, tsig: f64, rsig_mult: f64, rev_mult: f64) -> Vec<i8> {
    let n = b.len();
    let a = atr(b, 14);
    let mut out = vec![0i8; n];
    let (mut st, mut peak, mut trough) = (0i8, f64::NAN, f64::NAN);
    for i in 0..n {
        let rs = rsig_mult * a[i];
        if !(rs.is_finite() && rs > 0.0) {
            out[i] = st;
            continue;
        }
        let src = b[i].close;
        let p1 = if i >= 1 { b[i - 1].close } else { src };
        let p2 = if i >= 2 { b[i - 2].close } else { p1 };
        let reference = src.min(p1).max(src.max(p1).min(p2));
        let span = window.min(i + 1);
        let (mut ws, mut vs) = (0.0, 0.0);
        for k in 0..span {
            let s = b[i - k].close;
            let tw = (-0.5 * (k * k) as f64 / (tsig * tsig)).exp();
            let d = (s - reference) / rs;
            let w = tw * (-0.5 * d * d).exp();
            ws += w;
            vs += w * s;
        }
        if ws <= 0.0 {
            out[i] = st;
            continue;
        }
        let f = vs / ws;
        let rd = rev_mult * rs;
        if st == 0 {
            peak = if peak.is_nan() { f } else { peak.max(f) };
            trough = if trough.is_nan() { f } else { trough.min(f) };
            if f >= trough + rd {
                st = 1;
                peak = f;
            } else if f <= peak - rd {
                st = -1;
                trough = f;
            }
        } else if st == 1 {
            peak = peak.max(f);
            if f <= peak - rd {
                st = -1;
                trough = f;
            }
        } else {
            trough = trough.min(f);
            if f >= trough + rd {
                st = 1;
                peak = f;
            }
        }
        out[i] = st;
    }
    out
}

fn decycler(src: &[f64], period: usize) -> Vec<f64> {
    let ang = 0.707 * 2.0 * std::f64::consts::PI / period as f64;
    let alpha = (ang.cos() + ang.sin() - 1.0) / ang.cos();
    let decay = 1.0 - alpha;
    let gain = (1.0 - alpha * 0.5).powi(2);
    let mut hp = vec![0.0; src.len()];
    let mut out = vec![0.0; src.len()];
    for i in 0..src.len() {
        hp[i] = if i >= 2 { gain * (src[i] - 2.0 * src[i - 1] + src[i - 2]) + 2.0 * decay * hp[i - 1] - decay * decay * hp[i - 2] } else { 0.0 };
        out[i] = src[i] - hp[i];
    }
    out
}

/// Decycler High-Low Breakout (dOcUNfyv) state ±1/0 (default 115/14/0.1/3/0.05/0.5/0.45).
#[allow(clippy::too_many_arguments)]
pub fn decycler_state(b: &[Bar], period: usize, atr_len: usize, pad: f64, slope_len: usize, slope_conf: f64, breathing: f64, overr: f64) -> Vec<i8> {
    let h: Vec<f64> = b.iter().map(|x| x.high).collect();
    let l: Vec<f64> = b.iter().map(|x| x.low).collect();
    let c: Vec<f64> = b.iter().map(|x| x.close).collect();
    let (dh, dl, dc) = (decycler(&h, period), decycler(&l, period), decycler(&c, period));
    let a = atr(b, atr_len);
    let warm = period.max(atr_len).max(slope_len);
    let mut out = vec![0i8; b.len()];
    let mut sq = 0i8;
    for i in 0..b.len() {
        if i < warm || !a[i].is_finite() {
            out[i] = sq;
            continue;
        }
        let up = dh[i].max(dl[i]).max(dc[i]);
        let lo = dh[i].min(dl[i]).min(dc[i]);
        let atrv = a[i].max(1e-12);
        let ns = (dc[i] - dc[i - slope_len]) / (atrv * (slope_len as f64).sqrt());
        let bs = ns.clamp(0.0, 2.0);
        let brs = (-ns).clamp(0.0, 2.0);
        let lower_def = lo - atrv * pad * (1.0 + breathing * bs);
        let upper_def = up + atrv * pad * (1.0 + breathing * brs);
        let bear_bo = lower_def - atrv * overr;
        let bull_bo = upper_def + atrv * overr;
        let bear_break = c[i] < lower_def && (ns < -slope_conf || c[i] < bear_bo);
        let bull_break = c[i] > upper_def && (ns > slope_conf || c[i] > bull_bo);
        if sq == 0 {
            sq = if c[i] >= dc[i] { 1 } else { -1 };
        } else if sq == -1 && bull_break {
            sq = 1;
        } else if sq == 1 && bear_break {
            sq = -1;
        }
        out[i] = sq;
    }
    out
}

/// Distributional Rank (gGTzHW8l): skor 0..100 posisi close dalam distribusi `lookback` close terakhir,
/// dihaluskan (Q25 + 2·Q50 + Q75)/4 atas `smooth` skor terakhir.
pub fn dist_rank(c: &[f64], lookback: usize, smooth: usize) -> Vec<f64> {
    let n = c.len();
    let mut raw = vec![f64::NAN; n];
    if lookback >= 2 {
        for i in (lookback - 1)..n {
            let mut w: Vec<f64> = c[i + 1 - lookback..=i].to_vec();
            w.sort_by(|a, b| a.total_cmp(b));
            let mut above = 0;
            for p in 1..=99 {
                let rank = (p as f64 / 100.0) * (w.len() - 1) as f64;
                let (lo, hi) = (rank.floor() as usize, rank.ceil() as usize);
                let q = w[lo] + (w[hi] - w[lo]) * (rank - lo as f64);
                if q < c[i] {
                    above += 1;
                }
            }
            raw[i] = above as f64 / 99.0 * 100.0;
        }
    }
    let mut out = vec![f64::NAN; n];
    for i in 0..n {
        if i + 1 < smooth {
            continue;
        }
        let w = &raw[i + 1 - smooth..=i];
        if w.iter().any(|x| x.is_nan()) {
            continue;
        }
        out[i] = (percentile(w, 25.0) + 2.0 * percentile(w, 50.0) + percentile(w, 75.0)) / 4.0;
    }
    out
}

fn tanh_c(x: f64) -> f64 {
    x.clamp(-20.0, 20.0).tanh()
}

/// ApexWave Live Sentiment (KduAs9nf) bull% (10..90), versi bar tertutup. Default 10/8/21/5/14, sens 1, smooth 3.
pub fn apexwave_bull(b: &[Bar]) -> Vec<f64> {
    let n = b.len();
    let c: Vec<f64> = b.iter().map(|x| x.close).collect();
    let a = atr(b, 14);
    let fast = ema(&c, 8);
    let slow = ema(&c, 21);
    let mom: Vec<f64> = (0..n).map(|i| if i >= 10 { c[i] - c[i - 10] } else { 0.0 }).collect();
    let mom_sm = ema(&mom, 8);
    let mut cyc = vec![0.0; n];
    for i in 0..n {
        let atr_s = if a[i].is_finite() { a[i].max(1e-12) } else { 1e-12 };
        let trend_n = (fast[i] - slow[i]) / atr_s;
        let mom_n = mom_sm[i] / (atr_s * 10f64.sqrt());
        let bounded = tanh_c((trend_n + mom_n) * 0.5 * 1.5);
        cyc[i] = 100.0 * (bounded * std::f64::consts::FRAC_PI_2).sin();
    }
    let osc = ema(&cyc, 5);
    let dir: Vec<f64> = b.iter().map(|x| if x.close > x.open { 1.0 } else if x.close < x.open { -1.0 } else { 0.0 }).collect();
    let s4 = crate::sma(&dir, 10);
    let mut raw = vec![50.0; n];
    for i in 0..n {
        let atr_s = if a[i].is_finite() { a[i].max(1e-12) } else { 1e-12 };
        let s1 = osc[i] / 100.0;
        let prev = if i >= 3 { osc[i - 3] } else { osc[i] };
        let s2 = tanh_c((osc[i] - prev) / 100.0 * 4.0);
        let s3 = tanh_c((c[i] - slow[i]) / atr_s);
        let s4v = if s4[i].is_finite() { s4[i] } else { 0.0 };
        let score = 0.40 * s1 + 0.20 * s2 + 0.25 * s3 + 0.15 * s4v;
        raw[i] = 50.0 + 50.0 * tanh_c(score * 1.5);
    }
    ema(&raw, 3).into_iter().map(|x| x.clamp(10.0, 90.0).round()).collect()
}

/// MTF trend vote (NjcGEJ94 versi bar tertutup): +1 bila EMA21>EMA55 dan close>EMA21, −1 sebaliknya, 0 campur.
pub fn mtf_vote(c: &[f64]) -> i8 {
    if c.len() < 60 {
        return 0;
    }
    let e21 = ema(c, 21);
    let e55 = ema(c, 55);
    let i = c.len() - 1;
    if e21[i] > e55[i] && c[i] > e21[i] {
        1
    } else if e21[i] < e55[i] && c[i] < e21[i] {
        -1
    } else {
        0
    }
}
