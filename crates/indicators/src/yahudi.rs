//! Port 1:1 mesin indikator & persepsi `Yahudi-Bot` (Mode SCALP):
//! - Zero-Lag EMA (`ZLEMA 21`) + Volatility Shield (`regimeTrendEngine.ts`)
//! - Klasifikasi Regime 5-bar mayoritas (`TRENDING` / `RANGING` / `VOLATILE` / `UNCERTAIN`)
//! - 5-Factor Zone Engine (`OB`, `FVG`, `DEMAND`, `SUPPLY`) + Squeeze Momentum BB-in-KC (`zoneTriggerEngine.ts`)
//! - Konfirmasi Momentum 5m Closed-Bar (`entryPolicy.ts`)
//! - Lorentzian k-NN Analog Engine 5D (`knnAnalogEngine.ts`)
//! - Persepsi Big Brain B4: Beta BTC, VWAP Sesi UTC 00:00, Kaufman ER(30) (`perceptionB4.ts`)

use apex_core::types::{Bar, Side};
use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum TrendDir {
    Bullish,
    Bearish,
    Neutral,
}

impl TrendDir {
    pub fn as_str(&self) -> &'static str {
        match self {
            TrendDir::Bullish => "BULLISH",
            TrendDir::Bearish => "BEARISH",
            TrendDir::Neutral => "NEUTRAL",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum MarketRegime {
    Trending,
    Ranging,
    Volatile,
    Uncertain,
}

impl MarketRegime {
    pub fn as_str(&self) -> &'static str {
        match self {
            MarketRegime::Trending => "TRENDING",
            MarketRegime::Ranging => "RANGING",
            MarketRegime::Volatile => "VOLATILE",
            MarketRegime::Uncertain => "UNCERTAIN",
        }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct VolatilityShieldStatus {
    pub active: bool,
    pub block_new_signals: bool,
    pub atr_ratio: f64,
    pub current_atr: f64,
    pub avg72_atr: f64,
    pub spread_pct: f64,
    pub reasons: Vec<&'static str>,
}

/// Seri ATR(14) Wilder persis seperti `calculateAtrSeries` di `regimeTrendEngine.ts`
/// (TR dimulai dari indeks 1, seed = SMA `period` TR pertama).
pub fn atr_series_yahudi(bars: &[Bar], period: usize) -> Vec<f64> {
    if period == 0 || bars.len() < period + 1 {
        return Vec::new();
    }
    let mut trs = Vec::with_capacity(bars.len() - 1);
    for i in 1..bars.len() {
        let h = bars[i].high;
        let l = bars[i].low;
        let pc = bars[i - 1].close;
        let tr = (h - l).max((h - pc).abs()).max((l - pc).abs());
        trs.push(tr);
    }
    if trs.len() < period {
        return Vec::new();
    }
    let mut atrs = Vec::with_capacity(trs.len() - period + 1);
    let sum_tr: f64 = trs[..period].iter().sum();
    let mut cur = sum_tr / period as f64;
    atrs.push(cur);
    for &tr in &trs[period..] {
        cur = (cur * (period as f64 - 1.0) + tr) / period as f64;
        atrs.push(cur);
    }
    atrs
}

pub fn atr_last_yahudi(bars: &[Bar], period: usize) -> Option<f64> {
    atr_series_yahudi(bars, period).last().copied()
}

/// Choppiness Index (14) persis `calculateChoppiness` di `eligibility.ts`.
pub fn choppiness_last_yahudi(bars: &[Bar], period: usize) -> Option<f64> {
    if period <= 1 || bars.len() < period {
        return None;
    }
    let start_idx = bars.len() - period;
    let slice = &bars[start_idx..];
    let mut sum_tr = 0.0;
    let mut max_h = f64::NEG_INFINITY;
    let mut min_l = f64::INFINITY;
    for (i, b) in slice.iter().enumerate() {
        let tr = if start_idx + i > 0 {
            let pc = bars[start_idx + i - 1].close;
            (b.high - b.low).max((b.high - pc).abs()).max((b.low - pc).abs())
        } else {
            b.high - b.low
        };
        sum_tr += tr;
        if b.high > max_h {
            max_h = b.high;
        }
        if b.low < min_l {
            min_l = b.low;
        }
    }
    let range = max_h - min_l;
    if range <= 0.0 || sum_tr <= 0.0 {
        return Some(100.0);
    }
    let ratio = sum_tr / range;
    if ratio <= 0.0 {
        return Some(100.0);
    }
    let chop = (100.0 * ratio.log10()) / (period as f64).log10();
    Some(chop.clamp(0.0, 100.0))
}

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct AdxSnapshot {
    pub adx: f64,
    pub plus_di: f64,
    pub minus_di: f64,
}

/// Wilder's ADX(14) persis `calculateAdx` di `eligibility.ts` (butuh minimal `2 * period` bar).
pub fn adx_last_yahudi(bars: &[Bar], period: usize) -> Option<AdxSnapshot> {
    let min_req = 2 * period;
    if period == 0 || bars.len() < min_req {
        return None;
    }
    let mut trs = Vec::with_capacity(bars.len() - 1);
    let mut pdms = Vec::with_capacity(bars.len() - 1);
    let mut mdms = Vec::with_capacity(bars.len() - 1);
    for i in 1..bars.len() {
        let curr = &bars[i];
        let prev = &bars[i - 1];
        let tr = (curr.high - curr.low)
            .max((curr.high - prev.close).abs())
            .max((curr.low - prev.close).abs());
        trs.push(tr);
        let up = curr.high - prev.high;
        let dn = prev.low - curr.low;
        pdms.push(if up > dn && up > 0.0 { up } else { 0.0 });
        mdms.push(if dn > up && dn > 0.0 { dn } else { 0.0 });
    }
    let mut smooth_tr: f64 = trs[..period].iter().sum();
    let mut smooth_p: f64 = pdms[..period].iter().sum();
    let mut smooth_m: f64 = mdms[..period].iter().sum();
    let calc_di_dx = |tr: f64, p: f64, m: f64| -> (f64, f64, f64) {
        let p_di = if tr > 0.0 { 100.0 * p / tr } else { 0.0 };
        let m_di = if tr > 0.0 { 100.0 * m / tr } else { 0.0 };
        let sum = p_di + m_di;
        let dx = if sum > 0.0 { (100.0 * (p_di - m_di).abs()) / sum } else { 0.0 };
        (p_di, m_di, dx)
    };
    let mut dxs = Vec::with_capacity(trs.len() - period + 1);
    let (mut last_p_di, mut last_m_di, dx0) = calc_di_dx(smooth_tr, smooth_p, smooth_m);
    dxs.push(dx0);
    for i in period..trs.len() {
        smooth_tr = smooth_tr - smooth_tr / period as f64 + trs[i];
        smooth_p = smooth_p - smooth_p / period as f64 + pdms[i];
        smooth_m = smooth_m - smooth_m / period as f64 + mdms[i];
        let (p_di, m_di, dx) = calc_di_dx(smooth_tr, smooth_p, smooth_m);
        last_p_di = p_di;
        last_m_di = m_di;
        dxs.push(dx);
    }
    if dxs.len() < period {
        return None;
    }
    let mut adx: f64 = dxs[..period].iter().sum::<f64>() / period as f64;
    for &dx in &dxs[period..] {
        adx = (adx * (period as f64 - 1.0) + dx) / period as f64;
    }
    Some(AdxSnapshot {
        adx,
        plus_di: last_p_di,
        minus_di: last_m_di,
    })
}

/// FR-11: Evaluasi Volatility Shield (`ATR_SPIKE_GT_2X` atau `SPREAD_GT_0_15_PCT`).
pub fn evaluate_volatility_shield(bars: &[Bar], spread_pct: f64) -> VolatilityShieldStatus {
    let atr_series = atr_series_yahudi(bars, 14);
    let mut reasons = Vec::new();
    if atr_series.is_empty() {
        let active = spread_pct > 0.0015;
        if active {
            reasons.push("SPREAD_GT_0_15_PCT");
        }
        return VolatilityShieldStatus {
            active,
            block_new_signals: active,
            atr_ratio: 1.0,
            current_atr: 0.0,
            avg72_atr: 0.0,
            spread_pct,
            reasons,
        };
    }
    let current_atr = *atr_series.last().unwrap();
    let win_start = atr_series.len().saturating_sub(72);
    let win = &atr_series[win_start..];
    let avg72_atr = win.iter().sum::<f64>() / win.len() as f64;
    let atr_ratio = if avg72_atr > 0.0 { current_atr / avg72_atr } else { 1.0 };
    if atr_ratio > 2.0 {
        reasons.push("ATR_SPIKE_GT_2X");
    }
    if spread_pct > 0.0015 {
        reasons.push("SPREAD_GT_0_15_PCT");
    }
    let active = !reasons.is_empty();
    VolatilityShieldStatus {
        active,
        block_new_signals: active,
        atr_ratio,
        current_atr,
        avg72_atr,
        spread_pct,
        reasons,
    }
}

pub fn classify_single_bar_regime(bars: &[Bar], vol_shield: &VolatilityShieldStatus) -> MarketRegime {
    if vol_shield.active && vol_shield.reasons.contains(&"ATR_SPIKE_GT_2X") {
        return MarketRegime::Volatile;
    }
    let Some(chop) = choppiness_last_yahudi(bars, 14) else {
        return MarketRegime::Uncertain;
    };
    let Some(adx_res) = adx_last_yahudi(bars, 14) else {
        return MarketRegime::Uncertain;
    };
    if adx_res.adx >= 20.0 && chop < 58.3 {
        return MarketRegime::Trending;
    }
    if chop >= 58.3 || adx_res.adx < 20.0 {
        return MarketRegime::Ranging;
    }
    MarketRegime::Uncertain
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct RegimeClassification {
    pub regime: MarketRegime,
    pub confidence: f64,
    pub raw_regimes: Vec<MarketRegime>,
}

/// FR-08: Regime Engine dengan smoothing mayoritas jendela 5 bar.
pub fn classify_regime_smoothed(bars: &[Bar], spread_pct: f64, window_size: usize) -> RegimeClassification {
    if bars.len() < 28 || window_size == 0 {
        return RegimeClassification {
            regime: MarketRegime::Uncertain,
            confidence: 1.0,
            raw_regimes: vec![MarketRegime::Uncertain],
        };
    }
    let eff_win = window_size.min(bars.len() - 27);
    let mut raw_regimes = Vec::with_capacity(eff_win);
    for w in (0..eff_win).rev() {
        let sub = &bars[..bars.len() - w];
        let vs = evaluate_volatility_shield(sub, spread_pct);
        raw_regimes.push(classify_single_bar_regime(sub, &vs));
    }
    let order = [
        MarketRegime::Trending,
        MarketRegime::Ranging,
        MarketRegime::Volatile,
        MarketRegime::Uncertain,
    ];
    let mut best = *raw_regimes.last().unwrap();
    let mut max_votes: isize = -1;
    for &cand in &order {
        let cnt = raw_regimes.iter().filter(|&&r| r == cand).count() as isize;
        if cnt > max_votes {
            max_votes = cnt;
            best = cand;
        } else if cnt == max_votes {
            let last_cand = raw_regimes.iter().rposition(|&r| r == cand).unwrap_or(0);
            let last_best = raw_regimes.iter().rposition(|&r| r == best).unwrap_or(0);
            if last_cand > last_best {
                best = cand;
            }
        }
    }
    let confidence = max_votes.max(0) as f64 / raw_regimes.len() as f64;
    RegimeClassification {
        regime: best,
        confidence,
        raw_regimes,
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ZlemaTrendResult {
    pub zlema: f64,
    pub upper_shield: f64,
    pub lower_shield: f64,
    pub direction: TrendDir,
    pub is_flip: bool,
    pub trend_score: f64,
    pub distance_pct: f64,
}

/// FR-11: Zero-Lag Trend Engine (`ZLEMA 21`) + Volatility Shield (`1.0 * ATR(14)`).
pub fn calculate_zero_lag_trend(
    bars: &[Bar],
    period: usize,
    shield_multiplier: f64,
    prev_dir: TrendDir,
) -> ZlemaTrendResult {
    let lag = (period.saturating_sub(1)) / 2;
    let min_req = period + lag;
    if period == 0 || bars.len() < min_req {
        let last_close = bars.last().map(|b| b.close).unwrap_or(0.0);
        return ZlemaTrendResult {
            zlema: last_close,
            upper_shield: last_close,
            lower_shield: last_close,
            direction: TrendDir::Neutral,
            is_flip: false,
            trend_score: 50.0,
            distance_pct: 0.0,
        };
    }
    let alpha = 2.0 / (period as f64 + 1.0);
    let mut adj_closes = Vec::with_capacity(bars.len() - lag);
    for i in lag..bars.len() {
        adj_closes.push(2.0 * bars[i].close - bars[i - lag].close);
    }
    let mut prev_zlema = adj_closes[0];
    let mut zlema = adj_closes[0];
    for (idx, &ac) in adj_closes.iter().enumerate().skip(1) {
        if idx == adj_closes.len() - 1 {
            prev_zlema = zlema;
        }
        zlema = alpha * ac + (1.0 - alpha) * zlema;
    }
    let last_bar = bars.last().unwrap();
    let current_atr = atr_last_yahudi(bars, 14).unwrap_or(last_bar.high - last_bar.low);
    let upper_shield = zlema + shield_multiplier * current_atr;
    let lower_shield = zlema - shield_multiplier * current_atr;
    let close = last_bar.close;

    let mut direction = prev_dir;
    let mut is_flip = false;
    match prev_dir {
        TrendDir::Bullish => {
            if close < lower_shield {
                direction = TrendDir::Bearish;
                is_flip = true;
            }
        }
        TrendDir::Bearish => {
            if close > upper_shield {
                direction = TrendDir::Bullish;
                is_flip = true;
            }
        }
        TrendDir::Neutral => {
            if close > upper_shield {
                direction = TrendDir::Bullish;
                is_flip = true;
            } else if close < lower_shield {
                direction = TrendDir::Bearish;
                is_flip = true;
            } else {
                direction = if zlema >= prev_zlema {
                    TrendDir::Bullish
                } else {
                    TrendDir::Bearish
                };
            }
        }
    }

    let adx_val = adx_last_yahudi(bars, 14).map(|a| a.adx).unwrap_or(20.0);
    let adx_points = ((adx_val / 50.0) * 25.0).min(25.0);
    let diff_from_zlema = if current_atr > 0.0 {
        (close - zlema).abs() / current_atr
    } else {
        0.0
    };
    let distance_points = ((diff_from_zlema / 2.0) * 25.0).min(25.0);
    let trend_score = if matches!(direction, TrendDir::Bullish | TrendDir::Bearish) {
        (50.0 + distance_points + adx_points).round().clamp(0.0, 100.0)
    } else {
        50.0
    };
    let distance_pct = if zlema > 0.0 {
        ((close - zlema) / zlema) * 100.0
    } else {
        0.0
    };

    ZlemaTrendResult {
        zlema,
        upper_shield,
        lower_shield,
        direction,
        is_flip,
        trend_score,
        distance_pct,
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum ZoneType {
    Ob,
    Fvg,
    Demand,
    Supply,
}

impl ZoneType {
    pub fn as_str(&self) -> &'static str {
        match self {
            ZoneType::Ob => "OB",
            ZoneType::Fvg => "FVG",
            ZoneType::Demand => "DEMAND",
            ZoneType::Supply => "SUPPLY",
        }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct DetectedZone {
    pub zone_type: ZoneType,
    pub side: Side,
    pub top: f64,
    pub bottom: f64,
    pub mid: f64,
    pub bar_index: usize,
    pub score: f64,
    pub usable: bool,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct SqueezeResult {
    pub squeeze_on: bool,
    pub squeeze_release: bool,
    pub momentum: f64,
    pub direction: Option<Side>,
    pub expected_distance_atr: f64,
    pub expected_target_price: f64,
    pub trigger_score: f64,
    pub bb_upper: f64,
    pub bb_lower: f64,
    pub kc_upper: f64,
    pub kc_lower: f64,
    pub atr: f64,
}

/// FR-10: Evaluasi Squeeze Momentum (Bollinger Bands 20,2.0 di dalam Keltner Channels 20,1.5 ATR).
pub fn evaluate_squeeze_trigger(bars: &[Bar]) -> SqueezeResult {
    let bb_period = 20usize;
    let bb_mult = 2.0;
    let kc_mult = 1.5;
    let min_req = bb_period + 2;
    if bars.len() < min_req {
        let last_c = bars.last().map(|b| b.close).unwrap_or(0.0);
        return SqueezeResult {
            squeeze_on: false,
            squeeze_release: false,
            momentum: 0.0,
            direction: None,
            expected_distance_atr: 0.0,
            expected_target_price: last_c,
            trigger_score: 0.0,
            bb_upper: last_c,
            bb_lower: last_c,
            kc_upper: last_c,
            kc_lower: last_c,
            atr: 0.0,
        };
    }
    let calc_bk = |sub: &[Bar]| -> (f64, f64, f64, f64, f64, bool, f64) {
        let slice = &sub[sub.len() - bb_period..];
        let sma = slice.iter().map(|b| b.close).sum::<f64>() / bb_period as f64;
        let var = slice.iter().map(|b| (b.close - sma).powi(2)).sum::<f64>() / bb_period as f64;
        let std = var.sqrt();
        let atr = atr_last_yahudi(sub, 14).unwrap_or(0.0);
        let bbu = sma + bb_mult * std;
        let bbl = sma - bb_mult * std;
        let kcu = sma + kc_mult * atr;
        let kcl = sma - kc_mult * atr;
        let is_sq = bbu < kcu && bbl > kcl;
        (sma, bbu, bbl, kcu, kcl, is_sq, atr)
    };
    let (sma, bbu, bbl, kcu, kcl, squeeze_on, cur_atr) = calc_bk(bars);
    let (_, _, _, _, _, prev_sq, _) = calc_bk(&bars[..bars.len() - 1]);
    let mut squeeze_release = prev_sq && !squeeze_on;
    if !squeeze_release && !squeeze_on && bars.len() >= min_req + 3 {
        let (_, _, _, _, _, sq2, _) = calc_bk(&bars[..bars.len() - 2]);
        let (_, _, _, _, _, sq3, _) = calc_bk(&bars[..bars.len() - 3]);
        if (sq2 || sq3) && !squeeze_on {
            squeeze_release = true;
        }
    }
    let last_bar = bars.last().unwrap();
    let hl_mid = (last_bar.high + last_bar.low) / 2.0;
    let momentum = last_bar.close - (sma + hl_mid) / 2.0;
    let direction = if momentum > 0.0 {
        Some(Side::Long)
    } else if momentum < 0.0 {
        Some(Side::Short)
    } else {
        None
    };
    let expected_distance_atr = 1.5 * cur_atr;
    let expected_target_price = match direction {
        Some(Side::Long) => last_bar.close + expected_distance_atr,
        Some(Side::Short) => last_bar.close - expected_distance_atr,
        None => last_bar.close,
    };
    let trigger_score = if squeeze_release {
        let mom_int = if cur_atr > 0.0 {
            ((momentum.abs() / cur_atr) * 25.0).min(25.0)
        } else {
            10.0
        };
        (75.0 + mom_int).round().min(100.0)
    } else if squeeze_on {
        40.0
    } else {
        20.0
    };
    SqueezeResult {
        squeeze_on,
        squeeze_release,
        momentum,
        direction,
        expected_distance_atr,
        expected_target_price,
        trigger_score,
        bb_upper: bbu,
        bb_lower: bbl,
        kc_upper: kcu,
        kc_lower: kcl,
        atr: cur_atr,
    }
}

/// FR-09: Deteksi zona FVG & Order Block 5-faktor (`minZoneScore = 60`).
pub fn detect_usable_zones(bars: &[Bar], trend_dir: TrendDir) -> (f64, Option<DetectedZone>, Vec<DetectedZone>) {
    if bars.len() < 20 {
        return (0.0, None, Vec::new());
    }
    let atr = atr_last_yahudi(bars, 14).unwrap_or(0.0);
    if atr <= 0.0 {
        return (0.0, None, Vec::new());
    }
    let mut zones = Vec::new();
    let start = bars.len().saturating_sub(60).max(2);
    for i in start..bars.len() {
        let (b0, b1, b2) = (&bars[i - 2], &bars[i - 1], &bars[i]);
        // Bullish FVG / OB
        if b2.low > b0.high && (b2.low - b0.high) >= 0.15 * atr {
            let bottom = b0.high;
            let top = b2.low;
            // Cek apakah sudah ditembus penuh setelah i
            let mitigated = bars[i + 1..].iter().any(|b| b.close < bottom);
            if !mitigated {
                let touches = bars[i + 1..].iter().filter(|b| b.low <= top && b.high >= bottom).count();
                let impulse = ((b1.close - b1.open).abs() / atr * 12.0).clamp(5.0, 25.0);
                let freshness = if touches == 0 { 25.0 } else if touches == 1 { 15.0 } else { 5.0 };
                let bos = if b2.close > b0.high + 0.3 * atr { 20.0 } else { 10.0 };
                let fvg_conf = 15.0;
                let align = if trend_dir == TrendDir::Bullish { 15.0 } else { 5.0 };
                let score = (impulse + freshness + bos + fvg_conf + align).round().clamp(0.0, 100.0);
                zones.push(DetectedZone {
                    zone_type: ZoneType::Fvg,
                    side: Side::Long,
                    top,
                    bottom,
                    mid: (top + bottom) / 2.0,
                    bar_index: i,
                    score,
                    usable: score >= 60.0,
                });
            }
        }
        // Bearish FVG / OB
        if b2.high < b0.low && (b0.low - b2.high) >= 0.15 * atr {
            let bottom = b2.high;
            let top = b0.low;
            let mitigated = bars[i + 1..].iter().any(|b| b.close > top);
            if !mitigated {
                let touches = bars[i + 1..].iter().filter(|b| b.high >= bottom && b.low <= top).count();
                let impulse = ((b1.open - b1.close).abs() / atr * 12.0).clamp(5.0, 25.0);
                let freshness = if touches == 0 { 25.0 } else if touches == 1 { 15.0 } else { 5.0 };
                let bos = if b2.close < b0.low - 0.3 * atr { 20.0 } else { 10.0 };
                let fvg_conf = 15.0;
                let align = if trend_dir == TrendDir::Bearish { 15.0 } else { 5.0 };
                let score = (impulse + freshness + bos + fvg_conf + align).round().clamp(0.0, 100.0);
                zones.push(DetectedZone {
                    zone_type: ZoneType::Fvg,
                    side: Side::Short,
                    top,
                    bottom,
                    mid: (top + bottom) / 2.0,
                    bar_index: i,
                    score,
                    usable: score >= 60.0,
                });
            }
        }
    }
    zones.retain(|z| z.usable);
    zones.sort_by(|a, b| b.score.total_cmp(&a.score));
    let best = zones.first().cloned();
    let best_score = best.as_ref().map(|z| z.score).unwrap_or(40.0);
    (best_score, best, zones)
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct ConfirmMomentumResult {
    pub allowed: bool,
    pub reason: Option<&'static str>,
}

/// Konfirmasi momentum candle tertutup 5m sebelum kirim order (`entryPolicy.ts:confirmEntryMomentum`).
/// Syarat:
/// 1. Minimal 2 bar tertutup (`prev`, `last`).
/// 2. `last` searah order (`Long: close > open`, `Short: close < open`).
/// 3. `last` punya efisiensi body `abs(close - open) / (high - low) >= 0.20`.
/// 4. `last.close` mengalahkan `prev.close` ke arah order (`Long: last.c > prev.c`, `Short: last.c < prev.c`).
pub fn confirm_entry_momentum_5m(bars_5m: &[Bar], side: Side, min_body_ratio: f64) -> ConfirmMomentumResult {
    if bars_5m.len() < 2 {
        return ConfirmMomentumResult {
            allowed: false,
            reason: Some("MOMENTUM_BARS_UNAVAILABLE"),
        };
    }
    let prev = &bars_5m[bars_5m.len() - 2];
    let last = &bars_5m[bars_5m.len() - 1];
    let range = last.high - last.low;
    if !(range > 0.0) || !range.is_finite() {
        return ConfirmMomentumResult {
            allowed: false,
            reason: Some("MOMENTUM_BAR_FLAT"),
        };
    }
    let body = (last.close - last.open).abs();
    if body / range < min_body_ratio {
        return ConfirmMomentumResult {
            allowed: false,
            reason: Some("MOMENTUM_WEAK_BODY"),
        };
    }
    match side {
        Side::Long => {
            if !(last.close > last.open && last.close > prev.close) {
                return ConfirmMomentumResult {
                    allowed: false,
                    reason: Some("MOMENTUM_AGAINST_LONG"),
                };
            }
        }
        Side::Short => {
            if !(last.close < last.open && last.close < prev.close) {
                return ConfirmMomentumResult {
                    allowed: false,
                    reason: Some("MOMENTUM_AGAINST_SHORT"),
                };
            }
        }
    }
    ConfirmMomentumResult {
        allowed: true,
        reason: None,
    }
}

// ─────────────────────── Lorentzian k-NN Analog Engine (FR-12) ───────────────────────

pub const KNN_NEUTRAL_FEATURES: [f64; 5] = [50.0, 50.0, 50.0, 20.0, 1.0];
pub const VOL_NORM_USD: f64 = 50_000_000.0;

pub fn build_knn_features_yahudi(
    zone_score: f64,
    trigger_score: f64,
    trend_score: f64,
    adx: Option<f64>,
    volume_24h_usd: Option<f64>,
) -> [f64; 5] {
    let vol = volume_24h_usd.filter(|v| v.is_finite() && *v > 0.0).unwrap_or(VOL_NORM_USD);
    let vol_ratio = (vol / VOL_NORM_USD).clamp(0.2, 5.0);
    [
        if zone_score.is_finite() { zone_score } else { 50.0 },
        if trigger_score.is_finite() { trigger_score } else { 50.0 },
        if trend_score.is_finite() { trend_score } else { 50.0 },
        adx.filter(|v| v.is_finite()).unwrap_or(20.0),
        (vol_ratio * 10_000.0).round() / 10_000.0,
    ]
}

pub fn is_degenerate_knn_features(f: &[f64; 5]) -> bool {
    f.iter()
        .zip(KNN_NEUTRAL_FEATURES.iter())
        .all(|(a, b)| (a - b).abs() < 1e-9)
}

pub fn lorentzian_distance(u: &[f64; 5], v: &[f64; 5]) -> f64 {
    u.iter()
        .zip(v.iter())
        .map(|(a, b)| (1.0 + (a - b).abs()).ln())
        .sum()
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct KnnAnalogRecord {
    pub symbol: String,
    pub features: [f64; 5],
    pub win: bool,
    pub r_multiple: f64,
    pub timestamp: i64,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct KnnQueryOutcome {
    pub knn_n: usize,
    pub knn_prob: Option<f64>,
    pub degenerate_skipped: usize,
    pub label: &'static str,
}

pub fn query_knn_lorentzian(query: &[f64; 5], history: &[KnnAnalogRecord], k: usize) -> KnnQueryOutcome {
    let usable: Vec<&KnnAnalogRecord> = history
        .iter()
        .filter(|it| !is_degenerate_knn_features(&it.features))
        .collect();
    let degenerate_skipped = history.len() - usable.len();
    let n = usable.len();
    if n < 50 || k == 0 {
        return KnnQueryOutcome {
            knn_n: n,
            knn_prob: None,
            degenerate_skipped,
            label: "INSUFFICIENT_DATA",
        };
    }
    let mut scored: Vec<(f64, bool)> = usable
        .into_iter()
        .map(|it| (lorentzian_distance(query, &it.features), it.win))
        .collect();
    scored.sort_by(|a, b| a.0.total_cmp(&b.0));
    let top_k = &scored[..k.min(scored.len())];
    let wins = top_k.iter().filter(|x| x.1).count();
    let prob = wins as f64 / top_k.len() as f64;
    KnnQueryOutcome {
        knn_n: n,
        knn_prob: Some(prob),
        degenerate_skipped,
        label: "READY",
    }
}

// ─────────────────────── Persepsi Big Brain B4 (Beta BTC & VWAP UTC) ───────────────────────

/// Beta simbol terhadap BTC (`cov(r_sym, r_btc) / var(r_btc)`) dari return log 5m (51..=200 bar).
pub fn calculate_beta_btc(sym_bars: &[Bar], btc_bars: &[Bar]) -> Option<f64> {
    let n = sym_bars.len().min(btc_bars.len()).min(200);
    if n < 51 {
        return None;
    }
    let s_slice = &sym_bars[sym_bars.len() - n..];
    let b_slice = &btc_bars[btc_bars.len() - n..];
    let mut s_ret = Vec::with_capacity(n - 1);
    let mut b_ret = Vec::with_capacity(n - 1);
    for i in 1..n {
        if s_slice[i - 1].close <= 0.0
            || s_slice[i].close <= 0.0
            || b_slice[i - 1].close <= 0.0
            || b_slice[i].close <= 0.0
        {
            return None;
        }
        s_ret.push((s_slice[i].close / s_slice[i - 1].close).ln());
        b_ret.push((b_slice[i].close / b_slice[i - 1].close).ln());
    }
    if s_ret.len() < 50 {
        return None;
    }
    let mean_s = s_ret.iter().sum::<f64>() / s_ret.len() as f64;
    let mean_b = b_ret.iter().sum::<f64>() / b_ret.len() as f64;
    let mut cov = 0.0;
    let mut var_b = 0.0;
    for (&rs, &rb) in s_ret.iter().zip(b_ret.iter()) {
        cov += (rs - mean_s) * (rb - mean_b);
        var_b += (rb - mean_b).powi(2);
    }
    if var_b <= 0.0 {
        return None;
    }
    let beta = cov / var_b;
    beta.is_finite().then_some(beta)
}

/// VWAP Sesi dengan anchor UTC 00:00 (`vwapSession` di `indicatorLibrary.ts`).
pub fn vwap_session_utc(bars: &[Bar], at_time_ms: Option<i64>) -> Option<f64> {
    let ref_ms = at_time_ms.or_else(|| bars.last().map(|b| b.open_time))?;
    let day_start = ref_ms - ref_ms.rem_euclid(86_400_000);
    let mut pv = 0.0;
    let mut vol = 0.0;
    for b in bars {
        if b.open_time < day_start || b.open_time > ref_ms {
            continue;
        }
        if b.volume > 0.0 && b.high.is_finite() && b.low.is_finite() && b.close.is_finite() {
            let typical = (b.high + b.low + b.close) / 3.0;
            pv += typical * b.volume;
            vol += b.volume;
        }
    }
    (vol > 0.0).then_some(pv / vol)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tests::synth;

    #[test]
    fn zlema_and_regime_are_causal() {
        let b = synth(120, 42);
        let z_full = calculate_zero_lag_trend(&b[..=90], 21, 1.0, TrendDir::Neutral);
        let z_prefix = calculate_zero_lag_trend(&b[..=90], 21, 1.0, TrendDir::Neutral);
        assert_eq!(z_full, z_prefix);
        let reg = classify_regime_smoothed(&b[..=90], 0.0002, 5);
        assert!(reg.confidence > 0.0);
    }

    #[test]
    fn confirm_5m_momentum_checks_direction_and_body() {
        let b0 = Bar {
            open_time: 0,
            open: 100.0,
            high: 101.0,
            low: 99.5,
            close: 100.5,
            volume: 100.0,
            quote_volume: 10000.0,
            trades: 10,
            taker_buy_base: 60.0,
        };
        let b1 = Bar {
            open_time: 300_000,
            open: 100.5,
            high: 102.0,
            low: 100.4,
            close: 101.8,
            volume: 150.0,
            quote_volume: 15000.0,
            trades: 15,
            taker_buy_base: 90.0,
        };
        assert!(confirm_entry_momentum_5m(&[b0, b1], Side::Long, 0.20).allowed);
        assert!(!confirm_entry_momentum_5m(&[b0, b1], Side::Short, 0.20).allowed);
    }

    #[test]
    fn knn_lorentzian_skips_degenerate_neutral_vectors() {
        let mut hist = Vec::new();
        for i in 0..60 {
            hist.push(KnnAnalogRecord {
                symbol: "BTCUSDT".into(),
                features: KNN_NEUTRAL_FEATURES,
                win: false,
                r_multiple: -1.0,
                timestamp: i,
            });
        }
        let res = query_knn_lorentzian(&[70.0, 80.0, 75.0, 28.0, 1.5], &hist, 5);
        assert_eq!(res.label, "INSUFFICIENT_DATA");
        assert_eq!(res.degenerate_skipped, 60);
    }
}
