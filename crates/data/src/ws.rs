//! WebSocket PUBLIC Binance USDⓈ-M Futures (`/public` dan `/market`):
//! kline (hanya `x=true` / bar tertutup dipakai) + forceOrder (MR-06).
//!
//! Sejak 2026-04-23 Binance memensiunkan path lama `/ws` dan `/stream` tanpa kategori.
//! Setiap koneksi kini wajib diarahkan ke `/public` atau `/market`, dan satu koneksi
//! hanya boleh menggabungkan stream dari rute yang sama (maks. 200 stream, umur <24 jam).
use apex_core::types::{Bar, Ms};
use futures_util::{SinkExt, StreamExt};
use serde_json::Value;
use std::collections::BTreeMap;
use std::sync::atomic::{AtomicI64, Ordering};
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::mpsc;
use tokio_tungstenite::tungstenite::Message;

/// Batas resmi Binance untuk jumlah stream pada satu koneksi combined-stream.
pub const MAX_STREAMS_PER_CONN: usize = 200;

/// Reconnect proaktif di jam ke-23 supaya pergantian koneksi terkendali sebelum
/// pemutusan paksa 24 jam dari sisi server Binance.
pub const PROACTIVE_RECONNECT_SECS: u64 = 23 * 3600;

/// Kategori rute endpoint WebSocket publik Binance sejak migrasi 2026-04-23.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum WsRoute {
    Public,
    Market,
}

