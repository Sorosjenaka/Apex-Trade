//! Muat semua file config/*.toml (file tidak ada → default aman).
use apex_core::config::{load_toml, EngineConfig, RiskConfig};
use apex_core::filters::SymbolFilters;
use apex_screener::regime::ScreenerConfig;
use apex_screener::volume_gate::VolumeGateConfig;
use apex_strategies::{StrategyId, StrategyParams};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct StrategiesFile {
    /// urutan = prioritas
    pub enabled: Vec<String>,
    /// HYPOTHESIS | NO-GO | GO. Hanya GO yang boleh order testnet/live.
    pub status: BTreeMap<String, String>,
    pub params: StrategyParams,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct ScreenerFile {
    pub regime: ScreenerConfig,
    pub volume_gate: VolumeGateConfig,
}

pub struct AllConfig {
    pub dir: PathBuf,
    pub engine: EngineConfig,
    pub risk: RiskConfig,
    pub screener: ScreenerFile,
    pub strategies: StrategiesFile,
}

impl AllConfig {
    pub fn load(dir: &Path) -> anyhow::Result<AllConfig> {
        let engine: EngineConfig = load_toml(&dir.join("engine.toml"))?;
        let risk: RiskConfig = load_toml(&dir.join("risk.toml"))?;
        risk.validate_golden_rules()?;
        let screener: ScreenerFile = load_toml(&dir.join("screener.toml"))?;
        let strategies: StrategiesFile = load_toml(&dir.join("strategies.toml"))?;
        Ok(AllConfig { dir: dir.to_path_buf(), engine, risk, screener, strategies })
    }
    pub fn enabled(&self) -> Vec<StrategyId> {
        let v: Vec<StrategyId> = self.strategies.enabled.iter().filter_map(|s| StrategyId::parse(s)).collect();
        if v.is_empty() {
            StrategyId::ALL.to_vec()
        } else {
            v
        }
    }
    pub fn is_go(&self, id: StrategyId) -> bool {
        self.strategies.status.get(id.code()).is_some_and(|s| s.eq_ignore_ascii_case("GO"))
    }
    pub fn universe(&self) -> Vec<String> {
        if self.engine.universe.is_empty() {
            DEFAULT_UNIVERSE.iter().map(|s| s.to_string()).collect()
        } else {
            self.engine.universe.clone()
        }
    }
}

pub const DEFAULT_UNIVERSE: &[&str] = &["ETHUSDT", "SOLUSDT", "XRPUSDT", "DOGEUSDT", "SUIUSDT", "LINKUSDT", "ENAUSDT", "NEARUSDT", "1000PEPEUSDT", "HYPEUSDT", "WLDUSDT"];

/// Filter dari file exchangeInfo (format `{"symbols":[...]}`).
pub fn load_filters(path: &Path) -> anyhow::Result<BTreeMap<String, SymbolFilters>> {
    let v: serde_json::Value = serde_json::from_str(&std::fs::read_to_string(path)?)?;
    Ok(filters_from_ei(&v))
}

pub fn filters_from_ei(v: &serde_json::Value) -> BTreeMap<String, SymbolFilters> {
    v["symbols"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(SymbolFilters::from_exchange_info_symbol)
        .map(|f| (f.symbol.clone(), f))
        .collect()
}

pub fn load_allowlist(dir: &Path) -> anyhow::Result<apex_screener::allowlist::Allowlist> {
    let v: serde_json::Value = serde_json::from_str(&std::fs::read_to_string(dir.join("blacklist.json"))?)?;
    Ok(apex_screener::allowlist::Allowlist::from_blacklist_json(&v))
}
