//! Parser + runner user-data stream (`ORDER_TRADE_UPDATE`, `ALGO_UPDATE`, `ACCOUNT_UPDATE`).
//!
//! Sejak migrasi rute WebSocket Binance 2026-04-23, stream privat akun berada di rute `/private/ws`
//! dengan parameter `listenKey` dan daftar `events` eksplisit.
use crate::types::*;
use futures_util::{SinkExt, StreamExt};
use rust_decimal::Decimal;
use serde_json::Value;
use std::str::FromStr;
use std::sync::atomic::{AtomicI64, Ordering};
use std::sync::Arc;
use std::time::Duration;
use tokio_tungstenite::tungstenite::Message;

/// Daftar event user-data yang dibutuhkan OMS sesuai dokumentasi resmi Binance USDⓈ-M Futures.
pub const DEFAULT_USER_STREAM_EVENTS: &[&str] =
    &["ORDER_TRADE_UPDATE", "ACCOUNT_UPDATE", "ALGO_UPDATE"];

/// Reconnect proaktif di jam ke-23 sebelum batas paksa 24 jam per koneksi WebSocket.
const PROACTIVE_RECONNECT_SECS: u64 = 23 * 3600;

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

fn normalize_private_root(root: &str) -> String {
    let trimmed = root.trim().trim_end_matches('/');
    for scheme in ["wss://", "ws://"] {
        if let Some(rest) = trimmed.strip_prefix(scheme) {
            let host = rest.split(['/', '?']).next().unwrap_or(rest);
            return format!("{scheme}{host}");
        }
    }
    trimmed
        .split(['/', '?'])
        .next()
        .unwrap_or(trimmed)
        .to_string()
}

/// Bangun URL koneksi user-data stream di rute baru `/private/ws`.
/// Bila `events` diisi, memakai format query `?listenKey=...&events=...`;
/// bila kosong, memakai `/private/ws/<listenKey>`.
pub fn build_private_ws_url(root: &str, listen_key: &str, events: &[&str]) -> String {
    let base = normalize_private_root(root);
    let key = listen_key.trim();
    let evs: Vec<&str> = events
        .iter()
        .copied()
        .filter(|e| !e.trim().is_empty())
        .collect();
    if evs.is_empty() {
        format!("{base}/private/ws/{key}")
    } else {
        format!("{base}/private/ws?listenKey={key}&events={}", evs.join(","))
    }
}

