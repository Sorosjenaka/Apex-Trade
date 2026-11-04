//! Parser + runner user-data stream (ORDER_TRADE_UPDATE, ALGO_UPDATE, ACCOUNT_UPDATE).
use crate::types::*;
use rust_decimal::Decimal;
use serde_json::Value;
use std::str::FromStr;

fn d(v: &Value) -> Decimal {
    match v {
        Value::String(s) => Decimal::from_str(s).unwrap_or_default(),
        Value::Number(n) => Decimal::from_str(&n.to_string()).unwrap_or_default(),
        _ => Decimal::ZERO,
    }
}
fn st(v: &Value) -> String {
    v.as_str().unwrap_or_default().to_string()
}

pub fn parse_user_event(text: &str) -> Option<UserEvent> {
    let v: Value = serde_json::from_str(text).ok()?;
    let ts = v["E"].as_i64().unwrap_or(0);
    match v["e"].as_str()? {
        "ORDER_TRADE_UPDATE" => {
            let o = &v["o"];
            Some(UserEvent::Order(OrderUpdate {
                symbol: st(&o["s"]),
                client_id: st(&o["c"]),
                order_id: o["i"].as_i64().unwrap_or(0),
                side: st(&o["S"]),
                orig_type: st(if o["ot"].is_string() { &o["ot"] } else { &o["o"] }),
                status: OrderStatus::parse(o["X"].as_str().unwrap_or("NEW")),
                exec_type: st(&o["x"]),
                last_qty: d(&o["l"]),
                cum_qty: d(&o["z"]),
                last_price: d(&o["L"]),
                avg_price: d(&o["ap"]),
                fee: d(&o["n"]),
                realized_pnl: d(&o["rp"]),
                reduce_only: o["R"].as_bool().unwrap_or(false),
                close_position: o["cp"].as_bool().unwrap_or(false),
                ts: o["T"].as_i64().unwrap_or(ts),
            }))
        }
        "ALGO_UPDATE" => {
            let o = &v["o"];
            Some(UserEvent::Algo { symbol: st(&o["s"]), algo_id: o["aid"].as_i64().unwrap_or(0), client_algo_id: st(&o["caid"]), status: st(&o["X"]), ts })
        }
        "ACCOUNT_UPDATE" => {
            let ps = v["a"]["P"].as_array().map(|a| a.iter().map(|p| (st(&p["s"]), d(&p["pa"]), d(&p["ep"]))).collect()).unwrap_or_default();
            Some(UserEvent::Account { positions: ps, ts })
        }
        "listenKeyExpired" => Some(UserEvent::ListenKeyExpired),
        _ => None,
    }
}

/// Jalankan stream user-data (reconnect + keepalive 30 menit) dan kirim event ke `tx`.
/// Dipakai hanya di mode testnet/live yang sudah lolos SafetyGuard.
pub async fn run_user_stream(rest: std::sync::Arc<crate::binance::BinanceRest>, ws_base: String, tx: tokio::sync::mpsc::UnboundedSender<UserEvent>) {
    use futures_util::StreamExt;
    loop {
        let key = match rest.listen_key(false).await {
            Ok(k) => k,
            Err(e) => {
                tracing::warn!("listenKey gagal: {e}");
                tokio::time::sleep(std::time::Duration::from_secs(10)).await;
                continue;
            }
        };
        let url = format!("{}/ws/{}", ws_base.trim_end_matches('/'), key);
        match tokio_tungstenite::connect_async(url.as_str()).await {
            Ok((mut ws, _)) => {
                let mut ka = tokio::time::interval(std::time::Duration::from_secs(1800));
                ka.tick().await;
                loop {
                    tokio::select! {
                        m = ws.next() => match m {
                            Some(Ok(tokio_tungstenite::tungstenite::Message::Text(t))) => {
                                if let Some(ev) = parse_user_event(&t) {
                                    let expired = ev == UserEvent::ListenKeyExpired;
                                    let _ = tx.send(ev);
                                    if expired { break; }
                                }
                            }
                            Some(Ok(_)) => {}
                            _ => break,
                        },
                        _ = ka.tick() => { let _ = rest.listen_key(true).await; }
                    }
                }
            }
            Err(e) => tracing::warn!("user stream connect gagal: {e}"),
        }
        tokio::time::sleep(std::time::Duration::from_secs(2)).await;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn parses_order_and_algo() {
        let t = r#"{"e":"ORDER_TRADE_UPDATE","E":1,"o":{"s":"ETHUSDT","c":"apxS1-ETH-x-1","S":"BUY","o":"LIMIT","ot":"LIMIT","X":"FILLED","x":"TRADE","i":9,"l":"0.01","z":"0.01","L":"2500","ap":"2500","n":"0.005","rp":"0","R":false,"cp":false,"T":5}}"#;
        match parse_user_event(t).unwrap() {
            UserEvent::Order(o) => {
                assert_eq!(o.status, OrderStatus::Filled);
                assert_eq!(o.client_id, "apxS1-ETH-x-1");
                assert_eq!(o.ts, 5);
            }
            _ => panic!(),
        }
        let a = r#"{"e":"ALGO_UPDATE","E":2,"o":{"caid":"apxSL-1","aid":77,"s":"ETHUSDT","X":"TRIGGERED"}}"#;
        assert!(matches!(parse_user_event(a), Some(UserEvent::Algo { algo_id: 77, .. })));
        let acc = r#"{"e":"ACCOUNT_UPDATE","E":3,"a":{"P":[{"s":"ETHUSDT","pa":"0","ep":"0"}]}}"#;
        assert!(matches!(parse_user_event(acc), Some(UserEvent::Account { .. })));
    }
}
