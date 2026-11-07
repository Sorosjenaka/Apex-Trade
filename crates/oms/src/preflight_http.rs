//! Adaptor HTTP/WS/OS read-only di atas [`crate::binance::BinanceRest`] untuk preflight WP-9a.
//!
//! Seluruh pemanggilan HTTP ditempatkan di atas `BinanceRest` yang sudah ada agar:
//! 1. Perilaku tanda tangan HMAC, `recvWindow`, dan pembacaan header rate-limit identik dengan jalur OMS,
//! 2. Semua endpoint dapat diuji dengan server mock lokal (`127.0.0.1`) maupun implementasi trait in-memory
//!    tanpa menyentuh jaringan Binance pada saat test otomatis (H1/H2).

use crate::binance::{encode, BinanceRest};
use crate::notifier::{HttpWebhookNotifier, NotifyPayload};
use crate::types::ExError;
use apex_core::filters::SymbolFilters;
use apex_core::time::now_ms;
use apex_core::types::Ms;
use apex_data::ws::{build_public_ws_url, WsRoute};
use futures_util::StreamExt;
use rust_decimal::Decimal;
use serde_json::Value;
use std::collections::BTreeMap;
use std::io::Write;
use std::path::Path;
use std::str::FromStr;
use std::time::Duration;

