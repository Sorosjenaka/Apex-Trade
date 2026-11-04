//! Klien REST signed Binance USDⓈ-M (HMAC-SHA256, recvWindow, sinkron waktu, backoff weight/429/418).
//! TIDAK pernah dipakai di test (test memakai MockExchange). Tidak pernah me-log key/signature.
use crate::creds::Credentials;
use crate::exchange::FuturesExchange;
use crate::safety::{check_base, ExecMode, SafetyError};
use crate::types::*;
use apex_core::num::fmt_dec;
use apex_core::time::now_ms;
use apex_core::types::Ms;
use rust_decimal::Decimal;
use serde_json::Value;
use std::str::FromStr;
use std::sync::atomic::{AtomicI64, AtomicU64, Ordering};
use std::time::Duration;

pub struct BinanceRest {
    http: reqwest::Client,
    base: String,
    creds: Credentials,
    recv_window: u64,
    offset_ms: AtomicI64,
    used_weight: AtomicU64,
    banned_until: AtomicI64,
}

fn dec(v: &Value) -> Decimal {
    match v {
        Value::String(s) => Decimal::from_str(s).unwrap_or_default(),
        Value::Number(n) => Decimal::from_str(&n.to_string()).unwrap_or_default(),
        _ => Decimal::ZERO,
    }
}
fn i64v(v: &Value) -> i64 {
    v.as_i64().or_else(|| v.as_str().and_then(|s| s.parse().ok())).unwrap_or(0)
}
fn s(v: &Value) -> String {
    v.as_str().unwrap_or_default().to_string()
}

pub fn parse_order(v: &Value) -> OrderAck {
    OrderAck {
        symbol: s(&v["symbol"]),
        order_id: i64v(&v["orderId"]),
        client_id: s(&v["clientOrderId"]),
        status: OrderStatus::parse(v["status"].as_str().unwrap_or("NEW")),
        side: s(&v["side"]),
        orig_qty: dec(&v["origQty"]),
        executed_qty: dec(&v["executedQty"]),
        avg_price: dec(&v["avgPrice"]),
        reduce_only: v["reduceOnly"].as_bool().unwrap_or(false),
        update_ms: i64v(&v["updateTime"]),
    }
}

pub fn parse_algo(v: &Value) -> AlgoAck {
    AlgoAck {
        algo_id: i64v(&v["algoId"]),
        client_algo_id: s(&v["clientAlgoId"]),
        symbol: s(&v["symbol"]),
        side: s(&v["side"]),
        order_type: s(if v["orderType"].is_string() { &v["orderType"] } else { &v["type"] }),
        status: s(&v["algoStatus"]),
        trigger: dec(&v["triggerPrice"]),
    }
}

pub fn order_params(r: &OrderRequest) -> Vec<(&'static str, String)> {
    let mut p = vec![("symbol", r.symbol.clone()), ("side", r.side.clone())];
    match &r.kind {
        OrderKind::Market => {
            p.push(("type", "MARKET".into()));
        }
        OrderKind::LimitGtx { price } => {
            p.push(("type", "LIMIT".into()));
            p.push(("timeInForce", "GTX".into()));
            p.push(("price", fmt_dec(*price)));
        }
    }
    p.push(("quantity", fmt_dec(r.qty)));
    if r.reduce_only {
        p.push(("reduceOnly", "true".into()));
    }
    p.push(("newClientOrderId", r.client_id.clone()));
    // RESULT → MARKET mengembalikan status final (FILLED) bila sudah terisi; LIMIT tetap NEW (ACK).
    p.push(("newOrderRespType", "RESULT".into()));
    p
}

pub fn algo_params(r: &AlgoRequest) -> Vec<(&'static str, String)> {
    let mut p = vec![
        ("algoType", "CONDITIONAL".to_string()),
        ("symbol", r.symbol.clone()),
        ("side", r.side.clone()),
        ("type", r.kind.binance().to_string()),
        ("triggerPrice", fmt_dec(r.trigger)),
        ("workingType", "MARK_PRICE".to_string()),
        ("priceProtect", "TRUE".to_string()),
        ("clientAlgoId", r.client_algo_id.clone()),
    ];
    if r.close_position {
        p.push(("closePosition", "true".into()));
    } else if let Some(q) = r.qty {
        p.push(("quantity", fmt_dec(q)));
        p.push(("reduceOnly", "true".into()));
    }
    p
}

