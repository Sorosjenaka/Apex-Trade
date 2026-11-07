//! Port 1:1 mesin indikator mikrostruktur & kualitas tren:
//! - `trendQuality.ts`: Kaufman Efficiency Ratio (`ER(30)`) + `TrendQualityClassifier` dengan histeresis (`0.35` / `0.25`)
//! - `volumeProfile.ts`: Volume Profile 48-bin uniform overlap, `POC`, `VAH`/`VAL` (70% Value Area), `HVN`/`LVN`, `pocDistanceBins`
//! - `perceptionB4.ts`: Status eksplisit (`OK` / `NO_DATA` / `INSUFFICIENT_SAMPLES` / `FETCH_FAILED`) untuk Beta BTC & OI Change 30m
//! - `fibRetracementEngine.ts`: Detektor Setup Fibonacci Zone `0.5-0.618` pasca-BOS (`N=4` pivot delay, anti-lookahead FSM)
//! - `mrvStretchEngine.ts`: Detektor Mean-Reversion Stretch (`EMA40 ± 0.5*ATR14`, `RSI2 < 15 / > 85`, `EMA200`, `D_CLOSE vs D_EMA50`)

use apex_core::types::{Bar, Side};
use serde::{Deserialize, Serialize};

use crate::{atr_series_yahudi, calculate_beta_btc, ema, rsi_wilder};

// ─────────────────────── 1. Trend Quality Score & Hysteresis (trendQuality.ts) ───────────────────────

pub const ER_ENTER_TRENDING: f64 = 0.35;
pub const ER_EXIT_TRENDING: f64 = 0.25;
pub const ER_DEFAULT_PERIOD: usize = 30;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum TrendQuality {
    Trending,
    Sideways,
    Unavailable,
}