/// Endpoint Spot SAPI resmi untuk memeriksa pembatasan API key (`GET /sapi/v1/account/apiRestrictions`).
pub const DEFAULT_SPOT_SAPI_BASE: &str = "https://api.binance.com";

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ApiRestrictionsInfo {
    pub enable_withdrawals: bool,
    pub enable_futures: bool,
    pub ip_restrict: bool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AccountModeInfo {
    pub dual_side_position: bool,
    pub multi_assets_margin: bool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SymbolAccountConfig {
    pub symbol: String,
    pub margin_type: String,
    pub leverage: u32,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CommissionRateInfo {
    pub symbol: String,
    pub maker: Decimal,
    pub taker: Decimal,
}

#[allow(async_fn_in_trait)]
pub trait PreflightBackend {
    async fn clock_offset_ms(&self) -> Result<i64, String>;
    async fn ws_public_handshake(&self, ws_root: &str) -> Result<String, String>;
    async fn api_restrictions(&self) -> Result<ApiRestrictionsInfo, ExError>;
    async fn account_mode(&self) -> Result<AccountModeInfo, String>;
    async fn symbol_configs(&self) -> Result<Vec<SymbolAccountConfig>, String>;
    async fn usdt_available_balance(&self) -> Result<f64, String>;
    async fn exchange_filters_and_marks(
        &self,
    ) -> Result<(BTreeMap<String, SymbolFilters>, BTreeMap<String, Decimal>), String>;
    async fn commission_rate(&self, symbol: &str) -> Result<CommissionRateInfo, String>;
    async fn probe_listen_key(&self) -> Result<(), String>;
    async fn probe_state_fsync_and_disk(&self, state_dir: &Path) -> Result<u64, String>;
    async fn send_notify_test(&self, ts: Ms) -> Result<u16, String>;
}

fn dec_val(v: &Value) -> Decimal {
    match v {
        Value::String(s) => Decimal::from_str(s).unwrap_or_default(),
        Value::Number(n) => Decimal::from_str(&n.to_string()).unwrap_or_default(),
        _ => Decimal::ZERO,
    }
}

impl BinanceRest {
    /// Panggilan signed ke host Spot SAPI (`/sapi/v1/account/apiRestrictions`) memakai kredensial yang sama.
    pub async fn signed_on_base(
        &self,
        base_override: &str,
        method: reqwest::Method,
        path: &str,
        params: &[(&str, String)],
    ) -> Result<Value, ExError> {
        let mut p: Vec<(&str, String)> = params.to_vec();
        p.push(("recvWindow", self.recv_window.to_string()));
        p.push(("timestamp", (now_ms() + self.offset_ms()).to_string()));
        let q = encode(&p);
        let sig = self.creds.sign(&q);
        let root = base_override.trim_end_matches('/');
        let url = format!("{root}{path}?{q}&signature={sig}");
        let resp = self
            .http
            .request(method, url)
            .header("X-MBX-APIKEY", self.creds.api_key())
            .send()
            .await
            .map_err(|e| ExError::Network(e.without_url().to_string()))?;
        self.handle(resp).await
    }

    pub async fn fetch_api_restrictions(
        &self,
        sapi_base: &str,
    ) -> Result<ApiRestrictionsInfo, ExError> {
        let v = self
            .signed_on_base(
                sapi_base,
                reqwest::Method::GET,
                "/sapi/v1/account/apiRestrictions",
                &[],
            )
            .await?;
        Ok(ApiRestrictionsInfo {
            enable_withdrawals: v["enableWithdrawals"].as_bool().unwrap_or(false),
            enable_futures: v["enableFutures"].as_bool().unwrap_or(false),
            ip_restrict: v["ipRestrict"].as_bool().unwrap_or(false),
        })
    }

    pub async fn fetch_account_mode(&self) -> Result<AccountModeInfo, ExError> {
        let dual = self
            .signed(reqwest::Method::GET, "/fapi/v1/positionSide/dual", &[])
            .await?;
        let multi = self
            .signed(reqwest::Method::GET, "/fapi/v1/multiAssetsMargin", &[])
            .await?;
        Ok(AccountModeInfo {
            dual_side_position: dual["dualSidePosition"].as_bool().unwrap_or(true),
            multi_assets_margin: multi["multiAssetsMargin"].as_bool().unwrap_or(true),
        })
    }

    pub async fn fetch_symbol_configs(&self) -> Result<Vec<SymbolAccountConfig>, ExError> {
        let v = self
            .signed(reqwest::Method::GET, "/fapi/v1/symbolConfig", &[])
            .await?;
        let rows = v.as_array().cloned().unwrap_or_default();
        let mut out = Vec::with_capacity(rows.len());
        for r in rows {
            let symbol = r["symbol"].as_str().unwrap_or_default().to_string();
            let margin_type = r["marginType"].as_str().unwrap_or_default().to_string();
            let leverage = r["leverage"]
                .as_u64()
                .or_else(|| r["leverage"].as_str().and_then(|s| s.parse().ok()))
                .unwrap_or(0) as u32;
            if !symbol.is_empty() {
                out.push(SymbolAccountConfig {
                    symbol,
                    margin_type,
                    leverage,
                });
            }
        }
        Ok(out)
    }

    pub async fn fetch_usdt_available_balance(&self) -> Result<f64, ExError> {
        let v = self
            .signed(reqwest::Method::GET, "/fapi/v3/balance", &[])
            .await?;
        let usdt = v
            .as_array()
            .and_then(|a| a.iter().find(|b| b["asset"] == "USDT").cloned())
            .unwrap_or(Value::Null);
        let avail = dec_val(&usdt["availableBalance"]);
        Ok(apex_core::num::to_f64(avail))
    }

    pub async fn fetch_commission_rate(&self, symbol: &str) -> Result<CommissionRateInfo, ExError> {
        let v = self
            .signed(
                reqwest::Method::GET,
                "/fapi/v1/commissionRate",
                &[("symbol", symbol.to_string())],
            )
            .await?;
        Ok(CommissionRateInfo {
            symbol: v["symbol"].as_str().unwrap_or(symbol).to_string(),
            maker: dec_val(&v["makerCommissionRate"]),
            taker: dec_val(&v["takerCommissionRate"]),
        })
    }

    pub async fn fetch_filters_and_marks(
        &self,
    ) -> Result<(BTreeMap<String, SymbolFilters>, BTreeMap<String, Decimal>), ExError> {
        let ei = self.public("/fapi/v1/exchangeInfo").await?;
        let pi = self.public("/fapi/v1/premiumIndex").await?;
        let filters: BTreeMap<String, SymbolFilters> = ei["symbols"]
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(SymbolFilters::from_exchange_info_symbol)
            .map(|f| (f.symbol.clone(), f))
            .collect();
        let mut marks = BTreeMap::new();
        if let Some(arr) = pi.as_array() {
            for row in arr {
                let sym = row["symbol"].as_str().unwrap_or_default();
                let mk = dec_val(&row["markPrice"]);
                if !sym.is_empty() && mk > Decimal::ZERO {
                    marks.insert(sym.to_string(), mk);
                }
            }
        }
        Ok((filters, marks))
    }
}

pub struct BinancePreflightBackend<'a> {
    pub rest: Option<&'a BinanceRest>,
    pub sapi_base: String,
    pub notifier: Option<HttpWebhookNotifier>,
}

impl<'a> PreflightBackend for BinancePreflightBackend<'a> {
    async fn clock_offset_ms(&self) -> Result<i64, String> {
        let rest = self
            .rest
            .ok_or_else(|| "klien REST belum siap (kredensial tidak tersedia)".to_string())?;
        rest.sync_time().await.map_err(|e| e.to_string())
    }

    async fn ws_public_handshake(&self, ws_root: &str) -> Result<String, String> {
        let url = build_public_ws_url(ws_root, WsRoute::Public, &["btcusdt@bookTicker".into()]);
        let fut = tokio_tungstenite::connect_async(&url);
        let (mut ws, _) = tokio::time::timeout(Duration::from_secs(6), fut)
            .await
            .map_err(|_| "timeout handshake WebSocket publik (6 dtk)".to_string())?
            .map_err(|e| format!("gagal handshake WebSocket publik: {e}"))?;
        let _ = ws.close(None).await;
        let _ = tokio::time::timeout(Duration::from_millis(250), ws.next()).await;
        Ok(url)
    }

    async fn api_restrictions(&self) -> Result<ApiRestrictionsInfo, ExError> {
        let rest = self
            .rest
            .ok_or_else(|| ExError::Network("klien REST tidak tersedia".into()))?;
        rest.fetch_api_restrictions(&self.sapi_base).await
    }

    async fn account_mode(&self) -> Result<AccountModeInfo, String> {
        let rest = self
            .rest
            .ok_or_else(|| "klien REST tidak tersedia".to_string())?;
        rest.fetch_account_mode().await.map_err(|e| e.to_string())
    }

    async fn symbol_configs(&self) -> Result<Vec<SymbolAccountConfig>, String> {
        let rest = self
            .rest
            .ok_or_else(|| "klien REST tidak tersedia".to_string())?;
        rest.fetch_symbol_configs().await.map_err(|e| e.to_string())
    }

    async fn usdt_available_balance(&self) -> Result<f64, String> {
        let rest = self
            .rest
            .ok_or_else(|| "klien REST tidak tersedia".to_string())?;
        rest.fetch_usdt_available_balance()
            .await
            .map_err(|e| e.to_string())
    }

    async fn exchange_filters_and_marks(
        &self,
    ) -> Result<(BTreeMap<String, SymbolFilters>, BTreeMap<String, Decimal>), String> {
        let rest = self
            .rest
            .ok_or_else(|| "klien REST tidak tersedia".to_string())?;
        rest.fetch_filters_and_marks()
            .await
            .map_err(|e| e.to_string())
    }

    async fn commission_rate(&self, symbol: &str) -> Result<CommissionRateInfo, String> {
        let rest = self
            .rest
            .ok_or_else(|| "klien REST tidak tersedia".to_string())?;
        rest.fetch_commission_rate(symbol)
            .await
            .map_err(|e| e.to_string())
    }

    async fn probe_listen_key(&self) -> Result<(), String> {
        let rest = self
            .rest
            .ok_or_else(|| "klien REST tidak tersedia".to_string())?;
        let key = rest.listen_key(false).await.map_err(|e| e.to_string())?;
        if key.trim().is_empty() {
            return Err("respons listenKey kosong".into());
        }
        rest.delete_listen_key().await.map_err(|e| e.to_string())
    }

    async fn probe_state_fsync_and_disk(&self, state_dir: &Path) -> Result<u64, String> {
        probe_dir_fsync_and_free_bytes(state_dir)
    }

    async fn send_notify_test(&self, ts: Ms) -> Result<u16, String> {
        let n = self
            .notifier
            .as_ref()
            .ok_or_else(|| "APEX_NOTIFY_URL kosong atau tidak valid".to_string())?;
        let payload = NotifyPayload {
            level: "INFO".into(),
            event: "PREFLIGHT_NOTIFY_TEST".into(),
            text: "Uji konektivitas notifier dari apex preflight --notify-test".into(),
            ts,
        };
        n.send_sync(&payload).await
    }
}

/// Uji tulis + `File::sync_all()` pada direktori state serta periksa kapasitas kosong disk OS.
pub fn probe_dir_fsync_and_free_bytes(state_dir: &Path) -> Result<u64, String> {
    std::fs::create_dir_all(state_dir)
        .map_err(|e| format!("gagal membuat direktori state {}: {e}", state_dir.display()))?;
    let probe = state_dir.join(".preflight_fsync_probe.tmp");
    {
        let mut f = std::fs::OpenOptions::new()
            .create(true)
            .write(true)
            .truncate(true)
            .open(&probe)
            .map_err(|e| format!("gagal membuka file uji fsync: {e}"))?;
        f.write_all(b"apex-preflight-fsync-ok\n")
            .map_err(|e| format!("gagal menulis file uji fsync: {e}"))?;
        f.sync_all()
            .map_err(|e| format!("File::sync_all gagal: {e}"))?;
    }
    let _ = std::fs::remove_file(&probe);
    query_free_disk_bytes(state_dir)
}

#[cfg(unix)]
fn query_free_disk_bytes(path: &Path) -> Result<u64, String> {
    let out = std::process::Command::new("df")
        .arg("-Pk")
        .arg(path)
        .output()
        .map_err(|e| format!("gagal menjalankan df -Pk: {e}"))?;
    if !out.status.success() {
        return Err("perintah df -Pk gagal".into());
    }
    let text = String::from_utf8_lossy(&out.stdout);
    let line = text
        .lines()
        .nth(1)
        .ok_or_else(|| "keluaran df kosong".to_string())?;
    let avail_kib: u64 = line
        .split_whitespace()
        .nth(3)
        .and_then(|s| s.parse().ok())
        .ok_or_else(|| "gagal membaca kolom available df".to_string())?;
    Ok(avail_kib.saturating_mul(1024))
}

#[cfg(windows)]
fn query_free_disk_bytes(path: &Path) -> Result<u64, String> {
    let canon = std::fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf());
    let out = std::process::Command::new("powershell")
        .args([
            "-NoProfile",
            "-NonInteractive",
            "-Command",
            "(Get-Item -LiteralPath $env:APEX_PROBE_PATH).PSDrive.Free",
        ])
        .env("APEX_PROBE_PATH", canon.as_os_str())
        .output()
        .map_err(|e| format!("gagal memeriksa ruang disk Windows: {e}"))?;
    if !out.status.success() {
        return Err("pemeriksaan PSDrive.Free gagal".into());
    }
    String::from_utf8_lossy(&out.stdout)
        .trim()
        .parse::<u64>()
        .map_err(|e| format!("gagal mem-parse byte ruang disk kosong: {e}"))
}