impl WsRoute {
    pub fn as_str(self) -> &'static str {
        match self {
            WsRoute::Public => "public",
            WsRoute::Market => "market",
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub enum WsEvent {
    /// Bar TERTUTUP (`k.x == true`).
    ClosedKline {
        symbol: String,
        interval: String,
        bar: Bar,
    },
    Liquidation {
        symbol: String,
        ts: Ms,
        notional: f64,
    },
    Reconnected,
}

fn f(v: &Value) -> f64 {
    v.as_str()
        .and_then(|s| s.parse().ok())
        .or_else(|| v.as_f64())
        .unwrap_or(f64::NAN)
}

/// Klasifikasikan nama stream ke rute `/public` atau `/market` sesuai spesifikasi Binance.
/// Pencocokan `@trade` harus eksak pada segmen sesudah `@` agar `@aggTrade` tetap masuk `/market`.
pub fn route_for_stream(stream: &str) -> WsRoute {
    let s = stream.trim().to_ascii_lowercase();
    if s == "!bookticker" {
        return WsRoute::Public;
    }
    if let Some((_, suffix)) = s.split_once('@') {
        let head = suffix.split('@').next().unwrap_or(suffix);
        if head == "bookticker"
            || head == "rpidepth"
            || head == "trade"
            || head.starts_with("depth")
        {
            return WsRoute::Public;
        }
    }
    WsRoute::Market
}

/// Bersihkan host akar dari garis miring atau path warisan agar pembangun URL
/// tidak pernah menghasilkan endpoint lama yang sudah dimatikan.
pub fn normalize_ws_root(root: &str) -> String {
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

/// Bangun URL combined-stream rute baru `wss://<root>/<route>/stream?streams=...`.
pub fn build_public_ws_url(root: &str, route: WsRoute, streams: &[String]) -> String {
    let base = normalize_ws_root(root);
    format!(
        "{}/{}/stream?streams={}",
        base,
        route.as_str(),
        streams.join("/")
    )
}

/// Parse pesan combined-stream (`{"stream":..,"data":{..}}`) atau raw JSON.
pub fn parse_ws(text: &str) -> Option<WsEvent> {
    let v: Value = serde_json::from_str(text).ok()?;
    let d = v.get("data").unwrap_or(&v);
    match d.get("e")?.as_str()? {
        "kline" => {
            let k = d.get("k")?;
            if !k.get("x")?.as_bool()? {
                return None; // bar belum tutup — diabaikan demi kausalitas closed-bar
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
            Some(WsEvent::Liquidation {
                symbol: o.get("s")?.as_str()?.to_string(),
                ts: o.get("T")?.as_i64()?,
                notional: f(o.get("ap")?) * f(o.get("z")?),
            })
        }
        _ => None,
    }
}

fn touch_heartbeat(hb: &Option<Arc<AtomicI64>>) {
    if let Some(h) = hb {
        h.store(apex_core::time::now_ms(), Ordering::Relaxed);
    }
}

async fn run_route_chunk(
    url: String,
    tx: mpsc::Sender<WsEvent>,
    heartbeat: Option<Arc<AtomicI64>>,
) {
    let mut backoff = 1u64;
    loop {
        match tokio_tungstenite::connect_async(url.as_str()).await {
            Ok((mut ws, _)) => {
                backoff = 1;
                touch_heartbeat(&heartbeat);
                if tx.send(WsEvent::Reconnected).await.is_err() {
                    return;
                }
                let rotate_at =
                    tokio::time::Instant::now() + Duration::from_secs(PROACTIVE_RECONNECT_SECS);
                loop {
                    tokio::select! {
                        msg = ws.next() => match msg {
                            Some(Ok(Message::Text(t))) => {
                                touch_heartbeat(&heartbeat);
                                if let Some(ev) = parse_ws(&t) {
                                    if tx.send(ev).await.is_err() {
                                        return;
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
                        _ = tokio::time::sleep_until(rotate_at) => {
                            // Putus sopan sebelum batas 24 jam agar reconnect terjadi terjadwal.
                            let _ = ws.close(None).await;
                            break;
                        }
                    }
                }
            }
            Err(e) => tracing::warn!("WS connect gagal ({url}): {e}"),
        }
        if tx.is_closed() {
            return;
        }
        tokio::time::sleep(Duration::from_secs(backoff)).await;
        backoff = (backoff * 2).min(60);
    }
}

/// Kelompokkan daftar stream per rute (`/public` vs `/market`) dan pecah per ≤ 200 stream.
pub fn group_streams_by_route(streams: &[String]) -> Vec<(WsRoute, Vec<String>)> {
    let mut grouped: BTreeMap<WsRoute, Vec<String>> = BTreeMap::new();
    for s in streams {
        let trimmed = s.trim();
        if !trimmed.is_empty() {
            grouped
                .entry(route_for_stream(trimmed))
                .or_default()
                .push(trimmed.to_string());
        }
    }
    let mut out = Vec::new();
    for (route, list) in grouped {
        for chunk in list.chunks(MAX_STREAMS_PER_CONN) {
            out.push((route, chunk.to_vec()));
        }
    }
    out
}

/// Loop stream publik dengan pengelompokan rute otomatis dan pelaporan detak jantung opsional.
pub async fn run_public_stream_with_heartbeat(
    ws_base: &str,
    streams: Vec<String>,
    tx: mpsc::Sender<WsEvent>,
    heartbeat: Option<Arc<AtomicI64>>,
) -> anyhow::Result<()> {
    crate::init_tls();
    let groups = group_streams_by_route(&streams);
    if groups.is_empty() {
        return Ok(());
    }
    let mut handles = Vec::with_capacity(groups.len());
    for (route, chunk) in groups {
        let url = build_public_ws_url(ws_base, route, &chunk);
        let tx_clone = tx.clone();
        let hb_clone = heartbeat.clone();
        handles.push(tokio::spawn(async move {
            run_route_chunk(url, tx_clone, hb_clone).await;
        }));
    }
    drop(tx);
    for h in handles {
        let _ = h.await;
    }
    Ok(())
}

/// Loop stream publik dengan reconnect (backoff 1→60 dtk) dan rotasi proaktif <24 jam.
pub async fn run_public_stream(
    ws_base: &str,
    streams: Vec<String>,
    tx: mpsc::Sender<WsEvent>,
) -> anyhow::Result<()> {
    run_public_stream_with_heartbeat(ws_base, streams, tx, None).await
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
        assert_eq!(
            parse_ws(liq),
            Some(WsEvent::Liquidation {
                symbol: "ETHUSDT".into(),
                ts: 123,
                notional: 3000.0
            })
        );
    }

    #[test]
    fn route_classification_covers_all_repo_streams() {
        for market_s in [
            "btcusdt@kline_5m",
            "btcusdt@kline_15m",
            "ethusdt@kline_1h",
            "ethusdt@forceOrder",
            "!forceOrder@arr",
            "ethusdt@aggTrade",
            "ethusdt@markPrice@1s",
            "!markPrice@arr@1s",
            "solusdt@miniTicker",
            "solusdt@ticker",
        ] {
            assert_eq!(
                route_for_stream(market_s),
                WsRoute::Market,
                "stream={market_s}"
            );
        }
        for public_s in [
            "ethusdt@depth20@100ms",
            "ethusdt@depth@100ms",
            "ethusdt@depth5",
            "btcusdt@bookTicker",
            "!bookTicker",
            "btcusdt@rpiDepth",
            "btcusdt@trade",
        ] {
            assert_eq!(
                route_for_stream(public_s),
                WsRoute::Public,
                "stream={public_s}"
            );
        }
    }

    #[test]
    fn build_public_ws_url_never_emits_legacy_path() {
        let streams = vec![
            "btcusdt@kline_15m".to_string(),
            "ethusdt@kline_1h".to_string(),
        ];
        for root in [
            "wss://fstream.binance.com",
            "wss://fstream.binance.com/",
            "wss://fstream.binance.com/ws",
            "wss://fstream.binance.com/stream?streams=old",
            "wss://fstream.binance.com/market",
        ] {
            let url = build_public_ws_url(root, WsRoute::Market, &streams);
            assert_eq!(
                url,
                "wss://fstream.binance.com/market/stream?streams=btcusdt@kline_15m/ethusdt@kline_1h"
            );
            assert!(!url.contains("binance.com/stream?"));
            assert!(!url.contains("binance.com/ws"));
        }

        let demo_url = build_public_ws_url(
            "wss://demo-fstream.binance.com",
            WsRoute::Public,
            &["ethusdt@depth20@100ms".to_string()],
        );
        assert_eq!(
            demo_url,
            "wss://demo-fstream.binance.com/public/stream?streams=ethusdt@depth20@100ms"
        );
    }

    #[test]
    fn group_streams_splits_routes_and_respects_200_limit() {
        let mut many: Vec<String> = (0..205).map(|i| format!("sym{i}usdt@kline_15m")).collect();
        many.push("ethusdt@depth20@100ms".into());
        let groups = group_streams_by_route(&many);
        assert_eq!(groups.len(), 3);
        assert_eq!(groups[0].0, WsRoute::Public);
        assert_eq!(groups[0].1.len(), 1);
        assert_eq!(groups[1].0, WsRoute::Market);
        assert_eq!(groups[1].1.len(), 200);
        assert_eq!(groups[2].0, WsRoute::Market);
        assert_eq!(groups[2].1.len(), 5);
    }

    #[tokio::test]
    #[ignore]
    async fn live_readonly_market_kline_stream() {
        if std::env::var("APEX_NET_TESTS").as_deref() != Ok("1") {
            return;
        }
        let (tx, mut rx) = mpsc::channel(16);
        let handle = tokio::spawn(async move {
            let _ = run_public_stream(
                "wss://fstream.binance.com",
                vec!["btcusdt@kline_1m".to_string()],
                tx,
            )
            .await;
        });
        let deadline = tokio::time::Instant::now() + Duration::from_secs(90);
        let mut saw_reconnect = false;
        let mut saw_closed_bar = false;
        while tokio::time::Instant::now() < deadline {
            match tokio::time::timeout_at(deadline, rx.recv()).await {
                Ok(Some(WsEvent::Reconnected)) => saw_reconnect = true,
                Ok(Some(WsEvent::ClosedKline {
                    symbol, interval, ..
                })) => {
                    assert_eq!(symbol, "BTCUSDT");
                    assert_eq!(interval, "1m");
                    saw_closed_bar = true;
                    break;
                }
                Ok(Some(_)) => {}
                _ => break,
            }
        }
        handle.abort();
        assert!(
            saw_reconnect,
            "tidak menerima sinyal Reconnected dari /market"
        );
        assert!(
            saw_closed_bar,
            "tidak menerima ClosedKline 1m dalam 90 detik"
        );
    }
}