pub fn parse_user_event(text: &str) -> Option<UserEvent> {
    let v: Value = serde_json::from_str(text).ok()?;
    let d_node = v.get("data").unwrap_or(&v);
    let ts = d_node["E"].as_i64().unwrap_or(0);
    match d_node["e"].as_str()? {
        "ORDER_TRADE_UPDATE" => {
            let o = &d_node["o"];
            Some(UserEvent::Order(OrderUpdate {
                symbol: st(&o["s"]),
                client_id: st(&o["c"]),
                order_id: o["i"].as_i64().unwrap_or(0),
                side: st(&o["S"]),
                orig_type: st(if o["ot"].is_string() {
                    &o["ot"]
                } else {
                    &o["o"]
                }),
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
            let o = &d_node["o"];
            Some(UserEvent::Algo {
                symbol: st(&o["s"]),
                algo_id: o["aid"].as_i64().unwrap_or(0),
                client_algo_id: st(&o["caid"]),
                status: st(&o["X"]),
                ts,
            })
        }
        "ACCOUNT_UPDATE" => {
            let ps = d_node["a"]["P"]
                .as_array()
                .map(|a| {
                    a.iter()
                        .map(|p| (st(&p["s"]), d(&p["pa"]), d(&p["ep"])))
                        .collect()
                })
                .unwrap_or_default();
            Some(UserEvent::Account { positions: ps, ts })
        }
        "listenKeyExpired" => Some(UserEvent::ListenKeyExpired),
        _ => None,
    }
}

fn touch_heartbeat(hb: &Option<Arc<AtomicI64>>) {
    if let Some(h) = hb {
        h.store(apex_core::time::now_ms(), Ordering::Relaxed);
    }
}

/// Jalankan stream user-data dengan pencatatan detak jantung opsional untuk supervisi task.
pub async fn run_user_stream_with_heartbeat(
    rest: Arc<crate::binance::BinanceRest>,
    ws_base: String,
    tx: tokio::sync::mpsc::UnboundedSender<UserEvent>,
    heartbeat: Option<Arc<AtomicI64>>,
) {
    loop {
        if tx.is_closed() {
            return;
        }
        let key = match rest.listen_key(false).await {
            Ok(k) => k,
            Err(e) => {
                tracing::warn!("listenKey gagal: {e}");
                tokio::time::sleep(Duration::from_secs(10)).await;
                continue;
            }
        };
        let url = build_private_ws_url(&ws_base, &key, DEFAULT_USER_STREAM_EVENTS);
        match tokio_tungstenite::connect_async(url.as_str()).await {
            Ok((mut ws, _)) => {
                touch_heartbeat(&heartbeat);
                let mut ka = tokio::time::interval(Duration::from_secs(1800));
                ka.tick().await;
                let rotate_at =
                    tokio::time::Instant::now() + Duration::from_secs(PROACTIVE_RECONNECT_SECS);
                loop {
                    tokio::select! {
                        m = ws.next() => match m {
                            Some(Ok(Message::Text(t))) => {
                                touch_heartbeat(&heartbeat);
                                if let Some(ev) = parse_user_event(&t) {
                                    let expired = ev == UserEvent::ListenKeyExpired;
                                    if tx.send(ev).is_err() {
                                        return;
                                    }
                                    if expired {
                                        break;
                                    }
                                }
                            }
                            Some(Ok(Message::Ping(p))) => {
                                touch_heartbeat(&heartbeat);
                                let _ = ws.send(Message::Pong(p)).await;
                            }
                            Some(Ok(Message::Pong(_))) => touch_heartbeat(&heartbeat),
                            Some(Ok(Message::Close(_))) | Some(Err(_)) | None => break,
                            _ => {}
                        },
                        _ = ka.tick() => {
                            if rest.listen_key(true).await.is_ok() {
                                touch_heartbeat(&heartbeat);
                            }
                        }
                        _ = tokio::time::sleep_until(rotate_at) => {
                            let _ = ws.close(None).await;
                            break;
                        }
                    }
                }
            }
            Err(e) => tracing::warn!("user stream connect gagal: {e}"),
        }
        if tx.is_closed() {
            return;
        }
        tokio::time::sleep(Duration::from_secs(2)).await;
    }
}

/// Jalankan stream user-data (reconnect + keepalive 30 menit) dan kirim event ke `tx`.
/// Dipakai hanya di mode testnet/live yang sudah lolos SafetyGuard.
pub async fn run_user_stream(
    rest: Arc<crate::binance::BinanceRest>,
    ws_base: String,
    tx: tokio::sync::mpsc::UnboundedSender<UserEvent>,
) {
    run_user_stream_with_heartbeat(rest, ws_base, tx, None).await;
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
        assert!(matches!(
            parse_user_event(a),
            Some(UserEvent::Algo { algo_id: 77, .. })
        ));
        let acc = r#"{"e":"ACCOUNT_UPDATE","E":3,"a":{"P":[{"s":"ETHUSDT","pa":"0","ep":"0"}]}}"#;
        assert!(matches!(
            parse_user_event(acc),
            Some(UserEvent::Account { .. })
        ));
    }

    #[test]
    fn build_private_ws_url_uses_private_route_and_events() {
        for root in [
            "wss://fstream.binance.com",
            "wss://fstream.binance.com/",
            "wss://fstream.binance.com/ws",
            "wss://fstream.binance.com/private",
        ] {
            let url = build_private_ws_url(root, "lk_123", DEFAULT_USER_STREAM_EVENTS);
            assert_eq!(
                url,
                "wss://fstream.binance.com/private/ws?listenKey=lk_123&events=ORDER_TRADE_UPDATE,ACCOUNT_UPDATE,ALGO_UPDATE"
            );
            assert!(!url.contains("binance.com/ws/"));
        }

        let demo_no_filter = build_private_ws_url("wss://demo-fstream.binance.com", "demo_lk", &[]);
        assert_eq!(
            demo_no_filter,
            "wss://demo-fstream.binance.com/private/ws/demo_lk"
        );
    }
}