pub fn encode(params: &[(&str, String)]) -> String {
    params
        .iter()
        .map(|(k, v)| {
            let enc: String = v
                .bytes()
                .map(|b| if b.is_ascii_alphanumeric() || b"-_.~".contains(&b) { (b as char).to_string() } else { format!("%{b:02X}") })
                .collect();
            format!("{k}={enc}")
        })
        .collect::<Vec<_>>()
        .join("&")
}

impl BinanceRest {
    /// `mode` harus hasil [`crate::safety::authorize`]; base harus cocok dengan mode.
    pub fn new(mode: ExecMode, base: &str, creds: Credentials, recv_window: u64) -> Result<Self, SafetyError> {
        check_base(mode, base)?;
        apex_data::init_tls();
        let http = reqwest::Client::builder().timeout(Duration::from_secs(10)).build().map_err(|e| SafetyError::Killed(e.to_string()))?;
        Ok(BinanceRest {
            http,
            base: base.trim_end_matches('/').to_string(),
            creds,
            recv_window,
            offset_ms: AtomicI64::new(0),
            used_weight: AtomicU64::new(0),
            banned_until: AtomicI64::new(0),
        })
    }

    pub fn offset_ms(&self) -> i64 {
        self.offset_ms.load(Ordering::Relaxed)
    }

    /// Sinkron waktu: offset = serverTime − titik tengah lokal.
    pub async fn sync_time(&self) -> Result<i64, ExError> {
        let t0 = now_ms();
        let v = self.public("/fapi/v1/time").await?;
        let t1 = now_ms();
        let off = i64v(&v["serverTime"]) - (t0 + t1) / 2;
        self.offset_ms.store(off, Ordering::Relaxed);
        Ok(off)
    }

    async fn public(&self, path: &str) -> Result<Value, ExError> {
        let r = self.http.get(format!("{}{}", self.base, path)).send().await.map_err(|e| ExError::Network(e.to_string()))?;
        self.handle(r).await
    }

    async fn handle(&self, r: reqwest::Response) -> Result<Value, ExError> {
        let st = r.status().as_u16();
        if let Some(w) = r.headers().get("x-mbx-used-weight-1m").and_then(|h| h.to_str().ok()).and_then(|s| s.parse::<u64>().ok()) {
            self.used_weight.store(w, Ordering::Relaxed);
        }
        let retry = r.headers().get("retry-after").and_then(|h| h.to_str().ok()).and_then(|s| s.parse::<u64>().ok()).unwrap_or(60) * 1000;
        let body = r.text().await.map_err(|e| ExError::Network(e.to_string()))?;
        if st == 429 || st == 418 {
            if st == 418 {
                self.banned_until.store(now_ms() + retry as i64, Ordering::Relaxed);
            }
            return Err(ExError::RateLimited { retry_after_ms: retry, banned: st == 418 });
        }
        if st >= 500 {
            return Err(ExError::Unknown(format!("http {st}")));
        }
        let v: Value = serde_json::from_str(&body).map_err(|e| ExError::Network(format!("json: {e}")))?;
        if st >= 400 || (v.get("code").and_then(|c| c.as_i64()).is_some_and(|c| c < 0)) {
            return Err(ExError::Api { code: v["code"].as_i64().unwrap_or(0), msg: s(&v["msg"]), http: st });
        }
        Ok(v)
    }

