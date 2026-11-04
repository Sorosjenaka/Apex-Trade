//! Safety guard (Rules GR-9, §6). Satu-satunya pintu menuju order sungguhan.
use apex_core::config::{EngineConfig, Mode};

pub const LIVE_CONFIRM_ENV: &str = "APEX_LIVE_CONFIRM";
pub const LIVE_CONFIRM_VALUE: &str = "I_UNDERSTAND_LIVE_ORDERS";

#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum ExecMode {
    /// Simulasi lokal; tidak ada panggilan exchange private.
    Paper,
    Testnet,
    Live,
}

#[derive(Debug, Clone, PartialEq, thiserror::Error)]
pub enum SafetyError {
    #[error("ARM=false: order testnet/live diblokir (ubah arm=true sendiri di config/engine.toml)")]
    NotArmed,
    #[error("mode live butuh env {LIVE_CONFIRM_ENV}={LIVE_CONFIRM_VALUE}")]
    MissingLiveConfirm,
    #[error("base URL {0} tidak cocok dengan mode {1:?}")]
    BaseMismatch(String, Mode),
    #[error("kill switch aktif: {0}")]
    Killed(String),
}

fn is_live_host(url: &str) -> bool {
    let u = url.to_ascii_lowercase();
    u.contains("fapi.binance.com") && !u.contains("demo-fapi") && !u.contains("testnet")
}

/// Putuskan mode eksekusi. `live_confirm` = nilai env APEX_LIVE_CONFIRM (dibaca pemanggil).
pub fn authorize(cfg: &EngineConfig, live_confirm: Option<&str>) -> Result<ExecMode, SafetyError> {
    match cfg.mode {
        Mode::Paper => Ok(ExecMode::Paper),
        Mode::Testnet => {
            if !cfg.arm {
                return Err(SafetyError::NotArmed);
            }
            if is_live_host(&cfg.testnet_rest_base) {
                return Err(SafetyError::BaseMismatch(cfg.testnet_rest_base.clone(), Mode::Testnet));
            }
            Ok(ExecMode::Testnet)
        }
        Mode::Live => {
            if !cfg.arm {
                return Err(SafetyError::NotArmed);
            }
            if live_confirm != Some(LIVE_CONFIRM_VALUE) {
                return Err(SafetyError::MissingLiveConfirm);
            }
            Ok(ExecMode::Live)
        }
    }
}

/// Cek base URL REST yang akan dipakai klien signed sesuai mode.
pub fn check_base(mode: ExecMode, base: &str) -> Result<(), SafetyError> {
    match mode {
        ExecMode::Paper => Err(SafetyError::BaseMismatch(base.into(), Mode::Paper)),
        ExecMode::Testnet if is_live_host(base) => Err(SafetyError::BaseMismatch(base.into(), Mode::Testnet)),
        ExecMode::Live if !is_live_host(base) => Err(SafetyError::BaseMismatch(base.into(), Mode::Live)),
        _ => Ok(()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_is_paper_and_unarmed() {
        let c = EngineConfig::default();
        assert_eq!(authorize(&c, Some(LIVE_CONFIRM_VALUE)).unwrap(), ExecMode::Paper);
    }

    #[test]
    fn live_needs_all_three() {
        let mut c = EngineConfig { mode: Mode::Live, ..Default::default() };
        assert_eq!(authorize(&c, Some(LIVE_CONFIRM_VALUE)), Err(SafetyError::NotArmed));
        c.arm = true;
        assert_eq!(authorize(&c, None), Err(SafetyError::MissingLiveConfirm));
        assert_eq!(authorize(&c, Some("yes")), Err(SafetyError::MissingLiveConfirm));
        assert_eq!(authorize(&c, Some(LIVE_CONFIRM_VALUE)), Ok(ExecMode::Live));
    }

    #[test]
    fn testnet_needs_arm_and_demo_host() {
        let mut c = EngineConfig { mode: Mode::Testnet, ..Default::default() };
        assert_eq!(authorize(&c, None), Err(SafetyError::NotArmed));
        c.arm = true;
        assert_eq!(authorize(&c, None), Ok(ExecMode::Testnet));
        c.testnet_rest_base = "https://fapi.binance.com".into();
        assert!(authorize(&c, None).is_err());
        assert!(check_base(ExecMode::Testnet, "https://fapi.binance.com").is_err());
        assert!(check_base(ExecMode::Testnet, "https://demo-fapi.binance.com").is_ok());
        assert!(check_base(ExecMode::Live, "https://demo-fapi.binance.com").is_err());
        assert!(check_base(ExecMode::Paper, "https://demo-fapi.binance.com").is_err());
    }
}
