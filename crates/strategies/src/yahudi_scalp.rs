//! Port 1:1 Strategi Scalping Yahudi-Bot (`SCALP` Mode Only — Tanpa `RSW` Swing):
//! - `S-01 Liquidity Sweep Reclaim` (`RANGE_30B` + `ASIA` / `PREV_DAY` session sweep reclaim)
//! - `S-02 Liquidity Sniper` (`Alpha_Wizard` Liquidation Heatmap 100x/50x/25x/10x + Stop Pools Pivot 8)
//! - Hard Gates:
//!   - Regime Gate (`CHOPPY_NO_TRADE` blokir semua entry; `HIGH_VOL_NEWS` hanya izinkan `S-01` jika `allow_news_sweep`)
//!   - Volatility Shield (`ATR_SPIKE_GT_2X`, `SPREAD_GT_0_15_PCT`)
//!   - CVD Alignment Gate (`is_cvd_aligned`)
//!   - SL Clamping Band (`2.0% - 4.0%`, buffer `0.46 ATR`, `min_sl_atr = 0.805`, council floor `SL/ATR >= 0.50x`)
//!   - Single-Target `1.7R` (`$1.00` Risk / `$1.70` TP)

use apex_core::sizing::EntryKind;
use apex_core::types::{Bar, Side, Tf};
use apex_indicators::yahudi::{
    adx_last_yahudi, atr_last_yahudi, calculate_zero_lag_trend, choppiness_last_yahudi,
    classify_regime_smoothed, confirm_entry_momentum_5m, detect_usable_zones,
    evaluate_squeeze_trigger, evaluate_volatility_shield, vwap_session_utc, MarketRegime, TrendDir,
};
use serde::{Deserialize, Serialize};

use crate::{Signal, StrategyId, StrategyParams};

pub const MIN_SL_DISTANCE_PCT: f64 = 0.02; // 2.0%
pub const MAX_SL_DISTANCE_PCT: f64 = 0.04; // 4.0%
pub const COUNCIL_MIN_SL_ATR_RATIO: f64 = 0.50;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum CvdDivergence {
    None,
    Bullish,
    Bearish,
}

/// Konteks orderflow/microstructure tambahan untuk strategi scalping Yahudi-Bot.
#[derive(Clone, Copy, Debug, Serialize, Deserialize)]
pub struct YahudiOrderflowCtx {
    pub cvd_slope: f64,
    pub cvd_divergence: CvdDivergence,
    pub obi: f64,
    pub spread_pct: f64,
    pub allow_news_sweep: bool,
}

impl Default for YahudiOrderflowCtx {
    fn default() -> Self {
        Self {
            cvd_slope: 0.0,
            cvd_divergence: CvdDivergence::None,
            obi: 0.0,
            spread_pct: 0.0002,
            allow_news_sweep: false,
        }
    }
}

/// Estimasi CVD slope dan divergence secara kausal dari bar OHLCV (`taker_buy_base` vs `volume`)
/// ketika dipanggil dari engine backtest atau bar-only pipeline.
pub fn derive_bar_orderflow_ctx(bars: &[Bar], idx: usize) -> YahudiOrderflowCtx {
    if idx < 10 || idx >= bars.len() {
        return YahudiOrderflowCtx::default();
    }
    let start = idx.saturating_sub(9);
    let mut cvd_series = Vec::with_capacity(idx - start + 1);
    let mut acc = 0.0;
    for b in &bars[start..=idx] {
        let buy = b.taker_buy_base;
        let sell = (b.volume - buy).max(0.0);
        let delta = buy - sell;
        acc += delta;
        cvd_series.push(acc);
    }
    let cvd_slope =
        cvd_series.last().copied().unwrap_or(0.0) - cvd_series.first().copied().unwrap_or(0.0);
    let price_slope = bars[idx].close - bars[start].close;

    let cvd_divergence = if price_slope < 0.0 && cvd_slope > 0.0 {
        CvdDivergence::Bullish
    } else if price_slope > 0.0 && cvd_slope < 0.0 {
        CvdDivergence::Bearish
    } else {
        CvdDivergence::None
    };

    let total_vol: f64 = bars[idx.saturating_sub(4)..=idx].iter().map(|b| b.volume).sum();
    let total_buy: f64 = bars[idx.saturating_sub(4)..=idx]
        .iter()
        .map(|b| b.taker_buy_base)
        .sum();
    let obi = if total_vol > 0.0 {
        ((2.0 * total_buy - total_vol) / total_vol).clamp(-1.0, 1.0)
    } else {
        0.0
    };

    YahudiOrderflowCtx {
        cvd_slope,
        cvd_divergence,
        obi,
        spread_pct: 0.0002,
        allow_news_sweep: false,
    }
}