    async fn signed(&self, method: reqwest::Method, path: &str, params: &[(&str, String)]) -> Result<Value, ExError> {
        if now_ms() < self.banned_until.load(Ordering::Relaxed) {
            return Err(ExError::RateLimited { retry_after_ms: (self.banned_until.load(Ordering::Relaxed) - now_ms()) as u64, banned: true });
        }
        if self.used_weight.load(Ordering::Relaxed) > 2000 {
            tokio::time::sleep(Duration::from_millis(1500)).await;
        }
        for attempt in 0..2 {
            let mut p: Vec<(&str, String)> = params.to_vec();
            p.push(("recvWindow", self.recv_window.to_string()));
            p.push(("timestamp", (now_ms() + self.offset_ms()).to_string()));
            let q = encode(&p);
            let sig = self.creds.sign(&q);
            let url = format!("{}{}?{}&signature={}", self.base, path, q, sig);
            let r = self
                .http
                .request(method.clone(), url)
                .header("X-MBX-APIKEY", self.creds.api_key())
                .send()
                .await
                .map_err(|e| if e.is_timeout() { ExError::Unknown(format!("timeout {path}")) } else { ExError::Network(e.without_url().to_string()) })?;
            match self.handle(r).await {
                Err(e) if e.is_timestamp() && attempt == 0 => {
                    self.sync_time().await?;
                }
                other => return other,
            }
        }
        Err(ExError::Unknown("timestamp retry".into()))
    }

    /// listenKey user-data stream (hanya header API key).
    pub async fn listen_key(&self, keepalive: bool) -> Result<String, ExError> {
        let m = if keepalive { reqwest::Method::PUT } else { reqwest::Method::POST };
        let r = self.http.request(m, format!("{}/fapi/v1/listenKey", self.base)).header("X-MBX-APIKEY", self.creds.api_key()).send().await.map_err(|e| ExError::Network(e.without_url().to_string()))?;
        let v = self.handle(r).await?;
        Ok(s(&v["listenKey"]))
    }
}

