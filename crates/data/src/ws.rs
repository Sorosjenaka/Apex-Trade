//! WebSocket PUBLIC Binance: kline (hanya `x=true` / bar tertutup dipakai) + forceOrder (MR-06).
use apex_core::types::{Bar, Ms};
use futures_util::{SinkExt, StreamExt};
use serde_json::Value;
use std::time::Duration;
use tokio::sync::mpsc;
use tokio_tungstenite::tungstenite::Message;

#[derive(Clone, Debug, PartialEq)]
pub enum WsEvent {
    /// Bar TERTUTUP (k.x == true).
    ClosedKline { symbol: String, interval: String, bar: Bar },
    Liquidation { symbol: String, ts: Ms, notional: f64 },
    Reconnected,
}

fn f(v: &Value) -> f64 {
    v.as_str().and_then(|s| s.parse().ok()).or_else(|| v.as_f64()).unwrap_or(f64::NAN)
}

/// Parse pesan combined-stream (`{"stream":..,"data":{..}}`) atau raw.
pub fn parse_ws(text: &str) -> Option<WsEvent> {
    let v: Value = serde_json::from_str(text).ok()?;
    let d = v.get("data").unwrap_or(&v);
    match d.get("e")?.as_str()? {
        "kline" => {
            let k = d.get("k")?;
            if !k.get("x")?.as_bool()? {
                return None; // bar belum tutup — diabaikan (closed-bar only)
            }
            Some(WsEvent::ClosedKline {
                symbol: d.get("s")?.as_str()?.to_string(),
                interval: k.get("i")?.as_str()?.to_string(),
                bar: Bar {
                    open_time: k.get("t")?.as_i64()?,
                    open: f(k.get("o")?),
                    high: f(k.get("h")?),
                    low: f(k.get("l")?),
                    close: f(k.get("c")?),
                    volume: f(k.get("v")?),
                    quote_volume: f(k.get("q")?),
                    trades: k.get("n")?.as_u64()?,
                    taker_buy_base: f(k.get("V")?),
                },
            })
        }
        "forceOrder" => {
            let o = d.get("o")?;
            Some(WsEvent::Liquidation { symbol: o.get("s")?.as_str()?.to_string(), ts: o.get("T")?.as_i64()?, notional: f(o.get("ap")?) * f(o.get("z")?) })
        }
        _ => None,
    }
}

/// Loop stream publik dengan reconnect (backoff ≤ 60 s). Binance memutus koneksi tiap 24 jam → reconnect normal.
pub async fn run_public_stream(ws_base: &str, streams: Vec<String>, tx: mpsc::Sender<WsEvent>) -> anyhow::Result<()> {
    crate::init_tls();
    let url = format!("{}/stream?streams={}", ws_base.trim_end_matches('/'), streams.join("/"));
    let mut backoff = 1u64;
    loop {
        match tokio_tungstenite::connect_async(url.as_str()).await {
            Ok((mut ws, _)) => {
                backoff = 1;
                let _ = tx.send(WsEvent::Reconnected).await;
                while let Some(msg) = ws.next().await {
                    match msg {
                        Ok(Message::Text(t)) => {
                            if let Some(ev) = parse_ws(&t) {
                                if tx.send(ev).await.is_err() {
                                    return Ok(());
                                }
                            }
                        }
                        Ok(Message::Ping(p)) => {
                            let _ = ws.send(Message::Pong(p)).await;
                        }
                        Ok(Message::Close(_)) | Err(_) => break,
                        _ => {}
                    }
                }
            }
            Err(e) => tracing::warn!("WS connect gagal: {e}"),
        }
        if tx.is_closed() {
            return Ok(());
        }
        tokio::time::sleep(Duration::from_secs(backoff)).await;
        backoff = (backoff * 2).min(60);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_closed_klines() {
        let open = r#"{"stream":"btcusdt@kline_15m","data":{"e":"kline","E":1,"s":"BTCUSDT","k":{"t":0,"T":899999,"s":"BTCUSDT","i":"15m","o":"1","c":"2","h":"3","l":"0.5","v":"10","n":5,"x":false,"q":"20","V":"6","Q":"12"}}}"#;
        assert_eq!(parse_ws(open), None);
        let closed = open.replace("\"x\":false", "\"x\":true");
        match parse_ws(&closed) {
            Some(WsEvent::ClosedKline { symbol, bar, .. }) => {
                assert_eq!(symbol, "BTCUSDT");
                assert_eq!(bar.trades, 5);
                assert!((bar.taker_buy_base - 6.0).abs() < 1e-12);
            }
            other => panic!("{other:?}"),
        }
        let liq = r#"{"e":"forceOrder","E":1,"o":{"s":"ETHUSDT","S":"SELL","ap":"2000","z":"1.5","T":123}}"#;
        assert_eq!(parse_ws(liq), Some(WsEvent::Liquidation { symbol: "ETHUSDT".into(), ts: 123, notional: 3000.0 }));
    }
}
