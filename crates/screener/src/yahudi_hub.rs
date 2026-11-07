//! Port 1:1 Sub-sistem Market Data Hub, Macro Gate 1h, Interleaved Scanner,
//! Orderbook Wall Lifelines (`PULLED` vs `EATEN`), Size-Tier Flow (`MEGA..SMALL`),
//! dan Sentiment Crowding dari `Yahudi-Bot`.

use apex_core::types::{Bar, Ms, Side};
use apex_indicators::yahudi::{
    adx_last_yahudi, calculate_zero_lag_trend, choppiness_last_yahudi, classify_regime_smoothed,
    MarketRegime, TrendDir,
};
use serde::{Deserialize, Serialize};

// ============================================================================
// 1. Macro Gate 1h (BTCUSDT, ETHUSDT, BTCDOMUSDT)
// ============================================================================

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct MacroGateSnapshot {
    pub btc_regime: String,
    pub btc_trend: String,
    pub eth_regime: String,
    pub eth_trend: String,
    pub btcdom_change_1h_pct: f64,
    pub btcdom_spike: bool,
    pub altcoins_allowed: bool,
    pub macro_bias: String, // "BULLISH", "BEARISH", "NEUTRAL", "RISK_OFF"
    pub reason: String,
}

impl Default for MacroGateSnapshot {
    fn default() -> Self {
        Self {
            btc_regime: "TRENDING".to_string(),
            btc_trend: "BULLISH".to_string(),
            eth_regime: "TRENDING".to_string(),
            eth_trend: "BULLISH".to_string(),
            btcdom_change_1h_pct: 0.0,
            btcdom_spike: false,
            altcoins_allowed: true,
            macro_bias: "NEUTRAL".to_string(),
            reason: "DEFAULT_OPEN".to_string(),
        }
    }
}

/// Evaluasi Macro Gate 1h (`BTCUSDT`, `ETHUSDT`, `BTCDOMUSDT`).
/// Jika `BTCDOMUSDT` naik `> +0.5%` dalam 1 jam terakhir, entry altcoin diblokir (`altcoins_allowed = false`).
pub fn evaluate_macro_gate_1h(
    btc_1h: &[Bar],
    eth_1h: &[Bar],
    btcdom_1h: &[Bar],
) -> MacroGateSnapshot {
    let btc_reg = if btc_1h.len() >= 30 {
        classify_regime_smoothed(btc_1h, 0.0002, 5).regime
    } else {
        MarketRegime::Ranging
    };
    let btc_zl = calculate_zero_lag_trend(btc_1h, 21, 1.0, TrendDir::Neutral);
    let btc_chop = choppiness_last_yahudi(btc_1h, 14).unwrap_or(50.0);
    let btc_adx = adx_last_yahudi(btc_1h, 14).map(|a| a.adx).unwrap_or(25.0);

    let eth_reg = if eth_1h.len() >= 30 {
        classify_regime_smoothed(eth_1h, 0.0002, 5).regime
    } else {
        MarketRegime::Ranging
    };
    let eth_zl = calculate_zero_lag_trend(eth_1h, 21, 1.0, TrendDir::Neutral);
    let eth_chop = choppiness_last_yahudi(eth_1h, 14).unwrap_or(50.0);
    let eth_adx = adx_last_yahudi(eth_1h, 14).map(|a| a.adx).unwrap_or(25.0);

    let btcdom_change_1h_pct = if btcdom_1h.len() >= 2 {
        let last = &btcdom_1h[btcdom_1h.len() - 1];
        let prev = &btcdom_1h[btcdom_1h.len() - 2];
        if prev.close > 0.0 {
            (last.close - prev.close) / prev.close
        } else {
            0.0
        }
    } else {
        0.0
    };

    // BTCDOM spike > +0.5% (0.005) dalam 1 jam menyedot likuiditas altcoin
    let btcdom_spike = btcdom_change_1h_pct > 0.005;
    let btc_extreme_chop = (btc_chop > 61.8 && btc_adx < 20.0) && (eth_chop > 61.8 && eth_adx < 20.0);
    let altcoins_allowed = !btcdom_spike && !btc_extreme_chop;

    let macro_bias = if btc_reg == MarketRegime::Volatile || btcdom_spike {
        "RISK_OFF"
    } else if btc_zl.direction == TrendDir::Bullish && eth_zl.direction == TrendDir::Bullish {
        "BULLISH"
    } else if btc_zl.direction == TrendDir::Bearish && eth_zl.direction == TrendDir::Bearish {
        "BEARISH"
    } else {
        "NEUTRAL"
    };

    let reason = if btcdom_spike {
        format!("BTCDOM_SPIKE_1H(+{:.2}%)", btcdom_change_1h_pct * 100.0)
    } else if btc_extreme_chop {
        "BTC_ETH_DOUBLE_CHOP_1H".to_string()
    } else {
        format!("MACRO_OK(btc={},eth={})", btc_zl.direction.as_str(), eth_zl.direction.as_str())
    };

    MacroGateSnapshot {
        btc_regime: btc_reg.as_str().to_string(),
        btc_trend: btc_zl.direction.as_str().to_string(),
        eth_regime: eth_reg.as_str().to_string(),
        eth_trend: eth_zl.direction.as_str().to_string(),
        btcdom_change_1h_pct,
        btcdom_spike,
        altcoins_allowed,
        macro_bias: macro_bias.to_string(),
        reason,
    }
}

