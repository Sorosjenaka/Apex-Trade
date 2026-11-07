//! Config. `RiskConfig` = angka Golden Rules (Rules §1). Engine menolak start bila berbeda (T-0.6).
use rust_decimal::prelude::*;
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

#[derive(Debug, thiserror::Error)]
pub enum ConfigError {
    #[error("gagal baca config {0}: {1}")]
    Io(PathBuf, std::io::Error),
    #[error("config TOML tidak valid {0}: {1}")]
    Parse(PathBuf, toml::de::Error),
    #[error("risk.toml melanggar Golden Rules: {0}")]
    GoldenRule(String),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum Mode {
    #[default]
    Paper,
    Testnet,
    Live,
}

impl std::str::FromStr for Mode {
    type Err = String;
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.to_ascii_lowercase().as_str() {
            "paper" => Ok(Mode::Paper),
            "testnet" => Ok(Mode::Testnet),
            "live" | "mainnet" => Ok(Mode::Live),
            other => Err(format!("mode tidak dikenal: {other} (paper|testnet|live)")),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct RiskConfig {
    pub start_balance_usd: f64,
    pub risk_usd: f64,
    pub min_risk_usd: f64,
    pub sl_min_pct: f64,
    pub sl_max_pct: f64,
    pub leverage: u32,
    pub margin_type: String,
    pub max_positions: usize,
    pub daily_sl_limit: u32,
    pub daily_loss_limit_usd: f64,
    pub cooldown_ms: i64,
    pub equity_floor_usd: f64,
    pub rr: f64,
    pub max_spread_bps: f64,
    pub fee_maker: f64,
    pub fee_taker: f64,
    pub slip_sl: f64,
}

impl Default for RiskConfig {
    fn default() -> Self {
        RiskConfig {
            start_balance_usd: 60.0,
            risk_usd: 1.0,
            min_risk_usd: 0.80,
            sl_min_pct: 0.02,
            sl_max_pct: 0.04,
            leverage: 25,
            margin_type: "CROSSED".into(),
            max_positions: 2,
            daily_sl_limit: 6,
            daily_loss_limit_usd: 6.0,
            cooldown_ms: 3_600_000,
            equity_floor_usd: 40.0,
            rr: 2.0,
            max_spread_bps: 3.0,
            fee_maker: 0.0002,
            fee_taker: 0.0005,
            slip_sl: 0.0002,
        }
    }
}

impl RiskConfig {
    /// Golden Rules tidak boleh berubah tanpa revisi eksplisit user (Rules §6.4).
    pub fn validate_golden_rules(&self) -> Result<(), ConfigError> {
        let gr = RiskConfig::default();
        let mut bad = Vec::new();
        let eq = |a: f64, b: f64| (a - b).abs() < 1e-12;
        if !eq(self.start_balance_usd, gr.start_balance_usd) {
            bad.push("start_balance_usd != 60");
        }
        if !eq(self.risk_usd, gr.risk_usd) {
            bad.push("risk_usd != 1.00 (GR-1)");
        }
        if !eq(self.min_risk_usd, gr.min_risk_usd) {
            bad.push("min_risk_usd != 0.80 (GR-2)");
        }
        if !eq(self.sl_min_pct, gr.sl_min_pct) || !eq(self.sl_max_pct, gr.sl_max_pct) {
            bad.push("SL range != 2%..4% (GR-3)");
        }
        if self.leverage != 25 || self.margin_type != "CROSSED" {
            bad.push("leverage/margin != 25x CROSSED (GR-4)");
        }
        if self.max_positions != 2 {
            bad.push("max_positions != 2 (GR-5)");
        }
        if self.daily_sl_limit != 6 || !eq(self.daily_loss_limit_usd, 6.0) {
            bad.push("daily stop != 6 SL / -$6 (GR-6)");
        }
        if self.cooldown_ms != 3_600_000 {
            bad.push("cooldown != 60 menit (GR-7)");
        }
        if !eq(self.equity_floor_usd, 40.0) {
            bad.push("equity_floor != 40 (GR-10)");
        }
        if !eq(self.rr, 2.0) {
            bad.push("rr != 2.0");
        }
        if bad.is_empty() {
            Ok(())
        } else {
            Err(ConfigError::GoldenRule(bad.join("; ")))
        }
    }

    pub fn d(x: f64) -> Decimal {
        crate::num::to_dec(x)
    }
}

/// Kebijakan posisi yatim (posisi tanpa SL saat reconcile).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum OrphanPolicy {
    #[default]
    PlaceSl,
    Flatten,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum EntryOrderType {
    #[default]
    LimitGtx,
    Market,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct EngineConfig {
    /// paper (default) | testnet | live
    pub mode: Mode,
    /// ARM default false. Hanya user yang mengubahnya (GR-9).
    pub arm: bool,
    pub universe: Vec<String>,
    pub data_dir: PathBuf,
    pub state_dir: PathBuf,
    pub kill_file: PathBuf,
    pub live_rest_base: String,
    pub live_ws_base: String,
    pub testnet_rest_base: String,
    pub testnet_ws_base: String,
    pub public_rest_base: String,
    pub recv_window_ms: u64,
    pub max_clock_offset_ms: i64,
    pub entry_order_type: EntryOrderType,
    pub pending_ttl_ms: i64,
    pub protection_timeout_ms: u64,
    pub protection_retries: u32,
    pub orphan_policy: OrphanPolicy,
    pub flatten_on_kill: bool,
    /// true = SL/TP algo `closePosition=true` (default plan). false = quantity + reduceOnly.
    pub protective_close_position: bool,
    pub poll_interval_secs: u64,
}

impl Default for EngineConfig {
    fn default() -> Self {
        EngineConfig {
            mode: Mode::Paper,
            arm: false,
            universe: Vec::new(),
            data_dir: PathBuf::from("data"),
            state_dir: PathBuf::from("state"),
            kill_file: PathBuf::from("state").join("KILL"),
            live_rest_base: "https://fapi.binance.com".into(),
            live_ws_base: "wss://fstream.binance.com".into(),
            testnet_rest_base: "https://demo-fapi.binance.com".into(),
            testnet_ws_base: "wss://demo-fstream.binance.com".into(),
            public_rest_base: "https://fapi.binance.com".into(),
            recv_window_ms: 5_000,
            max_clock_offset_ms: 500,
            entry_order_type: EntryOrderType::LimitGtx,
            pending_ttl_ms: 3_600_000,
            protection_timeout_ms: 5_000,
            protection_retries: 3,
            orphan_policy: OrphanPolicy::PlaceSl,
            flatten_on_kill: false,
            protective_close_position: true,
            poll_interval_secs: 5,
        }
    }
}

impl EngineConfig {
    pub fn rest_base(&self) -> &str {
        match self.mode {
            Mode::Live => &self.live_rest_base,
            Mode::Testnet | Mode::Paper => &self.testnet_rest_base,
        }
    }
    pub fn ws_base(&self) -> &str {
        match self.mode {
            Mode::Live => &self.live_ws_base,
            Mode::Testnet | Mode::Paper => &self.testnet_ws_base,
        }
    }
    /// Pastikan `live_ws_base` dan `testnet_ws_base` hanya berisi host akar WebSocket
    /// karena rute `/public`, `/market`, dan `/private` ditambahkan otomatis oleh klien WS.
    pub fn validate_ws_urls(&self) -> Result<(), String> {
        validate_ws_root("live_ws_base", &self.live_ws_base)?;
        validate_ws_root("testnet_ws_base", &self.testnet_ws_base)
    }
}

fn validate_ws_root(field: &str, raw: &str) -> Result<(), String> {
    let trimmed = raw.trim().trim_end_matches('/');
    let rest = trimmed
        .strip_prefix("wss://")
        .or_else(|| trimmed.strip_prefix("ws://"))
        .ok_or_else(|| {
            format!(
                "konfigurasi URL WebSocket tidak valid ({field}): nilai '{raw}' harus diawali 'wss://' atau 'ws://' (contoh: 'wss://fstream.binance.com')"
            )
        })?;
    if rest.is_empty() || rest.contains('?') {
        return Err(format!(
            "konfigurasi URL WebSocket tidak valid ({field}): nilai '{raw}' hanya boleh berisi skema dan host akar tanpa query string"
        ));
    }
    if let Some((_, path)) = rest.split_once('/') {
        let p = format!("/{}", path.to_ascii_lowercase());
        if p == "/ws" || p.starts_with("/ws/") || p == "/stream" || p.starts_with("/stream?") || p.starts_with("/stream/") {
            return Err(format!(
                "konfigurasi URL WebSocket tidak valid ({field}): nilai '{raw}' memakai path WebSocket lama ('{p}') yang sudah dimatikan sejak 2026-04-23; hapus path dan isi host akar saja (contoh: 'wss://fstream.binance.com' atau 'wss://demo-fstream.binance.com')"
            ));
        }
        if p.ends_with("/public") || p.ends_with("/market") || p.ends_with("/private") {
            return Err(format!(
                "konfigurasi URL WebSocket tidak valid ({field}): nilai '{raw}' sudah berakhiran rute kategori ('{p}'); hapus akhiran tersebut dan isi host akar saja karena rute '/public', '/market', atau '/private' ditambahkan otomatis di kode"
            ));
        }
        return Err(format!(
            "konfigurasi URL WebSocket tidak valid ({field}): nilai '{raw}' tidak boleh memiliki path tambahan ('{p}'); isi host akar saja"
        ));
    }
    Ok(())
}

pub fn load_toml<T: for<'de> Deserialize<'de> + Default>(path: &Path) -> Result<T, ConfigError> {
    if !path.exists() {
        return Ok(T::default());
    }
    let s = std::fs::read_to_string(path).map_err(|e| ConfigError::Io(path.to_path_buf(), e))?;
    toml::from_str(&s).map_err(|e| ConfigError::Parse(path.to_path_buf(), e))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_are_safe() {
        let e = EngineConfig::default();
        assert_eq!(e.mode, Mode::Paper);
        assert!(!e.arm);
        assert!(e.testnet_rest_base.contains("demo-fapi"));
        RiskConfig::default().validate_golden_rules().unwrap();
    }

    #[test]
    fn golden_rule_violation_rejected() {
        let r = RiskConfig { risk_usd: 2.0, ..Default::default() };
        assert!(r.validate_golden_rules().is_err());
        let r = RiskConfig { leverage: 50, ..Default::default() };
        assert!(r.validate_golden_rules().is_err());
        let r = RiskConfig { max_positions: 3, ..Default::default() };
        assert!(r.validate_golden_rules().is_err());
    }

    #[test]
    fn parse_engine_toml_partial() {
        let e: EngineConfig = toml::from_str("mode = \"testnet\"\nuniverse=[\"ETHUSDT\"]").unwrap();
        assert_eq!(e.mode, Mode::Testnet);
        assert!(!e.arm);
        assert_eq!(e.rest_base(), "https://demo-fapi.binance.com");
    }

    #[test]
    fn ws_base_rejects_legacy_and_routed_paths() {
        let e = EngineConfig::default();
        assert_eq!(e.testnet_ws_base, "wss://demo-fstream.binance.com");
        e.validate_ws_urls().unwrap();

        for bad in [
            "wss://fstream.binance.com/ws",
            "wss://fstream.binance.com/ws/btcusdt@kline_15m",
            "wss://fstream.binance.com/stream",
            "wss://fstream.binance.com/stream?streams=btcusdt@kline_15m",
            "wss://fstream.binance.com/public",
            "wss://fstream.binance.com/market",
            "wss://demo-fstream.binance.com/private/",
            "https://fstream.binance.com",
        ] {
            let cfg = EngineConfig { live_ws_base: bad.into(), ..Default::default() };
            let err = cfg.validate_ws_urls().unwrap_err().to_string();
            assert!(err.contains("live_ws_base"), "expected field name in error: {err}");
        }
    }
}