impl TrendQuality {
    pub fn as_str(&self) -> &'static str {
        match self {
            TrendQuality::Trending => "TRENDING",
            TrendQuality::Sideways => "SIDEWAYS",
            TrendQuality::Unavailable => "UNAVAILABLE",
        }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct TrendQualitySnapshot {
    pub quality: TrendQuality,
    pub er: Option<f64>,
    pub period: usize,
    pub enter_at: f64,
    pub exit_at: f64,
}

/// Kaufman Efficiency Ratio (`ER`) pada seri close (`period` default 30, butuh `period + 1` bar).
pub fn calculate_efficiency_ratio(bars: &[Bar], period: usize) -> Option<f64> {
    if period < 2 || bars.len() < period + 1 {
        return None;
    }
    let slice = &bars[bars.len() - (period + 1)..];
    let net_move = (slice.last()?.close - slice.first()?.close).abs();
    let mut path_length = 0.0;
    for i in 1..slice.len() {
        path_length += (slice[i].close - slice[i - 1].close).abs();
    }
    if !(path_length > 0.0) || !path_length.is_finite() {
        return None;
    }
    let clamped = net_move.min(path_length);
    Some(((clamped / path_length) * 10_000.0).round() / 10_000.0)
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct TrendQualityClassifier {
    state: TrendQuality,
    enter_at: f64,
    exit_at: f64,
    period: usize,
}

impl Default for TrendQualityClassifier {
    fn default() -> Self {
        Self::new(ER_DEFAULT_PERIOD, ER_ENTER_TRENDING, ER_EXIT_TRENDING)
    }
}

impl TrendQualityClassifier {
    pub fn new(period: usize, enter_at: f64, exit_at: f64) -> Self {
        Self {
            state: TrendQuality::Sideways,
            enter_at,
            exit_at,
            period,
        }
    }

    pub fn update(&mut self, bars: &[Bar]) -> TrendQualitySnapshot {
        let er = calculate_efficiency_ratio(bars, self.period);
        let Some(val) = er else {
            return TrendQualitySnapshot {
                quality: TrendQuality::Unavailable,
                er: None,
                period: self.period,
                enter_at: self.enter_at,
                exit_at: self.exit_at,
            };
        };
        if self.state == TrendQuality::Sideways && val >= self.enter_at {
            self.state = TrendQuality::Trending;
        } else if self.state == TrendQuality::Trending && val < self.exit_at {
            self.state = TrendQuality::Sideways;
        }
        TrendQualitySnapshot {
            quality: self.state,
            er: Some(val),
            period: self.period,
            enter_at: self.enter_at,
            exit_at: self.exit_at,
        }
    }

    pub fn current(&self) -> TrendQuality {
        self.state
    }
}

// ─────────────────────── 2. Volume Profile 48-Bin & Value Area (volumeProfile.ts) ───────────────────────

pub const VP_DEFAULT_BINS: usize = 48;
pub const VP_VALUE_AREA_PCT: f64 = 0.70;

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct VpBin {
    pub mid: f64,
    pub volume: f64,
    pub buy_volume: f64,
    pub sell_volume: f64,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct VolumeProfile48 {
    pub bins: Vec<VpBin>,
    pub poc: Option<f64>,
    pub poc_volume: f64,
    pub poc_buy_share_pct: Option<f64>,
    pub vah: Option<f64>,
    pub val: Option<f64>,
    pub price_vs_value_area: &'static str,
    pub bin_size: f64,
    pub min_price: f64,
    pub total_volume: f64,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct HvLvnLevels {
    pub hvn: Vec<f64>,
    pub lvn: Vec<f64>,
}

fn round8(v: f64) -> f64 {
    (v * 1e8).round() / 1e8
}

/// Bangun Volume Profile 48-bin dari klines (`volumeProfile.ts`).
pub fn build_volume_profile_48(bars: &[Bar], bin_count: usize) -> VolumeProfile48 {
    let empty = VolumeProfile48 {
        bins: Vec::new(),
        poc: None,
        poc_volume: 0.0,
        poc_buy_share_pct: None,
        vah: None,
        val: None,
        price_vs_value_area: "UNKNOWN",
        bin_size: 0.0,
        min_price: 0.0,
        total_volume: 0.0,
    };
    if bars.is_empty() || bin_count < 2 {
        return empty;
    }
    let mut min_price = f64::INFINITY;
    let mut max_price = f64::NEG_INFINITY;
    let mut total_volume = 0.0;
    for b in bars {
        if !b.low.is_finite() || !b.high.is_finite() || !b.volume.is_finite() {
            return empty;
        }
        if b.low < min_price {
            min_price = b.low;
        }
        if b.high > max_price {
            max_price = b.high;
        }
        total_volume += b.volume;
    }
    if min_price > max_price || !(total_volume > 0.0) {
        return empty;
    }
    let range = max_price - min_price;
    let bin_size = if range > 0.0 {
        range / bin_count as f64
    } else {
        0.0
    };
    let last_close = bars.last().map(|b| b.close).unwrap_or(min_price);
    if bin_size == 0.0 {
        let p = round8(min_price);
        return VolumeProfile48 {
            bins: vec![VpBin {
                mid: p,
                volume: total_volume,
                buy_volume: total_volume * 0.5,
                sell_volume: total_volume * 0.5,
            }],
            poc: Some(p),
            poc_volume: total_volume,
            poc_buy_share_pct: Some(50.0),
            vah: Some(round8(max_price)),
            val: Some(p),
            price_vs_value_area: "INSIDE_VA",
            bin_size: 0.0,
            min_price: p,
            total_volume,
        };
    }

    let mut volumes = vec![0.0_f64; bin_count];
    let mut buy_vols = vec![0.0_f64; bin_count];
    let mut sell_vols = vec![0.0_f64; bin_count];

    for b in bars {
        let lo = if (b.low - max_price).abs() < 1e-12 {
            bin_count - 1
        } else {
            (((b.low - min_price) / bin_size).floor() as usize).min(bin_count - 1)
        };
        let hi = if (b.high - max_price).abs() < 1e-12 {
            bin_count - 1
        } else {
            (((b.high - min_price) / bin_size).floor() as usize).min(bin_count - 1)
        };
        if hi < lo {
            continue;
        }
        let touched = (hi - lo + 1) as f64;
        let per_bin = b.volume / touched;
        let buy_ratio = if b.volume > 0.0 && b.taker_buy_base > 0.0 {
            (b.taker_buy_base / b.volume).clamp(0.0, 1.0)
        } else if b.close >= b.open {
            0.58
        } else {
            0.42
        };
        for idx in lo..=hi {
            volumes[idx] += per_bin;
            buy_vols[idx] += per_bin * buy_ratio;
            sell_vols[idx] += per_bin * (1.0 - buy_ratio);
        }
    }

    let bins: Vec<VpBin> = (0..bin_count)
        .map(|i| VpBin {
            mid: round8(min_price + (i as f64 + 0.5) * bin_size),
            volume: volumes[i],
            buy_volume: buy_vols[i],
            sell_volume: sell_vols[i],
        })
        .collect();

    let mut poc_idx = 0usize;
    for i in 1..bin_count {
        if volumes[i] > volumes[poc_idx] {
            poc_idx = i;
        }
    }

    let target = VP_VALUE_AREA_PCT * total_volume;
    let mut lo = poc_idx;
    let mut hi = poc_idx;
    let mut cumulative = volumes[poc_idx];
    while cumulative < target && (lo > 0 || hi + 1 < bin_count) {
        let below = if lo > 0 { volumes[lo - 1] } else { -1.0 };
        let above = if hi + 1 < bin_count { volumes[hi + 1] } else { -1.0 };
        if above >= below {
            hi += 1;
            cumulative += volumes[hi];
        } else {
            lo -= 1;
            cumulative += volumes[lo];
        }
    }

    let poc = round8(min_price + (poc_idx as f64 + 0.5) * bin_size);
    let vah = round8(min_price + (hi as f64 + 1.0) * bin_size);
    let val = round8(min_price + lo as f64 * bin_size);
    let poc_vol = volumes[poc_idx];
    let poc_buy_share = if poc_vol > 0.0 {
        Some(((buy_vols[poc_idx] / poc_vol) * 1000.0).round() / 10.0)
    } else {
        None
    };
    let price_vs_va = if last_close > vah {
        "ABOVE_VA"
    } else if last_close < val {
        "BELOW_VA"
    } else {
        "INSIDE_VA"
    };

    VolumeProfile48 {
        bins,
        poc: Some(poc),
        poc_volume: poc_vol,
        poc_buy_share_pct: poc_buy_share,
        vah: Some(vah),
        val: Some(val),
        price_vs_value_area: price_vs_va,
        bin_size: round8(bin_size),
        min_price: round8(min_price),
        total_volume,
    }
}

/// Deteksi High Volume Nodes (`HVN`) & Low Volume Nodes (`LVN`) lokal (`findHvnLvn`).
pub fn find_hvn_lvn(profile: &VolumeProfile48, min_ratio: f64) -> HvLvnLevels {
    let mut hvn = Vec::new();
    let mut lvn = Vec::new();
    if profile.bins.len() < 3 || profile.poc.is_none() {
        return HvLvnLevels { hvn, lvn };
    }
    let max_vol = profile
        .bins
        .iter()
        .map(|b| b.volume)
        .fold(0.0_f64, f64::max);
    if !(max_vol > 0.0) {
        return HvLvnLevels { hvn, lvn };
    }
    for i in 1..profile.bins.len() - 1 {
        let v = profile.bins[i].volume;
        if !(v > 0.0) {
            continue;
        }
        let is_peak = v >= profile.bins[i - 1].volume && v > profile.bins[i + 1].volume;
        let is_valley = v <= profile.bins[i - 1].volume && v < profile.bins[i + 1].volume;
        if is_peak && v >= min_ratio * max_vol {
            hvn.push(round8(profile.bins[i].mid));
        }
        if is_valley {
            lvn.push(round8(profile.bins[i].mid));
        }
    }
    HvLvnLevels { hvn, lvn }
}

/// Jarak harga terhadap `POC` dalam satuan bin (`distanceFromPocInBins`).
pub fn distance_from_poc_in_bins(profile: &VolumeProfile48, price: f64) -> Option<f64> {
    let poc = profile.poc?;
    if !(profile.bin_size > 0.0) || !price.is_finite() {
        return None;
    }
    Some((((price - poc) / profile.bin_size) * 100.0).round() / 100.0)
}

// ─────────────────────── 3. Perception B4 Explicit Status (perceptionB4.ts) ───────────────────────

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum PerceptionStatus {
    Ok,
    NoData,
    InsufficientSamples,
    FetchFailed,
}

impl PerceptionStatus {
    pub fn as_str(&self) -> &'static str {
        match self {
            PerceptionStatus::Ok => "OK",
            PerceptionStatus::NoData => "NO_DATA",
            PerceptionStatus::InsufficientSamples => "INSUFFICIENT_SAMPLES",
            PerceptionStatus::FetchFailed => "FETCH_FAILED",
        }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct PerceptionResult {
    pub value: Option<f64>,
    pub status: &'static str,
    pub reason: String,
}

/// Evaluasi Beta simbol terhadap BTC dengan status eksplisit (`perceptionB4.ts:calculateBetaBtc`).
pub fn calculate_beta_btc_with_status(sym_bars: &[Bar], btc_bars: Option<&[Bar]>) -> PerceptionResult {
    let Some(btc) = btc_bars else {
        return PerceptionResult {
            value: None,
            status: PerceptionStatus::FetchFailed.as_str(),
            reason: "BETA_NO_BTC_REFERENCE".into(),
        };
    };
    if btc.len() < 51 {
        return PerceptionResult {
            value: None,
            status: PerceptionStatus::InsufficientSamples.as_str(),
            reason: format!("BETA_BTC_BARS_{}_LT_51", btc.len()),
        };
    }
    if sym_bars.len() < 51 {
        return PerceptionResult {
            value: None,
            status: PerceptionStatus::InsufficientSamples.as_str(),
            reason: format!("BETA_SYM_BARS_{}_LT_51", sym_bars.len()),
        };
    }
    match calculate_beta_btc(sym_bars, btc) {
        Some(beta) => PerceptionResult {
            value: Some((beta * 1e6).round() / 1e6),
            status: PerceptionStatus::Ok.as_str(),
            reason: "BETA_OK".into(),
        },
        None => PerceptionResult {
            value: None,
            status: PerceptionStatus::NoData.as_str(),
            reason: "BETA_ZERO_VARIANCE_OR_NON_FINITE".into(),
        },
    }
}

/// Evaluasi perubahan Open Interest % (~30m = 7 sampel 5m) atau estimasi impuls partisipasi volume
/// bila endpoint historis OI tidak dikirim.
pub fn compute_oi_change_pct_from_samples(samples: &[f64]) -> PerceptionResult {
    if samples.len() < 2 {
        return PerceptionResult {
            value: None,
            status: PerceptionStatus::NoData.as_str(),
            reason: format!("OI_HIST_EMPTY({})", samples.len()),
        };
    }
    let first = samples[0];
    let last = *samples.last().unwrap_or(&0.0);
    if !first.is_finite() || !last.is_finite() || first <= 0.0 {
        return PerceptionResult {
            value: None,
            status: PerceptionStatus::NoData.as_str(),
            reason: "OI_HIST_NON_NUMERIC".into(),
        };
    }
    let pct = ((last - first) / first) * 100.0;
    if !pct.is_finite() {
        return PerceptionResult {
            value: None,
            status: PerceptionStatus::NoData.as_str(),
            reason: "OI_PCT_NOT_FINITE".into(),
        };
    }
    PerceptionResult {
        value: Some((pct * 1e4).round() / 1e4),
        status: PerceptionStatus::Ok.as_str(),
        reason: "OI_HIST_OK".into(),
    }
}

// ─────────────────────── 4. Fibonacci Zone 0.5-0.618 BOS Detector (fibRetracementEngine.ts) ───────────────────────

pub const FIB_ENTRY_RATIO: f64 = 0.5;
pub const FIB_PIVOT_N: usize = 4;

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct FibRetracementSetup {
    pub side: Side,
    pub entry_level: f64,
    pub sl_level: f64,
    pub tp_level: f64,
    pub range: f64,
    pub swing_high: f64,
    pub swing_low: f64,
    pub swing_high_idx: usize,
    pub swing_low_idx: usize,
    pub bos_idx: usize,
    pub bars_since_bos: usize,
}

#[derive(Clone, Copy, Debug)]
struct FibLeg {
    side: Side,
    e: f64,
    s: f64,
    t: f64,
    hs: f64,
    lp: f64,
    high_idx: usize,
    low_idx: usize,
    bos_idx: usize,
}

/// Deteksi setup Fibonacci Zone `0.5-0.618` dengan konfirmasi pivot `N=4` bar anti-lookahead (`fibRetracementEngine.ts`).
pub fn detect_fib_retracement(
    bars: &[Bar],
    pivot_n: usize,
    include_last_bar: bool,
    close_based: bool,
) -> Option<FibRetracementSetup> {
    let n_piv = if pivot_n == 0 { FIB_PIVOT_N } else { pivot_n };
    let n = bars.len();
    let end = if include_last_bar { n } else { n.saturating_sub(1) };
    if n_piv < 1 || end < 2 * n_piv + 3 {
        return None;
    }

    let hg: Vec<f64> = bars
        .iter()
        .map(|b| if close_based { b.close } else { b.high })
        .collect();
    let lg: Vec<f64> = bars
        .iter()
        .map(|b| if close_based { b.close } else { b.low })
        .collect();

    let mut phc: Vec<Option<f64>> = vec![None; n];
    let mut plc: Vec<Option<f64>> = vec![None; n];

    for i in n_piv..end {
        let j = i - n_piv;
        if j < n_piv || j + n_piv >= end {
            continue;
        }
        let mut max_h = f64::NEG_INFINITY;
        let mut min_l = f64::INFINITY;
        let mut max_count = 0usize;
        let mut min_count = 0usize;
        for k in (j - n_piv)..=(j + n_piv) {
            if hg[k] > max_h {
                max_h = hg[k];
                max_count = 1;
            } else if (hg[k] - max_h).abs() < 1e-12 {
                max_count += 1;
            }
            if lg[k] < min_l {
                min_l = lg[k];
                min_count = 1;
            } else if (lg[k] - min_l).abs() < 1e-12 {
                min_count += 1;
            }
        }
        if (hg[j] - max_h).abs() < 1e-12 && max_count == 1 {
            phc[i] = Some(hg[j]);
        }
        if (lg[j] - min_l).abs() < 1e-12 && min_count == 1 {
            plc[i] = Some(lg[j]);
        }
    }

    let mut last_ph: Option<(f64, usize)> = None;
    let mut last_pl: Option<(f64, usize)> = None;
    let mut broken_ph_pi: Option<usize> = None;
    let mut broken_pl_pi: Option<usize> = None;

    #[derive(Clone, Copy, PartialEq, Eq)]
    enum State {
        Idle,
        Armed,
        Consumed,
    }
    let mut state = State::Idle;
    let mut leg: Option<FibLeg> = None;

    for i in 0..end {
        if let Some(p) = phc[i] {
            last_ph = Some((p, i - n_piv));
        }
        if let Some(p) = plc[i] {
            last_pl = Some((p, i - n_piv));
        }
        let ci = bars[i].close;
        let hi = bars[i].high;
        let li = bars[i].low;

        match state {
            State::Idle => {
                if let (Some((ph_price, ph_idx)), Some((pl_price, pl_idx))) = (last_ph, last_pl) {
                    if pl_idx < i && ci > ph_price && broken_ph_pi != Some(ph_idx) {
                        let lp = pl_price;
                        let mut hs = f64::NEG_INFINITY;
                        let mut high_idx = pl_idx;
                        for (k, &val) in hg.iter().enumerate().take(i + 1).skip(pl_idx) {
                            if val > hs {
                                hs = val;
                                high_idx = k;
                            }
                        }
                        if hs > lp {
                            let rng = hs - lp;
                            let e = hs - FIB_ENTRY_RATIO * rng;
                            if ci > e {
                                leg = Some(FibLeg {
                                    side: Side::Long,
                                    e,
                                    s: lp,
                                    t: hs,
                                    hs,
                                    lp,
                                    high_idx,
                                    low_idx: pl_idx,
                                    bos_idx: i,
                                });
                                state = State::Armed;
                                broken_ph_pi = Some(ph_idx);
                            }
                        }
                    } else if ph_idx < i && ci < pl_price && broken_pl_pi != Some(pl_idx) {
                        let hs = ph_price;
                        let mut lp = f64::INFINITY;
                        let mut low_idx = ph_idx;
                        for (k, &val) in lg.iter().enumerate().take(i + 1).skip(ph_idx) {
                            if val < lp {
                                lp = val;
                                low_idx = k;
                            }
                        }
                        if hs > lp {
                            let rng = hs - lp;
                            let e = lp + FIB_ENTRY_RATIO * rng;
                            if ci < e {
                                leg = Some(FibLeg {
                                    side: Side::Short,
                                    e,
                                    s: hs,
                                    t: lp,
                                    hs,
                                    lp,
                                    high_idx: ph_idx,
                                    low_idx,
                                    bos_idx: i,
                                });
                                state = State::Armed;
                                broken_pl_pi = Some(pl_idx);
                            }
                        }
                    }
                }
            }
            State::Armed => {
                if let Some(l) = leg {
                    let hit_e = if close_based {
                        match l.side {
                            Side::Long => ci <= l.e,
                            Side::Short => ci >= l.e,
                        }
                    } else {
                        li <= l.e && l.e <= hi
                    };
                    let inv = if close_based {
                        match l.side {
                            Side::Long => ci < l.s || ci >= l.t,
                            Side::Short => ci > l.s || ci <= l.t,
                        }
                    } else {
                        match l.side {
                            Side::Long => li < l.s || hi >= l.t,
                            Side::Short => hi > l.s || li <= l.t,
                        }
                    };
                    if inv {
                        state = State::Idle;
                        leg = None;
                    } else if hit_e {
                        state = State::Consumed;
                    }
                }
            }
            State::Consumed => {
                if let Some(l) = leg {
                    let resolved = if close_based {
                        match l.side {
                            Side::Long => ci >= l.t || ci <= l.s,
                            Side::Short => ci <= l.t || ci >= l.s,
                        }
                    } else {
                        match l.side {
                            Side::Long => hi >= l.t || li <= l.s,
                            Side::Short => li <= l.t || hi >= l.s,
                        }
                    };
                    if resolved {
                        state = State::Idle;
                        leg = None;
                    }
                }
            }
        }
    }

    if state != State::Armed {
        return None;
    }
    let l = leg?;
    Some(FibRetracementSetup {
        side: l.side,
        entry_level: l.e,
        sl_level: l.s,
        tp_level: l.t,
        range: l.hs - l.lp,
        swing_high: l.hs,
        swing_low: l.lp,
        swing_high_idx: l.high_idx,
        swing_low_idx: l.low_idx,
        bos_idx: l.bos_idx,
        bars_since_bos: end.saturating_sub(1).saturating_sub(l.bos_idx),
    })
}

// ─────────────────────── 5. Mean-Reversion Stretch + Trend Gate (mrvStretchEngine.ts) ───────────────────────

pub const MRV_EMA_FAST: usize = 40;
pub const MRV_EMA_SLOW: usize = 200;
pub const MRV_RSI_PERIOD: usize = 2;
pub const MRV_RSI_LONG_MAX: f64 = 15.0;
pub const MRV_RSI_SHORT_MIN: f64 = 85.0;
pub const MRV_ATR_PERIOD: usize = 14;
pub const MRV_STRETCH_ATR: f64 = 0.5;
pub const MRV_TP_R: f64 = 1.7;

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct MrvStretchGateResult {
    pub signal: Option<Side>,
    pub reasons: Vec<String>,
    pub bar_index: isize,
    pub close: Option<f64>,
    pub ema40: Option<f64>,
    pub ema200: Option<f64>,
    pub rsi2: Option<f64>,
    pub atr14: Option<f64>,
}

/// Evaluasi gate Mean-Reversion Stretch (`S-04`, `mrvStretchEngine.ts`) pada bar tertutup terakhir.
pub fn evaluate_mrv_stretch_gate(bars_5m: &[Bar], include_last_bar: bool) -> MrvStretchGateResult {
    let end = if include_last_bar {
        bars_5m.len()
    } else {
        bars_5m.len().saturating_sub(1)
    };
    if end < MRV_EMA_SLOW {
        return MrvStretchGateResult {
            signal: None,
            reasons: vec!["MRV_INSUFFICIENT_M5_BARS".into()],
            bar_index: -1,
            close: None,
            ema40: None,
            ema200: None,
            rsi2: None,
            atr14: None,
        };
    }
    let slice = &bars_5m[..end];
    let idx = end - 1;
    let closes: Vec<f64> = slice.iter().map(|b| b.close).collect();
    let ema40_s = ema(&closes, MRV_EMA_FAST);
    let ema200_s = ema(&closes, MRV_EMA_SLOW);
    let rsi2_s = rsi_wilder(&closes, MRV_RSI_PERIOD);
    let atr_s = atr_series_yahudi(slice, MRV_ATR_PERIOD);

    let c = closes[idx];
    let e40 = ema40_s[idx];
    let e200 = ema200_s[idx];
    let r2 = rsi2_s[idx];
    let atr14 = atr_s.last().copied().unwrap_or(f64::NAN);

    if !c.is_finite() || !e40.is_finite() || !e200.is_finite() || !r2.is_finite() || !atr14.is_finite() {
        return MrvStretchGateResult {
            signal: None,
            reasons: vec!["MRV_INDICATOR_WARMUP".into()],
            bar_index: idx as isize,
            close: c.is_finite().then_some(c),
            ema40: e40.is_finite().then_some(e40),
            ema200: e200.is_finite().then_some(e200),
            rsi2: r2.is_finite().then_some(r2),
            atr14: atr14.is_finite().then_some(atr14),
        };
    }

    let mut reasons = Vec::new();
    let stretch = MRV_STRETCH_ATR * atr14;
    let long_stretch = c < e40 - stretch;
    let short_stretch = c > e40 + stretch;
    let long_rsi = r2 < MRV_RSI_LONG_MAX;
    let short_rsi = r2 > MRV_RSI_SHORT_MIN;
    let long_trend = c > e200;
    let short_trend = c < e200;

    let signal = if long_stretch && long_rsi && long_trend {
        reasons.push("MRV_LONG_STRETCH_RSI2_TREND_OK".into());
        Some(Side::Long)
    } else if short_stretch && short_rsi && short_trend {
        reasons.push("MRV_SHORT_STRETCH_RSI2_TREND_OK".into());
        Some(Side::Short)
    } else {
        if !long_stretch && !short_stretch {
            reasons.push("MRV_NO_STRETCH".into());
        }
        if !long_rsi && !short_rsi {
            reasons.push("MRV_RSI2_NEUTRAL".into());
        }
        if !long_trend && !short_trend {
            reasons.push("MRV_TREND_CONFLICT".into());
        }
        None
    };

    MrvStretchGateResult {
        signal,
        reasons,
        bar_index: idx as isize,
        close: Some(c),
        ema40: Some(e40),
        ema200: Some(e200),
        rsi2: Some(r2),
        atr14: Some(atr14),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tests::synth;

    #[test]
    fn trend_quality_classifier_hysteresis_works() {
        let bars = synth(80, 7);
        let mut clf = TrendQualityClassifier::default();
        let snap = clf.update(&bars);
        assert!(snap.er.is_some());
        assert_ne!(snap.quality, TrendQuality::Unavailable);
    }

    #[test]
    fn volume_profile_48_computes_poc_vah_val_and_distance() {
        let bars = synth(100, 11);
        let vp = build_volume_profile_48(&bars, 48);
        assert_eq!(vp.bins.len(), 48);
        assert!(vp.poc.is_some());
        assert!(vp.vah.unwrap() >= vp.poc.unwrap());
        assert!(vp.val.unwrap() <= vp.poc.unwrap());
        let last_price = bars.last().unwrap().close;
        let dist = distance_from_poc_in_bins(&vp, last_price);
        assert!(dist.is_some());
        let hvn_lvn = find_hvn_lvn(&vp, 0.5);
        assert!(!hvn_lvn.hvn.is_empty());
    }

    #[test]
    fn perception_status_reports_explicit_reasons() {
        let bars = synth(90, 21);
        let btc = synth(90, 22);
        let ok = calculate_beta_btc_with_status(&bars, Some(&btc));
        assert_eq!(ok.status, "OK");
        assert!(ok.value.is_some());

        let short = &bars[..20];
        let ins = calculate_beta_btc_with_status(short, Some(&btc));
        assert_eq!(ins.status, "INSUFFICIENT_SAMPLES");

        let oi = compute_oi_change_pct_from_samples(&[1000.0, 1010.0, 1025.0]);
        assert_eq!(oi.status, "OK");
        assert_eq!(oi.value, Some(2.5));
    }

    #[test]
    fn mrv_stretch_evaluates_closed_bars() {
        let bars = synth(240, 99);
        let res = evaluate_mrv_stretch_gate(&bars, true);
        assert_eq!(res.bar_index, 239);
        assert!(res.ema40.is_some());
        assert!(res.ema200.is_some());
        assert!(res.rsi2.is_some());
    }
}