/// Cek apakah simbol diizinkan oleh Macro Gate 1h.
pub fn is_symbol_allowed_by_macro_gate(symbol: &str, gate: &MacroGateSnapshot) -> bool {
    let is_major = matches!(symbol, "BTCUSDT" | "ETHUSDT");
    if is_major {
        true
    } else {
        gate.altcoins_allowed
    }
}

// ============================================================================
// 2. Loser Drop Cap (-15%) & Interleaved Top Gainers/Losers Scanner (mtfTopN = 48)
// ============================================================================

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Ticker24hMover {
    pub symbol: String,
    pub last_price: f64,
    pub price_change_pct_24h: f64, // dalam persen, misal -12.5 atau +18.4
    pub quote_volume_24h: f64,
}

/// Proteksi Loser Drop Cap (`maxLoserDropPct = 15%` di `Yahudi-Bot`):
/// Menolak koin yang jatuh lebih dalam dari `-15.0%` dalam 24 jam terakhir (anti-falling-knife/delisting).
pub fn loser_drop_allowed(price_change_pct_24h: f64, max_loser_drop_pct: f64) -> bool {
    let cap = if max_loser_drop_pct > 0.0 {
        max_loser_drop_pct
    } else {
        15.0
    };
    price_change_pct_24h >= -cap
}

/// Interleaved Top Gainers & Top Losers (`mtfTopN = 48`):
/// Menyusun daftar simbol secara selang-seling (`Gainer #1, Loser #1, Gainer #2, Loser #2, ...`)
/// agar scanner multi-timeframe mendapatkan eksposur seimbang untuk setup LONG maupun SHORT.
pub fn rank_movers_interleaved(
    tickers: &[Ticker24hMover],
    min_quote_vol_usd: f64,
    max_loser_drop_pct: f64,
    top_n: usize,
) -> Vec<Ticker24hMover> {
    let filtered: Vec<Ticker24hMover> = tickers
        .iter()
        .filter(|t| {
            t.symbol.ends_with("USDT")
                && t.quote_volume_24h >= min_quote_vol_usd
                && loser_drop_allowed(t.price_change_pct_24h, max_loser_drop_pct)
        })
        .cloned()
        .collect();

    let mut gainers: Vec<Ticker24hMover> = filtered
        .iter()
        .filter(|t| t.price_change_pct_24h >= 0.0)
        .cloned()
        .collect();
    gainers.sort_by(|a, b| b.price_change_pct_24h.total_cmp(&a.price_change_pct_24h));

    let mut losers: Vec<Ticker24hMover> = filtered
        .iter()
        .filter(|t| t.price_change_pct_24h < 0.0)
        .cloned()
        .collect();
    losers.sort_by(|a, b| a.price_change_pct_24h.total_cmp(&b.price_change_pct_24h));

    let mut out = Vec::with_capacity(top_n);
    let max_len = gainers.len().max(losers.len());
    for i in 0..max_len {
        if out.len() >= top_n {
            break;
        }
        if let Some(g) = gainers.get(i) {
            out.push(g.clone());
        }
        if out.len() >= top_n {
            break;
        }
        if let Some(l) = losers.get(i) {
            out.push(l.clone());
        }
    }
    out
}

