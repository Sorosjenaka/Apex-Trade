use serde::{Deserialize, Serialize};

/// Epoch milliseconds UTC.
pub type Ms = i64;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
pub enum Side {
    Long,
    Short,
}

impl Side {
    pub fn sign(self) -> f64 {
        match self {
            Side::Long => 1.0,
            Side::Short => -1.0,
        }
    }
    pub fn opposite(self) -> Side {
        match self {
            Side::Long => Side::Short,
            Side::Short => Side::Long,
        }
    }
    /// Binance order side for opening this position.
    pub fn entry_order_side(self) -> &'static str {
        match self {
            Side::Long => "BUY",
            Side::Short => "SELL",
        }
    }
    /// Binance order side for closing this position.
    pub fn exit_order_side(self) -> &'static str {
        match self {
            Side::Long => "SELL",
            Side::Short => "BUY",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
pub enum Tf {
    M1,
    M5,
    M15,
    H1,
    H4,
    D1,
}

impl Tf {
    pub fn ms(self) -> Ms {
        match self {
            Tf::M1 => 60_000,
            Tf::M5 => 300_000,
            Tf::M15 => 900_000,
            Tf::H1 => 3_600_000,
            Tf::H4 => 14_400_000,
            Tf::D1 => 86_400_000,
        }
    }
    pub fn binance(self) -> &'static str {
        match self {
            Tf::M1 => "1m",
            Tf::M5 => "5m",
            Tf::M15 => "15m",
            Tf::H1 => "1h",
            Tf::H4 => "4h",
            Tf::D1 => "1d",
        }
    }
    pub fn parse(s: &str) -> Option<Tf> {
        Some(match s {
            "1m" => Tf::M1,
            "5m" => Tf::M5,
            "15m" => Tf::M15,
            "1h" => Tf::H1,
            "4h" => Tf::H4,
            "1d" => Tf::D1,
            _ => return None,
        })
    }
}

/// Bar OHLCV (Binance klines). `f64` hanya untuk indikator/fitur (Rules §2).
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct Bar {
    pub open_time: Ms,
    pub open: f64,
    pub high: f64,
    pub low: f64,
    pub close: f64,
    pub volume: f64,
    pub quote_volume: f64,
    pub trades: u64,
    pub taker_buy_base: f64,
}

impl Bar {
    /// Bar boleh dipakai hanya bila `available_at(tf) <= as_of` (RD-00.1).
    pub fn available_at(&self, tf: Tf) -> Ms {
        self.open_time + tf.ms()
    }
    /// delta = 2 × takerBuyBase − volume (STRATEGIES §0).
    pub fn delta(&self) -> f64 {
        2.0 * self.taker_buy_base - self.volume
    }
    pub fn range(&self) -> f64 {
        self.high - self.low
    }
    pub fn body(&self) -> f64 {
        (self.close - self.open).abs()
    }
    /// Posisi close dalam range bar (0 = low, 1 = high).
    pub fn close_pos(&self) -> f64 {
        let r = self.range();
        if r <= 0.0 {
            0.5
        } else {
            (self.close - self.low) / r
        }
    }
    pub fn is_valid(&self) -> bool {
        self.open.is_finite()
            && self.high.is_finite()
            && self.low.is_finite()
            && self.close.is_finite()
            && self.high >= self.low
            && self.high >= self.open.max(self.close)
            && self.low <= self.open.min(self.close)
            && self.low > 0.0
    }
}

/// Alasan SKIP — urutan gate tetap (Rules §4.2).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
pub enum SkipReason {
    NotArmed,
    Regime,
    RegimeSide,
    SlRange,
    MaxPositions,
    SymbolBusy,
    Cooldown,
    DailyStop,
    EquityFloor,
    Spread,
    Size,
    NotAllowlisted,
    VolumeGate,
    DataInvalid,
    StrategyNotGo,
}

impl SkipReason {
    pub fn code(self) -> &'static str {
        match self {
            SkipReason::NotArmed => "SKIP_NOT_ARMED",
            SkipReason::Regime => "SKIP_REGIME",
            SkipReason::RegimeSide => "REGIME_SIDE",
            SkipReason::SlRange => "SKIP_SL_RANGE",
            SkipReason::MaxPositions => "SKIP_MAX_POSITIONS",
            SkipReason::SymbolBusy => "SKIP_SYMBOL_BUSY",
            SkipReason::Cooldown => "SKIP_COOLDOWN",
            SkipReason::DailyStop => "SKIP_DAILY_STOP",
            SkipReason::EquityFloor => "SKIP_EQUITY_FLOOR",
            SkipReason::Spread => "SKIP_SPREAD",
            SkipReason::Size => "SKIP_SIZE",
            SkipReason::NotAllowlisted => "SKIP_NOT_ALLOWLISTED",
            SkipReason::VolumeGate => "SKIP_VOLUME_GATE",
            SkipReason::DataInvalid => "SKIP_DATA_INVALID",
            SkipReason::StrategyNotGo => "SKIP_STRATEGY_NOT_GO",
        }
    }
}

impl std::fmt::Display for SkipReason {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.code())
    }
}
