//! Klien REST PUBLIC Binance USDⓈ-M (tanpa API key, tanpa signature).
//! Menghormati X-MBX-USED-WEIGHT-1M, backoff 429 / 418 (IP ban) dengan Retry-After.
use apex_core::time::now_ms;
use apex_core::types::{Bar, Ms, Tf};
use serde_json::Value;
use std::sync::atomic::{AtomicI64, AtomicU64, Ordering};
use std::time::Duration;

pub struct PublicClient {
    pub base: String,
    http: reqwest::Client,
    pub used_weight: AtomicU64,
    pub weight_soft_limit: u64,
    pub ban_until_ms: AtomicI64,
}

fn num(v: &Value) -> f64 {
    match v {
        Value::String(s) => s.parse().unwrap_or(f64::NAN),
        Value::Number(n) => n.as_f64().unwrap_or(f64::NAN),
        _ => f64::NAN,
    }
}

/// Translate an exclusive internal endpoint into Binance's inclusive endpoint.
fn funding_end_time(start: Ms, end: Ms) -> Option<Ms> {
    (start < end).then(|| end - 1)
}

/// Parse satu baris kline REST `[openTime, o, h, l, c, v, closeTime, qv, trades, tbb, tbq, _]`.
pub fn parse_kline_row(r: &Value) -> Option<Bar> {
    let a = r.as_array()?;
    Some(Bar {
        open_time: a.first()?.as_i64()?,
        open: num(a.get(1)?),
        high: num(a.get(2)?),
        low: num(a.get(3)?),
        close: num(a.get(4)?),
        volume: num(a.get(5)?),
        quote_volume: num(a.get(7)?),
        trades: a.get(8)?.as_u64()?,
        taker_buy_base: num(a.get(9)?),
    })
}

impl PublicClient {
    pub fn new(base: &str) -> anyhow::Result<PublicClient> {
        crate::init_tls();
        let http = reqwest::Client::builder().timeout(Duration::from_secs(15)).user_agent("apex-scalper-rs/0.1").build()?;
        Ok(PublicClient {
            base: base.trim_end_matches('/').to_string(),
            http,
            used_weight: AtomicU64::new(0),
            weight_soft_limit: 900,
            ban_until_ms: AtomicI64::new(0),
        })
    }

    pub async fn get(&self, path: &str, query: &[(&str, String)]) -> anyhow::Result<Value> {
        let mut backoff = 1u64;
        for attempt in 0..6 {
            let now = now_ms();
            let ban_until = self.ban_until_ms.load(Ordering::Relaxed);
            if now < ban_until {
                let rem_s = ((ban_until - now) / 1000).max(1);
                anyhow::bail!("cooldown rate-limit aktif ({rem_s}s tersisa) — melewati {path} tanpa request");
            }
            if self.used_weight.load(Ordering::Relaxed) > self.weight_soft_limit {
                tokio::time::sleep(Duration::from_secs(10)).await;
                self.used_weight.store(0, Ordering::Relaxed);
            }
            let resp = self.http.get(format!("{}{}", self.base, path)).query(query).send().await;
            let resp = match resp {
                Ok(r) => r,
                Err(e) if attempt < 5 => {
                    tracing::warn!("GET {path} gagal ({e}); retry {backoff}s");
                    tokio::time::sleep(Duration::from_secs(backoff)).await;
                    backoff = (backoff * 2).min(60);
                    continue;
                }
                Err(e) => return Err(e.into()),
            };
            if let Some(w) = resp.headers().get("x-mbx-used-weight-1m").and_then(|v| v.to_str().ok()).and_then(|s| s.parse().ok()) {
                self.used_weight.store(w, Ordering::Relaxed);
            }
            let st = resp.status().as_u16();
            if st == 429 || st == 418 {
                let ra = resp.headers().get("retry-after").and_then(|v| v.to_str().ok()).and_then(|s| s.parse::<u64>().ok()).unwrap_or(if st == 418 { 120 } else { 30 });
                let until = now_ms() + (ra as i64) * 1000;
                self.ban_until_ms.store(until, Ordering::Relaxed);
                tracing::warn!("HTTP {st} rate limit pada {path}; cooldown {ra}s");
                if st == 418 {
                    anyhow::bail!("HTTP 418: IP cooldown sementara oleh Binance (Retry-After {ra}s)");
                }
                tokio::time::sleep(Duration::from_secs(ra)).await;
                continue;
            }
            if st >= 500 {
                tokio::time::sleep(Duration::from_secs(backoff)).await;
                backoff = (backoff * 2).min(60);
                continue;
            }
            let body = resp.text().await?;
            if st != 200 {
                anyhow::bail!("HTTP {st} {path}: {}", body.chars().take(300).collect::<String>());
            }
            return Ok(serde_json::from_str(&body)?);
        }
        anyhow::bail!("GET {path}: retry habis")
    }

