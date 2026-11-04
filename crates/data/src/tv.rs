//! TradingView (endpoint TIDAK resmi, anonim): scanner `CRYPTOCAP:*` + chart WS untuk backfill.
//! Throttle TV-01/02: scanner ≥ 10 s antar request, chart WS ≤ 1 sesi / 3 s. Parser defensif (RK-TV1).
use apex_core::types::Ms;
use futures_util::{SinkExt, StreamExt};
use serde_json::{json, Value};
use std::collections::BTreeMap;
use std::time::Duration;
use tokio_tungstenite::tungstenite::{client::IntoClientRequest, Message};

pub const SCAN_URL: &str = "https://scanner.tradingview.com/cfd/scan";
pub const WS_URL: &str = "wss://data.tradingview.com/socket.io/websocket";
pub const SYMS: &[&str] = &["CRYPTOCAP:BTC.D", "CRYPTOCAP:ETH.D", "CRYPTOCAP:USDT.D", "CRYPTOCAP:OTHERS.D", "CRYPTOCAP:TOTAL", "CRYPTOCAP:TOTAL2", "CRYPTOCAP:TOTAL3"];

/// Hasil scanner: simbol → (close, change% harian).
pub async fn scan(symbols: &[&str]) -> anyhow::Result<BTreeMap<String, (f64, f64)>> {
    crate::init_tls();
    let cols = ["close", "change"];
    let c = reqwest::Client::builder().timeout(Duration::from_secs(10)).build()?;
    let r = c
        .post(SCAN_URL)
        .header("Origin", "https://www.tradingview.com")
        .header("User-Agent", "Mozilla/5.0")
        .json(&json!({"symbols": {"tickers": symbols}, "columns": cols}))
        .send()
        .await?;
    if !r.status().is_success() {
        anyhow::bail!("TV scanner HTTP {}", r.status());
    }
    let v: Value = r.json().await?;
    parse_scan(&v)
}

pub fn parse_scan(v: &Value) -> anyhow::Result<BTreeMap<String, (f64, f64)>> {
    let mut out = BTreeMap::new();
    for d in v.get("data").and_then(|x| x.as_array()).ok_or_else(|| anyhow::anyhow!("format scanner berubah"))? {
        let s = d.get("s").and_then(|x| x.as_str()).unwrap_or_default().to_string();
        let a = d.get("d").and_then(|x| x.as_array()).cloned().unwrap_or_default();
        if let (Some(c), Some(ch)) = (a.first().and_then(|x| x.as_f64()), a.get(1).and_then(|x| x.as_f64())) {
            out.insert(s, (c, ch));
        }
    }
    Ok(out)
}

pub fn frame(f: &str, p: Value) -> String {
    let m = json!({"m": f, "p": p}).to_string();
    format!("~m~{}~m~{}", m.len(), m)
}

/// Pecah payload `~m~<len>~m~<body>...` (len = jumlah byte).
pub fn split_frames(raw: &str) -> Vec<String> {
    let mut out = Vec::new();
    let b = raw.as_bytes();
    let mut i = 0;
    while i < raw.len() && raw[i..].starts_with("~m~") {
        let Some(j) = raw[i + 3..].find("~m~").map(|x| x + i + 3) else { break };
        let Ok(n) = raw[i + 3..j].parse::<usize>() else { break };
        let s = j + 3;
        if s + n > b.len() {
            break;
        }
        out.push(String::from_utf8_lossy(&b[s..s + n]).into_owned());
        i = s + n;
    }
    out
}

/// Backfill bar (time_ms, o, h, l, c) dari chart WS anonim. `interval` "60", "240", "1D".
pub async fn history(symbol: &str, interval: &str, n: usize) -> anyhow::Result<Vec<(Ms, f64, f64, f64, f64)>> {
    crate::init_tls();
    let mut req = WS_URL.into_client_request()?;
    req.headers_mut().insert("Origin", "https://www.tradingview.com".parse()?);
    req.headers_mut().insert("User-Agent", "Mozilla/5.0".parse()?);
    let (mut ws, _) = tokio_tungstenite::connect_async(req).await?;
    let cs = format!("cs_{:012x}", (apex_core::time::now_ms() as u64).wrapping_mul(2654435761) & 0xffff_ffff_ffff);
    for (f, p) in [
        ("set_auth_token", json!(["unauthorized_user_token"])),
        ("chart_create_session", json!([cs, ""])),
        ("resolve_symbol", json!([cs, "sds_sym_1", format!("={{\"symbol\":\"{symbol}\",\"adjustment\":\"splits\"}}")])),
        ("create_series", json!([cs, "sds_1", "s1", "sds_sym_1", interval, n, ""])),
    ] {
        ws.send(Message::Text(frame(f, p))).await?;
    }
    let mut bars: BTreeMap<i64, (f64, f64, f64, f64)> = BTreeMap::new();
    let deadline = tokio::time::Instant::now() + Duration::from_secs(60);
    loop {
        let msg = tokio::time::timeout_at(deadline, ws.next()).await;
        let Ok(Some(Ok(Message::Text(raw)))) = msg else { break };
        let mut done = false;
        for body in split_frames(&raw) {
            if body.starts_with("~h~") {
                ws.send(Message::Text(format!("~m~{}~m~{}", body.len(), body))).await?;
                continue;
            }
            let Ok(d) = serde_json::from_str::<Value>(&body) else { continue };
            match d.get("m").and_then(|x| x.as_str()) {
                Some("timescale_update") => {
                    for b in d["p"][1]["sds_1"]["s"].as_array().into_iter().flatten() {
                        let v = &b["v"];
                        if let (Some(t), Some(o), Some(h), Some(l), Some(c)) = (v[0].as_f64(), v[1].as_f64(), v[2].as_f64(), v[3].as_f64(), v[4].as_f64()) {
                            bars.insert(t as i64 * 1000, (o, h, l, c));
                        }
                    }
                }
                Some("series_completed") => done = true,
                Some("symbol_error") | Some("series_error") | Some("critical_error") => anyhow::bail!("TV error: {body}"),
                _ => {}
            }
        }
        if done {
            break;
        }
    }
    let _ = ws.close(None).await;
    Ok(bars.into_iter().map(|(t, (o, h, l, c))| (t, o, h, l, c)).collect())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn framing_roundtrip() {
        let a = frame("x", json!([1]));
        let b = "~m~4~m~~h~1".to_string();
        let joined = format!("{a}{b}");
        let fr = split_frames(&joined);
        assert_eq!(fr.len(), 2);
        assert_eq!(fr[1], "~h~1");
        assert!(fr[0].contains("\"m\":\"x\""));
    }

    #[test]
    fn scan_parse() {
        let v = json!({"data": [{"s": "CRYPTOCAP:BTC.D", "d": [58.1, -0.3]}, {"s": "CRYPTOCAP:TOTAL", "d": [null, 1.0]}]});
        let m = parse_scan(&v).unwrap();
        assert_eq!(m.len(), 1);
        assert_eq!(m["CRYPTOCAP:BTC.D"], (58.1, -0.3));
    }
}