impl FuturesExchange for BinanceRest {
    fn name(&self) -> &'static str {
        "binance"
    }
    async fn server_time(&self) -> Result<Ms, ExError> {
        Ok(i64v(&self.public("/fapi/v1/time").await?["serverTime"]))
    }
    async fn set_one_way(&self) -> Result<(), ExError> {
        match self.signed(reqwest::Method::POST, "/fapi/v1/positionSide/dual", &[("dualSidePosition", "false".into())]).await {
            Ok(_) => Ok(()),
            Err(e) if e.is_no_change() => Ok(()),
            Err(e) => Err(e),
        }
    }
    async fn set_margin_crossed(&self, symbol: &str) -> Result<(), ExError> {
        match self.signed(reqwest::Method::POST, "/fapi/v1/marginType", &[("symbol", symbol.into()), ("marginType", "CROSSED".into())]).await {
            Ok(_) => Ok(()),
            Err(e) if e.is_no_change() => Ok(()),
            Err(e) => Err(e),
        }
    }
    async fn set_leverage(&self, symbol: &str, leverage: u32) -> Result<u32, ExError> {
        let v = self.signed(reqwest::Method::POST, "/fapi/v1/leverage", &[("symbol", symbol.into()), ("leverage", leverage.to_string())]).await?;
        Ok(v["leverage"].as_u64().unwrap_or(0) as u32)
    }
    async fn place_order(&self, req: &OrderRequest) -> Result<OrderAck, ExError> {
        Ok(parse_order(&self.signed(reqwest::Method::POST, "/fapi/v1/order", &order_params(req)).await?))
    }
    async fn query_order(&self, symbol: &str, client_id: &str) -> Result<Option<OrderAck>, ExError> {
        match self.signed(reqwest::Method::GET, "/fapi/v1/order", &[("symbol", symbol.into()), ("origClientOrderId", client_id.into())]).await {
            Ok(v) => Ok(Some(parse_order(&v))),
            Err(e) if e.code() == Some(-2013) => Ok(None),
            Err(e) => Err(e),
        }
    }
    async fn cancel_order(&self, symbol: &str, client_id: &str) -> Result<(), ExError> {
        self.signed(reqwest::Method::DELETE, "/fapi/v1/order", &[("symbol", symbol.into()), ("origClientOrderId", client_id.into())]).await.map(|_| ())
    }
    async fn open_orders(&self) -> Result<Vec<OrderAck>, ExError> {
        let v = self.signed(reqwest::Method::GET, "/fapi/v1/openOrders", &[]).await?;
        Ok(v.as_array().map(|a| a.iter().map(parse_order).collect()).unwrap_or_default())
    }
    async fn place_algo(&self, req: &AlgoRequest) -> Result<AlgoAck, ExError> {
        Ok(parse_algo(&self.signed(reqwest::Method::POST, "/fapi/v1/algoOrder", &algo_params(req)).await?))
    }
    async fn cancel_algo(&self, _symbol: &str, algo_id: i64) -> Result<(), ExError> {
        self.signed(reqwest::Method::DELETE, "/fapi/v1/algoOrder", &[("algoId", algo_id.to_string())]).await.map(|_| ())
    }
    async fn open_algo_orders(&self) -> Result<Vec<AlgoAck>, ExError> {
        let v = self.signed(reqwest::Method::GET, "/fapi/v1/openAlgoOrders", &[]).await?;
        let arr = v.as_array().cloned().or_else(|| v["orders"].as_array().cloned()).unwrap_or_default();
        Ok(arr.iter().map(parse_algo).collect())
    }
    async fn positions(&self) -> Result<Vec<PositionInfo>, ExError> {
        let v = self.signed(reqwest::Method::GET, "/fapi/v3/positionRisk", &[]).await?;
        Ok(v.as_array()
            .map(|a| {
                a.iter()
                    .map(|p| PositionInfo {
                        symbol: s(&p["symbol"]),
                        amt: dec(&p["positionAmt"]),
                        entry_price: dec(&p["entryPrice"]),
                        mark_price: dec(&p["markPrice"]),
                        margin_type: if p["isolatedMargin"].as_str().map(|x| x != "0" && x != "0.00000000").unwrap_or(false) { "isolated".into() } else { "cross".into() },
                        leverage: p["leverage"].as_str().and_then(|x| x.parse().ok()).unwrap_or(0),
                    })
                    .filter(|p| !p.amt.is_zero())
                    .collect()
            })
            .unwrap_or_default())
    }
    async fn wallet_balance(&self) -> Result<f64, ExError> {
        let v = self.signed(reqwest::Method::GET, "/fapi/v3/balance", &[]).await?;
        let usdt = v.as_array().and_then(|a| a.iter().find(|b| b["asset"] == "USDT").cloned()).unwrap_or(Value::Null);
        Ok(apex_core::num::to_f64(dec(&usdt["balance"])))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rust_decimal_macros::dec;

    #[test]
    fn builds_gtx_entry_and_algo_sl() {
        let o = OrderRequest { symbol: "ETHUSDT".into(), side: "BUY".into(), kind: OrderKind::LimitGtx { price: dec!(2500.10) }, qty: dec!(0.012), reduce_only: false, client_id: "apxS1-ETH-abc-1".into() };
        let q = encode(&order_params(&o));
        assert_eq!(q, "symbol=ETHUSDT&side=BUY&type=LIMIT&timeInForce=GTX&price=2500.1&quantity=0.012&newClientOrderId=apxS1-ETH-abc-1&newOrderRespType=RESULT");
        let a = AlgoRequest { symbol: "ETHUSDT".into(), side: "SELL".into(), kind: AlgoKind::StopMarket, trigger: dec!(2430.00), close_position: true, qty: None, client_algo_id: "apxSL-ETH-abc-1".into() };
        let q = encode(&algo_params(&a));
        assert!(q.starts_with("algoType=CONDITIONAL&symbol=ETHUSDT&side=SELL&type=STOP_MARKET&triggerPrice=2430&workingType=MARK_PRICE"));
        assert!(q.ends_with("closePosition=true"));
        assert!(!q.contains("quantity"));
    }

    #[test]
    fn refuses_mismatched_base() {
        let c = Credentials::new("k".into(), "s".into());
        assert!(BinanceRest::new(ExecMode::Testnet, "https://fapi.binance.com", c.clone(), 5000).is_err());
        assert!(BinanceRest::new(ExecMode::Paper, "https://demo-fapi.binance.com", c, 5000).is_err());
    }
}