    pub async fn server_time(&self) -> anyhow::Result<Ms> {
        Ok(self.get("/fapi/v1/time", &[]).await?["serverTime"].as_i64().unwrap_or(0))
    }
    pub async fn exchange_info(&self) -> anyhow::Result<Value> {
        self.get("/fapi/v1/exchangeInfo", &[]).await
    }
    pub async fn ticker_24h(&self) -> anyhow::Result<Value> {
        self.get("/fapi/v1/ticker/24hr", &[]).await
    }
    pub async fn book_ticker(&self) -> anyhow::Result<Value> {
        self.get("/fapi/v1/ticker/bookTicker", &[]).await
    }
    pub async fn premium_index(&self) -> anyhow::Result<Value> {
        self.get("/fapi/v1/premiumIndex", &[]).await
    }
    pub async fn premium_index_symbol(&self, sym: &str) -> anyhow::Result<Value> {
        self.get("/fapi/v1/premiumIndex", &[("symbol", sym.into())]).await
    }
    pub async fn depth(&self, sym: &str, limit: u32) -> anyhow::Result<Value> {
        self.get("/fapi/v1/depth", &[("symbol", sym.into()), ("limit", limit.to_string())]).await
    }
    pub async fn global_ls_ratio(&self, sym: &str) -> anyhow::Result<Value> {
        self.get("/futures/data/globalLongShortAccountRatio", &[("symbol", sym.into()), ("period", "5m".into()), ("limit", "1".into())]).await
    }
    pub async fn open_interest_hist(&self, sym: &str) -> anyhow::Result<Value> {
        self.get("/futures/data/openInterestHist", &[("symbol", sym.into()), ("period", "5m".into()), ("limit", "13".into())]).await
    }

    /// Klines [start, end) paging 1500. HANYA mengembalikan bar yang sudah TUTUP pada `now`.
    pub async fn klines(&self, sym: &str, tf: Tf, start: Ms, end: Ms, now: Ms) -> anyhow::Result<Vec<Bar>> {
        let mut out: Vec<Bar> = Vec::new();
        let mut cur = start;
        while cur < end {
            let v = self
                .get("/fapi/v1/klines", &[("symbol", sym.into()), ("interval", tf.binance().into()), ("startTime", cur.to_string()), ("endTime", (end - 1).to_string()), ("limit", "1500".into())])
                .await?;
            let rows: Vec<Bar> = v.as_array().map(|a| a.iter().filter_map(parse_kline_row).collect()).unwrap_or_default();
            if rows.is_empty() {
                break;
            }
            let last = rows.last().map(|b| b.open_time).unwrap_or(end);
            for b in rows {
                let newer = out.last().map(|l| l.open_time < b.open_time).unwrap_or(true);
                if b.open_time >= cur && b.open_time < end && b.available_at(tf) <= now && newer {
                    out.push(b);
                }
            }
            cur = last + tf.ms();
        }
        Ok(out)
    }

    /// Bar tertutup terakhir sebanyak `n`.
    pub async fn recent_closed(&self, sym: &str, tf: Tf, n: usize, now: Ms) -> anyhow::Result<Vec<Bar>> {
        let end = now - now.rem_euclid(tf.ms());
        let start = end - tf.ms() * n as i64;
        self.klines(sym, tf, start, end, now).await
    }

    pub async fn funding_history(&self, sym: &str, start: Ms, end: Ms) -> anyhow::Result<Vec<(Ms, f64)>> {
        let mut out = Vec::new();
        let Some(end_time) = funding_end_time(start, end) else {
            return Ok(out);
        };
        let mut cur = start;
        loop {
            // Binance's endTime is inclusive, while our data window is [start, end).
            let v = self.get("/fapi/v1/fundingRate", &[("symbol", sym.into()), ("startTime", cur.to_string()), ("endTime", end_time.to_string()), ("limit", "1000".into())]).await?;
            let a = v.as_array().cloned().unwrap_or_default();
            if a.is_empty() {
                break;
            }
            let previous_len = out.len();
            for r in &a {
                let time = r["fundingTime"].as_i64().ok_or_else(|| anyhow::anyhow!("fundingTime missing"))?;
                if time >= cur && time < end {
                    out.push((time, num(&r["fundingRate"])));
                }
            }
            if out.len() == previous_len {
                anyhow::bail!("funding response made no progress within requested window");
            }
            let last = out.last().map(|x| x.0).unwrap_or(end);
            if a.len() < 1000 || last >= end - 1 {
                break;
            }
            cur = last + 1;
        }
        Ok(out)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn funding_window_excludes_end_and_empty_windows() {
        let holdout_start = 1_777_593_600_000;
        assert_eq!(funding_end_time(1_735_689_600_000, holdout_start), Some(holdout_start - 1));
        assert_eq!(funding_end_time(holdout_start, holdout_start), None);
    }

    #[test]
    fn kline_row_parse() {
        let v: Value = serde_json::from_str(r#"[1700000000000,"1.0","2.0","0.5","1.5","100",1700000899999,"150",42,"60","90","0"]"#).unwrap();
        let b = parse_kline_row(&v).unwrap();
        assert_eq!(b.trades, 42);
        assert!((b.delta() - 20.0).abs() < 1e-9);
    }
}