// ============================================================================
// 3. Wall Lifelines Tracker (APPEARED, MOVED, PULLED Spoof vs EATEN Absorption)
// ============================================================================

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum WallStatus {
    Active,
    Pulled,
    Eaten,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct TrackedWall {
    pub side: Side, // Long = BID wall, Short = ASK wall
    pub price: f64,
    pub notional_usd: f64,
    pub peak_notional_usd: f64,
    pub first_seen_ms: Ms,
    pub last_seen_ms: Ms,
    pub status: WallStatus,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct WallLifelinesSummary {
    pub active_bid_walls: usize,
    pub active_ask_walls: usize,
    pub pulled_count: usize,
    pub eaten_count: usize,
    pub spoof_score: f64, // 0..1 (pulled / (pulled + eaten))
}

#[derive(Clone, Debug, Default)]
pub struct WallLifelinesTracker {
    pub walls: Vec<TrackedWall>,
    pub pulled_count: usize,
    pub eaten_count: usize,
}

impl WallLifelinesTracker {
    /// Update state wall lifelines dari snapshot L2 orderbook (`bids`, `asks` berisi `(price, qty)`).
    pub fn on_book_snapshot(
        &mut self,
        now_ms: Ms,
        mid_price: f64,
        bids: &[(f64, f64)],
        asks: &[(f64, f64)],
        min_wall_usd: f64,
    ) {
        if mid_price <= 0.0 {
            return;
        }
        let tol_pct = 0.0005; // 5 bps

        // Cek wall aktif yang hilang dari orderbook
        for w in &mut self.walls {
            if w.status != WallStatus::Active {
                continue;
            }
            let levels = match w.side {
                Side::Long => bids,
                Side::Short => asks,
            };
            let matched = levels
                .iter()
                .find(|(p, _)| ((*p - w.price).abs() / w.price) <= tol_pct);
            if let Some(&(p, q)) = matched {
                let notional = p * q;
                if notional >= min_wall_usd * 0.5 {
                    w.price = p;
                    w.notional_usd = notional;
                    w.peak_notional_usd = w.peak_notional_usd.max(notional);
                    w.last_seen_ms = now_ms;
                    continue;
                }
            }
            // Wall hilang: bedakan EATEN (harga menembus level wall) vs PULLED (dicabut sebelum tersentuh)
            let eaten = match w.side {
                Side::Long => mid_price <= w.price * (1.0 + tol_pct),
                Side::Short => mid_price >= w.price * (1.0 - tol_pct),
            };
            if eaten {
                w.status = WallStatus::Eaten;
                self.eaten_count += 1;
            } else {
                w.status = WallStatus::Pulled;
                self.pulled_count += 1;
            }
            w.last_seen_ms = now_ms;
        }

        // Tambahkan wall baru yang melampaui min_wall_usd
        for (side, levels) in [(Side::Long, bids), (Side::Short, asks)] {
            for &(p, q) in levels.iter().take(20) {
                let notional = p * q;
                if notional < min_wall_usd {
                    continue;
                }
                let exists = self.walls.iter().any(|w| {
                    w.status == WallStatus::Active
                        && w.side == side
                        && ((w.price - p).abs() / p) <= tol_pct
                });
                if !exists {
                    self.walls.push(TrackedWall {
                        side,
                        price: p,
                        notional_usd: notional,
                        peak_notional_usd: notional,
                        first_seen_ms: now_ms,
                        last_seen_ms: now_ms,
                        status: WallStatus::Active,
                    });
                }
            }
        }

        // Batasi histori wall maksimal 64 item
        if self.walls.len() > 64 {
            let drain = self.walls.len() - 64;
            self.walls.drain(0..drain);
        }
    }

    pub fn summary(&self) -> WallLifelinesSummary {
        let active_bid_walls = self
            .walls
            .iter()
            .filter(|w| w.status == WallStatus::Active && w.side == Side::Long)
            .count();
        let active_ask_walls = self
            .walls
            .iter()
            .filter(|w| w.status == WallStatus::Active && w.side == Side::Short)
            .count();
        let total_resolved = self.pulled_count + self.eaten_count;
        let spoof_score = if total_resolved > 0 {
            self.pulled_count as f64 / total_resolved as f64
        } else {
            0.0
        };
        WallLifelinesSummary {
            active_bid_walls,
            active_ask_walls,
            pulled_count: self.pulled_count,
            eaten_count: self.eaten_count,
            spoof_score,
        }
    }
}

// ============================================================================
// 4. Size-Tier Flow Tracker (MEGA / WHALE / LARGE / MEDIUM / SMALL)
// ============================================================================

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum SizeTier {
    Mega,   // >= $250,000
    Whale,  // >= $50,000
    Large,  // >= $10,000
    Medium, // >= $1,000
    Small,  // < $1,000
}

pub fn classify_trade_tier(notional_usd: f64) -> SizeTier {
    if notional_usd >= 250_000.0 {
        SizeTier::Mega
    } else if notional_usd >= 50_000.0 {
        SizeTier::Whale
    } else if notional_usd >= 10_000.0 {
        SizeTier::Large
    } else if notional_usd >= 1_000.0 {
        SizeTier::Medium
    } else {
        SizeTier::Small
    }
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct TierFlowSummary {
    pub whale_net_usd: f64,
    pub retail_net_usd: f64,
    pub whale_retail_bias: String, // "WHALE_ACCUMULATING", "WHALE_DISTRIBUTING", "ALIGNED_BULL", "ALIGNED_BEAR", "NEUTRAL"
    pub absorption_alert: Option<String>,
}

#[derive(Clone, Debug, Default)]
pub struct TierFlowTracker {
    pub mega_delta: f64,
    pub whale_delta: f64,
    pub large_delta: f64,
    pub medium_delta: f64,
    pub small_delta: f64,
}

impl TierFlowTracker {
    pub fn on_agg_trade(&mut self, price: f64, qty: f64, is_buyer_maker: bool) {
        let notional = price * qty;
        // is_buyer_maker == true berarti агреsor adalah SELL
        let signed = if is_buyer_maker { -notional } else { notional };
        match classify_trade_tier(notional) {
            SizeTier::Mega => self.mega_delta += signed,
            SizeTier::Whale => self.whale_delta += signed,
            SizeTier::Large => self.large_delta += signed,
            SizeTier::Medium => self.medium_delta += signed,
            SizeTier::Small => self.small_delta += signed,
        }
    }

    pub fn decay(&mut self, factor: f64) {
        let f = factor.clamp(0.0, 1.0);
        self.mega_delta *= f;
        self.whale_delta *= f;
        self.large_delta *= f;
        self.medium_delta *= f;
        self.small_delta *= f;
    }

    pub fn summary(&self) -> TierFlowSummary {
        let whale_net_usd = self.mega_delta + self.whale_delta + self.large_delta;
        let retail_net_usd = self.medium_delta + self.small_delta;

        let (bias, alert) = if whale_net_usd > 25_000.0 && retail_net_usd < -5_000.0 {
            (
                "WHALE_ACCUMULATING",
                Some("WHALE_ABSORPTION_BULL".to_string()),
            )
        } else if whale_net_usd < -25_000.0 && retail_net_usd > 5_000.0 {
            (
                "WHALE_DISTRIBUTING",
                Some("WHALE_ABSORPTION_BEAR".to_string()),
            )
        } else if whale_net_usd > 10_000.0 && retail_net_usd >= 0.0 {
            ("ALIGNED_BULL", None)
        } else if whale_net_usd < -10_000.0 && retail_net_usd <= 0.0 {
            ("ALIGNED_BEAR", None)
        } else {
            ("NEUTRAL", None)
        };

        TierFlowSummary {
            whale_net_usd,
            retail_net_usd,
            whale_retail_bias: bias.to_string(),
            absorption_alert: alert,
        }
    }
}

// ============================================================================
// 5. Sentiment & Crowding Classifier (Fear & Greed + Long/Short Ratio)
// ============================================================================

pub fn classify_fear_greed(val: u32) -> &'static str {
    match val {
        0..=24 => "EXTREME_FEAR",
        25..=44 => "FEAR",
        45..=55 => "NEUTRAL",
        56..=75 => "GREED",
        _ => "EXTREME_GREED",
    }
}

pub fn classify_crowding(long_short_ratio: f64) -> &'static str {
    if long_short_ratio >= 1.8 {
        "CROWDED_LONG"
    } else if long_short_ratio <= 0.65 {
        "CROWDED_SHORT"
    } else {
        "BALANCED"
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn loser_drop_cap_blocks_beyond_minus_15_pct() {
        assert!(loser_drop_allowed(-14.9, 15.0));
        assert!(!loser_drop_allowed(-15.5, 15.0));
    }

    #[test]
    fn interleaved_movers_alternates_gainers_and_losers() {
        let tickers = vec![
            Ticker24hMover {
                symbol: "AAAUSDT".into(),
                last_price: 10.0,
                price_change_pct_24h: 25.0,
                quote_volume_24h: 100_000_000.0,
            },
            Ticker24hMover {
                symbol: "BBBUSDT".into(),
                last_price: 5.0,
                price_change_pct_24h: -8.0,
                quote_volume_24h: 90_000_000.0,
            },
            Ticker24hMover {
                symbol: "CCCUSDT".into(),
                last_price: 1.0,
                price_change_pct_24h: -19.0, // Exceeds -15% cap -> filtered out
                quote_volume_24h: 120_000_000.0,
            },
            Ticker24hMover {
                symbol: "DDDUSDT".into(),
                last_price: 2.0,
                price_change_pct_24h: 12.0,
                quote_volume_24h: 80_000_000.0,
            },
        ];
        let ranked = rank_movers_interleaved(&tickers, 50_000_000.0, 15.0, 48);
        assert_eq!(ranked.len(), 3);
        assert_eq!(ranked[0].symbol, "AAAUSDT");
        assert_eq!(ranked[1].symbol, "BBBUSDT");
        assert_eq!(ranked[2].symbol, "DDDUSDT");
    }

    #[test]
    fn tier_flow_detects_whale_accumulation() {
        let mut tracker = TierFlowTracker::default();
        // Whale buy $100k (is_buyer_maker = false)
        tracker.on_agg_trade(100.0, 1000.0, false);
        // Retail sell $8k (is_buyer_maker = true)
        for _ in 0..10 {
            tracker.on_agg_trade(100.0, 8.0, true);
        }
        let sum = tracker.summary();
        assert_eq!(sum.whale_retail_bias, "WHALE_ACCUMULATING");
        assert_eq!(sum.absorption_alert.as_deref(), Some("WHALE_ABSORPTION_BULL"));
    }
}
