use apex_core::types::{Ms, Side};
use rust_decimal::Decimal;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, thiserror::Error)]
pub enum ExError {
    #[error("binance error {code}: {msg} (http {http})")]
    Api { code: i64, msg: String, http: u16 },
    #[error("rate limited (banned={banned}), retry after {retry_after_ms} ms")]
    RateLimited { retry_after_ms: u64, banned: bool },
    #[error("network: {0}")]
    Network(String),
    /// Timeout / 5xx: status order TIDAK diketahui → wajib query by clientOrderId.
    #[error("unknown outcome: {0}")]
    Unknown(String),
    #[error("blocked by safety guard: {0}")]
    Safety(String),
    #[error("not found")]
    NotFound,
}

impl ExError {
    pub fn code(&self) -> Option<i64> {
        match self {
            ExError::Api { code, .. } => Some(*code),
            _ => None,
        }
    }
    /// -4046 "No need to change margin type", -4059 "No need to change position side".
    pub fn is_no_change(&self) -> bool {
        matches!(self.code(), Some(-4046) | Some(-4059))
    }
    /// -2021 "Order would immediately trigger".
    pub fn would_trigger(&self) -> bool {
        self.code() == Some(-2021)
    }
    /// -1021 timestamp di luar recvWindow.
    pub fn is_timestamp(&self) -> bool {
        self.code() == Some(-1021)
    }
    /// -5022 GTX/post-only ditolak karena akan langsung match.
    pub fn is_post_only_reject(&self) -> bool {
        self.code() == Some(-5022)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum OrderStatus {
    New,
    PartiallyFilled,
    Filled,
    Canceled,
    Expired,
    Rejected,
}

impl OrderStatus {
    pub fn parse(s: &str) -> OrderStatus {
        match s {
            "PARTIALLY_FILLED" => OrderStatus::PartiallyFilled,
            "FILLED" => OrderStatus::Filled,
            "CANCELED" => OrderStatus::Canceled,
            "EXPIRED" | "EXPIRED_IN_MATCH" => OrderStatus::Expired,
            "REJECTED" => OrderStatus::Rejected,
            _ => OrderStatus::New,
        }
    }
    pub fn is_final(self) -> bool {
        matches!(self, OrderStatus::Filled | OrderStatus::Canceled | OrderStatus::Expired | OrderStatus::Rejected)
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub enum OrderKind {
    Market,
    /// LIMIT timeInForce=GTX (post-only).
    LimitGtx { price: Decimal },
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct OrderRequest {
    pub symbol: String,
    /// "BUY" | "SELL"
    pub side: String,
    pub kind: OrderKind,
    pub qty: Decimal,
    pub reduce_only: bool,
    pub client_id: String,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct OrderAck {
    pub symbol: String,
    pub order_id: i64,
    pub client_id: String,
    pub status: OrderStatus,
    pub side: String,
    pub orig_qty: Decimal,
    pub executed_qty: Decimal,
    pub avg_price: Decimal,
    pub reduce_only: bool,
    pub update_ms: Ms,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum AlgoKind {
    StopMarket,
    TakeProfitMarket,
}
impl AlgoKind {
    pub fn binance(self) -> &'static str {
        match self {
            AlgoKind::StopMarket => "STOP_MARKET",
            AlgoKind::TakeProfitMarket => "TAKE_PROFIT_MARKET",
        }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct AlgoRequest {
    pub symbol: String,
    /// sisi exit: "SELL" untuk long, "BUY" untuk short
    pub side: String,
    pub kind: AlgoKind,
    pub trigger: Decimal,
    /// true = closePosition (tanpa qty). false = qty + reduceOnly.
    pub close_position: bool,
    pub qty: Option<Decimal>,
    /// maks 36 char
    pub client_algo_id: String,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct AlgoAck {
    pub algo_id: i64,
    pub client_algo_id: String,
    pub symbol: String,
    pub side: String,
    pub order_type: String,
    pub status: String,
    pub trigger: Decimal,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct PositionInfo {
    pub symbol: String,
    /// positif = long, negatif = short (one-way)
    pub amt: Decimal,
    pub entry_price: Decimal,
    pub mark_price: Decimal,
    pub margin_type: String,
    pub leverage: u32,
}

impl PositionInfo {
    pub fn side(&self) -> Option<Side> {
        if self.amt > Decimal::ZERO {
            Some(Side::Long)
        } else if self.amt < Decimal::ZERO {
            Some(Side::Short)
        } else {
            None
        }
    }
}

/// Event user-data stream yang relevan.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub enum UserEvent {
    Order(OrderUpdate),
    Algo { symbol: String, algo_id: i64, client_algo_id: String, status: String, ts: Ms },
    Account { positions: Vec<(String, Decimal, Decimal)>, ts: Ms },
    ListenKeyExpired,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct OrderUpdate {
    pub symbol: String,
    pub client_id: String,
    pub order_id: i64,
    pub side: String,
    /// tipe original (ot): LIMIT / MARKET / STOP_MARKET / TAKE_PROFIT_MARKET
    pub orig_type: String,
    pub status: OrderStatus,
    pub exec_type: String,
    pub last_qty: Decimal,
    pub cum_qty: Decimal,
    pub last_price: Decimal,
    pub avg_price: Decimal,
    pub fee: Decimal,
    pub realized_pnl: Decimal,
    pub reduce_only: bool,
    pub close_position: bool,
    pub ts: Ms,
}

/// clientOrderId sesuai regex Binance `^[\.A-Z\:/a-z0-9_-]{1,36}$`.
pub fn make_client_id(prefix: &str, symbol: &str, ts: Ms, seq: u64) -> String {
    let sym: String = symbol.trim_end_matches("USDT").chars().filter(|c| c.is_ascii_alphanumeric()).take(10).collect();
    let id = format!("apx{prefix}-{sym}-{}-{seq}", to_base36(ts as u64));
    id.chars().take(36).collect()
}

fn to_base36(mut n: u64) -> String {
    const D: &[u8] = b"0123456789abcdefghijklmnopqrstuvwxyz";
    if n == 0 {
        return "0".into();
    }
    let mut v = Vec::new();
    while n > 0 {
        v.push(D[(n % 36) as usize]);
        n /= 36;
    }
    v.reverse();
    String::from_utf8(v).unwrap_or_default()
}

pub fn valid_client_id(s: &str) -> bool {
    !s.is_empty() && s.len() <= 36 && s.chars().all(|c| c.is_ascii_alphanumeric() || ".:/_-".contains(c))
}