/// Hard gate CVD alignment (`isCvdAligned` di `Yahudi-Bot`).
pub fn is_cvd_aligned(side: Side, ctx: &YahudiOrderflowCtx) -> bool {
    match side {
        Side::Long => ctx.cvd_divergence != CvdDivergence::Bearish && ctx.cvd_slope >= 0.0,
        Side::Short => ctx.cvd_divergence != CvdDivergence::Bullish && ctx.cvd_slope <= 0.0,
    }
}

/// Clamp SL ke rentang `2.0% - 4.0%`, dengan lantai ATR `min_sl_atr` (`0.805`) dan council floor `SL/ATR >= 0.50`.
pub fn clamp_sl_to_band(
    entry_price: f64,
    raw_sl: f64,
    side: Side,
    atr_14: f64,
    min_sl_atr: f64,
) -> Option<f64> {
    if entry_price <= 0.0 || !entry_price.is_finite() || !raw_sl.is_finite() {
        return None;
    }
    let raw_dist = (entry_price - raw_sl).abs();
    let raw_dist_pct = raw_dist / entry_price;

    if raw_dist_pct > MAX_SL_DISTANCE_PCT {
        return None;
    }

    let atr_floor_pct = if atr_14 > 0.0 && min_sl_atr > 0.0 {
        (atr_14 * min_sl_atr) / entry_price
    } else {
        0.0
    };
    let effective_min_pct = MIN_SL_DISTANCE_PCT.max(atr_floor_pct);
    if effective_min_pct > MAX_SL_DISTANCE_PCT {
        return None;
    }

    let sl_price = if raw_dist_pct < effective_min_pct {
        match side {
            Side::Long => entry_price * (1.0 - effective_min_pct),
            Side::Short => entry_price * (1.0 + effective_min_pct),
        }
    } else {
        raw_sl
    };

    let final_dist = (entry_price - sl_price).abs();
    if atr_14 > 0.0 && (final_dist / atr_14) < COUNCIL_MIN_SL_ATR_RATIO {
        return None;
    }

    Some(sl_price)
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct RangeSweepReclaim {
    pub side: Side,
    pub swept_level: f64,
    pub reclaim_close: f64,
    pub stop_extreme: f64,
    pub pool_label: String,
}

/// Deteksi `Range Sweep Reclaim` 30-bar (`detectRangeSweepReclaim` di `Yahudi-Bot`).
pub fn detect_range_sweep_reclaim(bars: &[Bar], atr_14: f64) -> Option<RangeSweepReclaim> {
    if bars.len() < 32 {
        return None;
    }
    let last = bars.last()?;
    let window = &bars[bars.len() - 31..bars.len() - 1];
    let range_high = window.iter().map(|b| b.high).fold(f64::NEG_INFINITY, f64::max);
    let range_low = window.iter().map(|b| b.low).fold(f64::INFINITY, f64::min);
    let tolerance = (atr_14 * 0.05).max(last.close * 0.0005);

    let swept_low_and_reclaimed =
        last.low < range_low - tolerance && last.close > range_low && last.close > last.open;
    if swept_low_and_reclaimed {
        return Some(RangeSweepReclaim {
            side: Side::Long,
            swept_level: range_low,
            reclaim_close: last.close,
            stop_extreme: last.low,
            pool_label: "RANGE_30B_LOW".to_string(),
        });
    }

    let swept_high_and_rejected =
        last.high > range_high + tolerance && last.close < range_high && last.close < last.open;
    if swept_high_and_rejected {
        return Some(RangeSweepReclaim {
            side: Side::Short,
            swept_level: range_high,
            reclaim_close: last.close,
            stop_extreme: last.high,
            pool_label: "RANGE_30B_HIGH".to_string(),
        });
    }

    None
}

/// Deteksi Session Liquidity Sweep Reclaim (`ASIA_LOW`, `PREV_DAY_LOW`, `ASIA_HIGH`, `PREV_DAY_HIGH`).
pub fn detect_session_sweep_reclaim(bars: &[Bar], atr_14: f64) -> Option<RangeSweepReclaim> {
    if bars.len() < 32 {
        return None;
    }
    let last = bars.last()?;
    let day_ms: i64 = 86_400_000;
    let asia_end_offset_ms: i64 = 8 * 3_600_000; // 08:00 UTC
    let current_day_start = last.open_time - last.open_time.rem_euclid(day_ms);
    let prev_day_start = current_day_start - day_ms;
    let current_time_in_day = last.open_time - current_day_start;

    let prior_bars = &bars[..bars.len() - 1];
    let mut asia_high = f64::NEG_INFINITY;
    let mut asia_low = f64::INFINITY;
    let mut asia_count = 0usize;

    let mut prev_day_high = f64::NEG_INFINITY;
    let mut prev_day_low = f64::INFINITY;
    let mut prev_day_count = 0usize;

    for b in prior_bars {
        if b.open_time >= current_day_start && b.open_time < current_day_start + asia_end_offset_ms {
            asia_high = asia_high.max(b.high);
            asia_low = asia_low.min(b.low);
            asia_count += 1;
        } else if b.open_time >= prev_day_start && b.open_time < current_day_start {
            prev_day_high = prev_day_high.max(b.high);
            prev_day_low = prev_day_low.min(b.low);
            prev_day_count += 1;
        }
    }

    let tolerance = (atr_14 * 0.05).max(last.close * 0.0005);

    let mut low_pools: Vec<(&str, f64)> = Vec::new();
    let mut high_pools: Vec<(&str, f64)> = Vec::new();

    if asia_count >= 4 && current_time_in_day >= 4 * 3_600_000 {
        low_pools.push(("ASIA_LOW", asia_low));
        high_pools.push(("ASIA_HIGH", asia_high));
    }
    if prev_day_count >= 8 {
        low_pools.push(("PREV_DAY_LOW", prev_day_low));
        high_pools.push(("PREV_DAY_HIGH", prev_day_high));
    }

    for (label, level) in low_pools {
        if level.is_finite()
            && last.low < level - tolerance
            && last.close > level
            && last.close > last.open
        {
            return Some(RangeSweepReclaim {
                side: Side::Long,
                swept_level: level,
                reclaim_close: last.close,
                stop_extreme: last.low,
                pool_label: label.to_string(),
            });
        }
    }

    for (label, level) in high_pools {
        if level.is_finite()
            && last.high > level + tolerance
            && last.close < level
            && last.close < last.open
        {
            return Some(RangeSweepReclaim {
                side: Side::Short,
                swept_level: level,
                reclaim_close: last.close,
                stop_extreme: last.high,
                pool_label: label.to_string(),
            });
        }
    }

    None
}

// ============================================================================
// S-02 Liquidity Sniper (Alpha_Wizard Liquidation Heatmap + Stop Hunt Pools)
// ============================================================================

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum LiquidityPoolType {
    ShortLiq,
    LongLiq,
    StopShort,
    StopLong,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct LiquidityPool {
    pub price: f64,
    pub pool_type: LiquidityPoolType,
    pub leverage_tier: String,
    pub volume_weight: f64,
    pub strength: f64,
    pub distance_pct: f64,
    pub age_bars: usize,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct LiquiditySniperSnapshot {
    pub current_price: f64,
    pub short_magnet: Option<LiquidityPool>,
    pub long_magnet: Option<LiquidityPool>,
    pub pull_direction: String, // "UP", "DOWN", "BALANCED"
    pub pull_bias: f64,
    pub pools_above_count: usize,
    pub pools_below_count: usize,
}

struct LeverageTierSpec {
    label: &'static str,
    dist_pct: f64,
    weight: f64,
}

const LEVERAGE_TIERS: [LeverageTierSpec; 4] = [
    LeverageTierSpec { label: "100x", dist_pct: 0.009, weight: 1.6 },
    LeverageTierSpec { label: "50x", dist_pct: 0.018, weight: 1.3 },
    LeverageTierSpec { label: "25x", dist_pct: 0.036, weight: 1.0 },
    LeverageTierSpec { label: "10x", dist_pct: 0.090, weight: 0.7 },
];

const STOP_PIVOT_LEN: usize = 8;
const DECAY_HALF_LIFE_BARS: f64 = 50.0;

#[derive(Clone, Debug)]
struct RawPool {
    price: f64,
    pool_type: LiquidityPoolType,
    tier: &'static str,
    weight: f64,
    bar_idx: usize,
}

/// Port 1:1 `evaluateLiquiditySniper` dari `Yahudi-Bot/apps/server/src/engines/liquiditySniper.ts`.
pub fn evaluate_liquidity_sniper(bars: &[Bar], atr_14: f64) -> LiquiditySniperSnapshot {
    let current_price = bars.last().map(|b| b.close).unwrap_or(0.0);
    if bars.len() < 30 || current_price <= 0.0 {
        return LiquiditySniperSnapshot {
            current_price,
            short_magnet: None,
            long_magnet: None,
            pull_direction: "BALANCED".to_string(),
            pull_bias: 1.0,
            pools_above_count: 0,
            pools_below_count: 0,
        };
    }

    let lookback = 120usize;
    let start_idx = bars.len().saturating_sub(lookback);
    let window = &bars[start_idx..];
    let avg_vol = window.iter().map(|b| b.volume).sum::<f64>() / (window.len().max(1) as f64);

    let mut raw_pools: Vec<RawPool> = Vec::new();

    // 1. Leverage liquidation levels per candle weighted by relative volume
    for (i, c) in window.iter().enumerate() {
        let rel_vol = if avg_vol > 0.0 { c.volume / avg_vol } else { 1.0 };
        if rel_vol < 0.8 {
            continue;
        }
        let body_mid = (c.open + c.close) / 2.0;
        for tier in &LEVERAGE_TIERS {
            raw_pools.push(RawPool {
                price: body_mid * (1.0 + tier.dist_pct),
                pool_type: LiquidityPoolType::ShortLiq,
                tier: tier.label,
                weight: rel_vol * tier.weight,
                bar_idx: i,
            });
            raw_pools.push(RawPool {
                price: body_mid * (1.0 - tier.dist_pct),
                pool_type: LiquidityPoolType::LongLiq,
                tier: tier.label,
                weight: rel_vol * tier.weight,
                bar_idx: i,
            });
        }
    }

    // 2. Swing High / Swing Low Stop-Loss Pools (pivot length = 8)
    if window.len() > STOP_PIVOT_LEN * 2 {
        for i in STOP_PIVOT_LEN..(window.len() - STOP_PIVOT_LEN) {
            let slice = &window[i - STOP_PIVOT_LEN..=i + STOP_PIVOT_LEN];
            let max_high = slice.iter().map(|b| b.high).fold(f64::NEG_INFINITY, f64::max);
            let min_low = slice.iter().map(|b| b.low).fold(f64::INFINITY, f64::min);
            let rel_vol = if avg_vol > 0.0 {
                (window[i].volume / avg_vol).max(1.0)
            } else {
                1.0
            };

            if (window[i].high - max_high).abs() <= 1e-12 {
                raw_pools.push(RawPool {
                    price: window[i].high * 1.002,
                    pool_type: LiquidityPoolType::StopShort,
                    tier: "SWING",
                    weight: rel_vol * 1.4,
                    bar_idx: i,
                });
            }
            if (window[i].low - min_low).abs() <= 1e-12 {
                raw_pools.push(RawPool {
                    price: window[i].low * 0.998,
                    pool_type: LiquidityPoolType::StopLong,
                    tier: "SWING",
                    weight: rel_vol * 1.4,
                    bar_idx: i,
                });
            }
        }
    }

    // 3. Purge pools already swept by subsequent candle wicks + apply exponential time decay
    let cluster_width = (current_price * 0.0025).max(atr_14 * 0.5);
    let mut clusters: Vec<LiquidityPool> = Vec::new();

    for pool in &raw_pools {
        let mut swept = false;
        for later in &window[pool.bar_idx + 1..] {
            match pool.pool_type {
                LiquidityPoolType::ShortLiq | LiquidityPoolType::StopShort => {
                    if later.high >= pool.price {
                        swept = true;
                        break;
                    }
                }
                LiquidityPoolType::LongLiq | LiquidityPoolType::StopLong => {
                    if later.low <= pool.price {
                        swept = true;
                        break;
                    }
                }
            }
        }
        if swept {
            continue;
        }

        let age_bars = window.len().saturating_sub(1 + pool.bar_idx);
        let decay = 0.5_f64.powf(age_bars as f64 / DECAY_HALF_LIFE_BARS);
        let decayed_weight = pool.weight * decay;

        let is_above_type = matches!(
            pool.pool_type,
            LiquidityPoolType::ShortLiq | LiquidityPoolType::StopShort
        );

        if let Some(existing) = clusters.iter_mut().find(|c| {
            let c_above = matches!(
                c.pool_type,
                LiquidityPoolType::ShortLiq | LiquidityPoolType::StopShort
            );
            (c.price - pool.price).abs() <= cluster_width && c_above == is_above_type
        }) {
            let total_w = existing.volume_weight + decayed_weight;
            if total_w > 0.0 {
                existing.price =
                    (existing.price * existing.volume_weight + pool.price * decayed_weight) / total_w;
            }
            existing.volume_weight = total_w;
            existing.age_bars = existing.age_bars.min(age_bars);
            if pool.tier == "SWING" {
                existing.pool_type = pool.pool_type;
                existing.leverage_tier = "SWING".to_string();
            }
        } else {
            clusters.push(LiquidityPool {
                price: pool.price,
                pool_type: pool.pool_type,
                leverage_tier: pool.tier.to_string(),
                volume_weight: decayed_weight,
                strength: 0.0,
                distance_pct: (pool.price - current_price) / current_price,
                age_bars,
            });
        }
    }

    // 4. Normalize strength (0..100) and rank magnets
    let max_weight = clusters
        .iter()
        .map(|c| c.volume_weight)
        .fold(1.0_f64, f64::max);

    for c in &mut clusters {
        c.strength = ((c.volume_weight / max_weight) * 100.0).round().min(100.0);
        c.distance_pct = (c.price - current_price) / current_price;
    }

    let mut pools_above: Vec<LiquidityPool> = clusters
        .iter()
        .filter(|c| c.price > current_price * 1.001)
        .cloned()
        .collect();
    pools_above.sort_by(|a, b| b.strength.total_cmp(&a.strength));
    pools_above.truncate(8);

    let mut pools_below: Vec<LiquidityPool> = clusters
        .iter()
        .filter(|c| c.price < current_price * 0.999)
        .cloned()
        .collect();
    pools_below.sort_by(|a, b| b.strength.total_cmp(&a.strength));
    pools_below.truncate(8);

    let short_magnet = pools_above
        .iter()
        .filter(|p| p.distance_pct.abs() <= 0.05)
        .max_by(|a, b| a.strength.total_cmp(&b.strength))
        .cloned()
        .or_else(|| pools_above.first().cloned());

    let long_magnet = pools_below
        .iter()
        .filter(|p| p.distance_pct.abs() <= 0.05)
        .max_by(|a, b| a.strength.total_cmp(&b.strength))
        .cloned()
        .or_else(|| pools_below.first().cloned());

    let above_gravity: f64 = pools_above
        .iter()
        .map(|p| p.strength / (p.distance_pct.abs() * 100.0).max(0.2))
        .sum();
    let below_gravity: f64 = pools_below
        .iter()
        .map(|p| p.strength / (p.distance_pct.abs() * 100.0).max(0.2))
        .sum();

    let pull_bias = if below_gravity > 0.0 {
        above_gravity / below_gravity
    } else if above_gravity > 0.0 {
        2.0
    } else {
        1.0
    };

    let pull_direction = if pull_bias > 1.25 {
        "UP"
    } else if pull_bias < 0.8 {
        "DOWN"
    } else {
        "BALANCED"
    };

    LiquiditySniperSnapshot {
        current_price,
        short_magnet,
        long_magnet,
        pull_direction: pull_direction.to_string(),
        pull_bias,
        pools_above_count: pools_above.len(),
        pools_below_count: pools_below.len(),
    }
}

// ============================================================================
// Evaluasi Strategi S-01 (Liquidity Sweep Reclaim) & S-02 (Liquidity Sniper)
// ============================================================================

/// Evaluasi `S-01 Liquidity Sweep Reclaim` pada indeks bar `i` (kausal: hanya memakai `bars[..=i]`).
pub fn evaluate_s01_sweep(
    symbol: &str,
    bars: &[Bar],
    i: usize,
    entry_tf: Tf,
    p: &StrategyParams,
    of_ctx: Option<YahudiOrderflowCtx>,
) -> Option<Signal> {
    if i < 55 || i >= bars.len() {
        return None;
    }
    let slice = &bars[..=i];
    let last = &slice[i];
    let atr_14 = atr_last_yahudi(slice, 14).unwrap_or(0.0);
    if !atr_14.is_finite() || atr_14 <= 0.0 {
        return None;
    }

    let ctx = of_ctx.unwrap_or_else(|| derive_bar_orderflow_ctx(slice, i));

    // 1. Volatility Shield
    let shield = evaluate_volatility_shield(slice, ctx.spread_pct);
    if shield.block_new_signals {
        return None;
    }

    // 2. Regime Gate (5-bar majority vote smoothing + anti-chop guard)
    let chop_14 = choppiness_last_yahudi(slice, 14).unwrap_or(50.0);
    let adx_14 = adx_last_yahudi(slice, 14).map(|a| a.adx).unwrap_or(20.0);
    if chop_14 > 61.8 && adx_14 < 20.0 {
        return None;
    }
    let regime = classify_regime_smoothed(slice, ctx.spread_pct, 5);
    if regime.regime == MarketRegime::Uncertain {
        return None;
    }
    if regime.regime == MarketRegime::Volatile && !ctx.allow_news_sweep {
        return None;
    }

    // 3. Detect Range Sweep (30-bar) or Session Sweep (ASIA / PREV_DAY)
    let sweep = detect_range_sweep_reclaim(slice, atr_14)
        .or_else(|| detect_session_sweep_reclaim(slice, atr_14))?;

    // 4. Hard Gate: CVD alignment
    if p.scalp_cvd_gate && !is_cvd_aligned(sweep.side, &ctx) {
        return None;
    }

    // 5. Compute raw SL from stop_extreme +- 0.46 ATR, then clamp to 2.0%..4.0% band
    let sl_buffer = atr_14 * p.scalp_sl_buffer_atr;
    let raw_sl = match sweep.side {
        Side::Long => sweep.stop_extreme - sl_buffer,
        Side::Short => sweep.stop_extreme + sl_buffer,
    };
    let sl = clamp_sl_to_band(
        sweep.reclaim_close,
        raw_sl,
        sweep.side,
        atr_14,
        p.scalp_min_sl_atr,
    )?;

    Some(Signal {
        strategy: StrategyId::S01Sweep,
        symbol: symbol.to_string(),
        side: sweep.side,
        entry: sweep.reclaim_close,
        sl,
        kind: EntryKind::LimitMaker,
        ttl_bars: p.scalp_ttl.max(1),
        bar_close_ms: last.open_time + entry_tf.ms(),
        cancel_beyond: Some(sweep.swept_level),
        note: format!("S01_SWEEP:{}", sweep.pool_label),
    })
}

/// Evaluasi `S-02 Liquidity Sniper` pada indeks bar `i` (kausal: hanya memakai `bars[..=i]`).
pub fn evaluate_s02_sniper(
    symbol: &str,
    bars: &[Bar],
    i: usize,
    entry_tf: Tf,
    p: &StrategyParams,
    of_ctx: Option<YahudiOrderflowCtx>,
) -> Option<Signal> {
    if i < 55 || i >= bars.len() {
        return None;
    }
    let slice = &bars[..=i];
    let last = &slice[i];
    let atr_14 = atr_last_yahudi(slice, 14).unwrap_or(0.0);
    if !atr_14.is_finite() || atr_14 <= 0.0 {
        return None;
    }

    let ctx = of_ctx.unwrap_or_else(|| derive_bar_orderflow_ctx(slice, i));

    // 1. Volatility Shield
    let shield = evaluate_volatility_shield(slice, ctx.spread_pct);
    if shield.block_new_signals {
        return None;
    }

    // 2. Regime Gate: S-02 diblokir pada CHOPPY_NO_TRADE maupun VOLATILE/UNCERTAIN
    let chop_14 = choppiness_last_yahudi(slice, 14).unwrap_or(50.0);
    let adx_14 = adx_last_yahudi(slice, 14).map(|a| a.adx).unwrap_or(20.0);
    if chop_14 > 61.8 && adx_14 < 20.0 {
        return None;
    }
    let regime = classify_regime_smoothed(slice, ctx.spread_pct, 5);
    if matches!(regime.regime, MarketRegime::Uncertain | MarketRegime::Volatile) {
        return None;
    }

    // 3. Zero-Lag Trend Engine (ZLEMA 21) + Liquidity Sniper Engine
    let zl = calculate_zero_lag_trend(slice, 21, 1.0, TrendDir::Neutral);
    let sniper = evaluate_liquidity_sniper(slice, atr_14);
    let threshold = if p.scalp_pull_bias_threshold > 0.0 {
        p.scalp_pull_bias_threshold
    } else {
        1.6
    };

    let (side, magnet) = if sniper.pull_direction == "UP"
        && sniper.pull_bias >= threshold
        && zl.direction == TrendDir::Bullish
        && last.close > last.open
    {
        (Side::Long, sniper.short_magnet?)
    } else if sniper.pull_direction == "DOWN"
        && (1.0 / sniper.pull_bias.max(0.01)) >= threshold
        && zl.direction == TrendDir::Bearish
        && last.close < last.open
    {
        (Side::Short, sniper.long_magnet?)
    } else {
        return None;
    };

    // 4. Hard Gate: CVD alignment
    if p.scalp_cvd_gate && !is_cvd_aligned(side, &ctx) {
        return None;
    }

    // 5. Zone-based limit entry (jika ada FVG/OB usable searah di dekat harga) atau close candle
    let (_, _, zones) = detect_usable_zones(slice, zl.direction);
    let entry_price = zones
        .iter()
        .find(|z| {
            z.usable
                && z.side == side
                && ((side == Side::Long && z.bottom <= last.close)
                    || (side == Side::Short && z.top >= last.close))
                && ((last.close - z.mid).abs() / last.close) <= 0.004
        })
        .map(|z| match side {
            Side::Long => z.bottom,
            Side::Short => z.top,
        })
        .unwrap_or(last.close);

    // 6. Raw SL dari candle extreme +- 0.46 ATR, lalu clamp ke 2.0%..4.0% band
    let sl_buffer = atr_14 * p.scalp_sl_buffer_atr;
    let raw_sl = match side {
        Side::Long => last.low - sl_buffer,
        Side::Short => last.high + sl_buffer,
    };
    let sl = clamp_sl_to_band(entry_price, raw_sl, side, atr_14, p.scalp_min_sl_atr)?;

    let squeeze = evaluate_squeeze_trigger(slice);
    let sq_tag = if squeeze.squeeze_release { ":SQZ" } else { "" };

    Some(Signal {
        strategy: StrategyId::S02Sniper,
        symbol: symbol.to_string(),
        side,
        entry: entry_price,
        sl,
        kind: EntryKind::LimitMaker,
        ttl_bars: p.scalp_ttl.max(1),
        bar_close_ms: last.open_time + entry_tf.ms(),
        cancel_beyond: Some(zl.zlema),
        note: format!(
            "S02_SNIPER:{}:{:.0}:BIAS_{:.2}{}",
            magnet.leverage_tier, magnet.strength, sniper.pull_bias, sq_tag
        ),
    })
}

/// Diagnostik lengkap untuk telemetri `YAHUDI_TRACE` di live loop & dashboard.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct YahudiScalpTrace {
    pub regime_state: String,
    pub regime_reason: String,
    pub chop_14: f64,
    pub adx_14: f64,
    pub atr_14: f64,
    pub vwap_session: f64,
    pub shield_safe: bool,
    pub shield_reason: String,
    pub zl_trend: String,
    pub zl_ema: f64,
    pub zl_score: f64,
    pub zl_is_flip: bool,
    pub squeeze_on: bool,
    pub squeeze_fired: bool,
    pub usable_zones_count: usize,
    pub s01_sweep_pool: Option<String>,
    pub s01_sweep_side: Option<String>,
    pub s02_pull_direction: String,
    pub s02_pull_bias: f64,
    pub s02_short_magnet: Option<f64>,
    pub s02_long_magnet: Option<f64>,
    pub cvd_aligned_long: bool,
    pub cvd_aligned_short: bool,
    pub momentum_5m_long_ok: bool,
    pub momentum_5m_short_ok: bool,
    pub blocker: String,
}

pub fn trace_yahudi_scalp(
    symbol: &str,
    bars_15m: &[Bar],
    bars_5m: &[Bar],
    p: &StrategyParams,
    of_ctx: YahudiOrderflowCtx,
) -> YahudiScalpTrace {
    if bars_15m.len() < 55 {
        return YahudiScalpTrace {
            regime_state: "WARMUP".into(),
            regime_reason: "INSUFFICIENT_BARS".into(),
            chop_14: 50.0,
            adx_14: 0.0,
            atr_14: 0.0,
            vwap_session: 0.0,
            shield_safe: false,
            shield_reason: "INSUFFICIENT_BARS".into(),
            zl_trend: "NEUTRAL".into(),
            zl_ema: 0.0,
            zl_score: 0.0,
            zl_is_flip: false,
            squeeze_on: false,
            squeeze_fired: false,
            usable_zones_count: 0,
            s01_sweep_pool: None,
            s01_sweep_side: None,
            s02_pull_direction: "BALANCED".into(),
            s02_pull_bias: 1.0,
            s02_short_magnet: None,
            s02_long_magnet: None,
            cvd_aligned_long: false,
            cvd_aligned_short: false,
            momentum_5m_long_ok: false,
            momentum_5m_short_ok: false,
            blocker: "WARMUP_BARS_LT_55".into(),
        };
    }

    let atr_14 = atr_last_yahudi(bars_15m, 14).unwrap_or(0.0);
    let chop_14 = choppiness_last_yahudi(bars_15m, 14).unwrap_or(50.0);
    let adx_14 = adx_last_yahudi(bars_15m, 14).map(|a| a.adx).unwrap_or(20.0);
    let vwap_session = vwap_session_utc(bars_15m, None).unwrap_or(0.0);
    let shield = evaluate_volatility_shield(bars_15m, of_ctx.spread_pct);
    let shield_safe = !shield.block_new_signals;
    let shield_reason = if shield.reasons.is_empty() {
        "OK".to_string()
    } else {
        shield.reasons.join(",")
    };
    let regime = classify_regime_smoothed(bars_15m, of_ctx.spread_pct, 5);
    let is_chop_no_trade = chop_14 > 61.8 && adx_14 < 20.0;
    let regime_state = if is_chop_no_trade {
        "CHOPPY_NO_TRADE".to_string()
    } else {
        regime.regime.as_str().to_string()
    };
    let regime_reason = format!(
        "conf={:.0}%,chop={:.1},adx={:.1}",
        regime.confidence * 100.0,
        chop_14,
        adx_14
    );

    let zl = calculate_zero_lag_trend(bars_15m, 21, 1.0, TrendDir::Neutral);
    let squeeze = evaluate_squeeze_trigger(bars_15m);
    let (_, _, zones) = detect_usable_zones(bars_15m, zl.direction);
    let usable_zones_count = zones.len();

    let sweep = detect_range_sweep_reclaim(bars_15m, atr_14)
        .or_else(|| detect_session_sweep_reclaim(bars_15m, atr_14));
    let sniper = evaluate_liquidity_sniper(bars_15m, atr_14);

    let cvd_aligned_long = is_cvd_aligned(Side::Long, &of_ctx);
    let cvd_aligned_short = is_cvd_aligned(Side::Short, &of_ctx);
    let mom_long = confirm_entry_momentum_5m(bars_5m, Side::Long, 0.20);
    let mom_short = confirm_entry_momentum_5m(bars_5m, Side::Short, 0.20);

    let i = bars_15m.len() - 1;
    let s01_sig = evaluate_s01_sweep(symbol, bars_15m, i, Tf::M15, p, Some(of_ctx));
    let s02_sig = evaluate_s02_sniper(symbol, bars_15m, i, Tf::M15, p, Some(of_ctx));

    let blocker = if !shield_safe {
        format!("VOL_SHIELD:{}", shield_reason)
    } else if is_chop_no_trade {
        format!("REGIME_CHOPPY(chop={:.1},adx={:.1})", chop_14, adx_14)
    } else if s01_sig.is_some() {
        "READY_S01_SWEEP".to_string()
    } else if s02_sig.is_some() {
        "READY_S02_SNIPER".to_string()
    } else if sweep.is_some() {
        "SWEEP_BLOCKED_CVD_OR_SL_BAND".to_string()
    } else {
        format!(
            "WAIT_SETUP(zl={},pull={}:{:.2})",
            zl.direction.as_str(),
            sniper.pull_direction,
            sniper.pull_bias
        )
    };

    YahudiScalpTrace {
        regime_state,
        regime_reason,
        chop_14,
        adx_14,
        atr_14,
        vwap_session,
        shield_safe,
        shield_reason,
        zl_trend: zl.direction.as_str().to_string(),
        zl_ema: zl.zlema,
        zl_score: zl.trend_score,
        zl_is_flip: zl.is_flip,
        squeeze_on: squeeze.squeeze_on,
        squeeze_fired: squeeze.squeeze_release,
        usable_zones_count,
        s01_sweep_pool: sweep.as_ref().map(|s| s.pool_label.clone()),
        s01_sweep_side: sweep.as_ref().map(|s| format!("{:?}", s.side)),
        s02_pull_direction: sniper.pull_direction,
        s02_pull_bias: sniper.pull_bias,
        s02_short_magnet: sniper.short_magnet.map(|m| m.price),
        s02_long_magnet: sniper.long_magnet.map(|m| m.price),
        cvd_aligned_long,
        cvd_aligned_short,
        momentum_5m_long_ok: mom_long.allowed,
        momentum_5m_short_ok: mom_short.allowed,
        blocker,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_bar(
        idx: usize,
        open: f64,
        high: f64,
        low: f64,
        close: f64,
        vol: f64,
        buy_vol: f64,
    ) -> Bar {
        Bar {
            open_time: (idx as i64) * 900_000,
            open,
            high,
            low,
            close,
            volume: vol,
            quote_volume: vol * close,
            trades: 100,
            taker_buy_base: buy_vol,
        }
    }

    #[test]
    fn sl_clamping_respects_2_to_4_pct_band_and_council_floor() {
        let sl = clamp_sl_to_band(100.0, 99.2, Side::Long, 1.5, 0.805).unwrap();
        assert!((sl - 98.0).abs() < 1e-6);
        assert!(clamp_sl_to_band(100.0, 95.0, Side::Long, 1.5, 0.805).is_none());
    }

    #[test]
    fn range_sweep_reclaim_detects_bullish_sweep() {
        let mut bars: Vec<Bar> = (0..35)
            .map(|i| make_bar(i, 100.0, 101.0, 99.0, 100.2, 1000.0, 520.0))
            .collect();
        bars.push(make_bar(35, 99.1, 99.8, 98.2, 99.5, 1800.0, 1200.0));
        let atr = atr_last_yahudi(&bars, 14).unwrap_or(1.0);
        let sweep = detect_range_sweep_reclaim(&bars, atr).expect("should detect sweep");
        assert_eq!(sweep.side, Side::Long);
        assert_eq!(sweep.pool_label, "RANGE_30B_LOW");
    }

    #[test]
    fn liquidity_sniper_computes_magnets_and_bias() {
        let bars: Vec<Bar> = (0..65)
            .map(|i| {
                let base = 100.0 + (i as f64) * 0.25;
                make_bar(
                    i,
                    base,
                    base * 1.006,
                    base * 0.994,
                    base * 1.002,
                    1500.0,
                    950.0,
                )
            })
            .collect();
        let atr = atr_last_yahudi(&bars, 14).unwrap_or(1.0);
        let snap = evaluate_liquidity_sniper(&bars, atr);
        assert!(snap.current_price > 100.0);
        assert!(snap.pull_bias.is_finite());
    }
}
