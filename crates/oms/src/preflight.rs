//! Pemeriksaan pra-terbang read-only (`apex preflight --target demo|live`, WP-9a).
//!
//! Modul ini **tidak pernah mengirim order**. Setiap pemeriksaan C1..C15 menghasilkan
//! `Pass`, `Warn`, atau `Fail` beserta alasan terukur. Proses keluar dengan kode bukan nol
//! apabila terdapat minimal satu `Fail`. Nilai rahasia environment **tidak pernah** dicetak
//! atau disimpan ke struktur laporan — hanya nama variabel dan status keberadaan/masker.

use crate::creds::{KEY_ENV, SECRET_ENV};
use crate::killswitch::KillSwitch;
use crate::notifier::NOTIFY_URL_ENV;
use crate::preflight_checks::*;
use crate::preflight_http::PreflightBackend;
use crate::safety::{LIVE_CONFIRM_ENV, LIVE_CONFIRM_VALUE};
use apex_core::config::{EngineConfig, RiskConfig};
use apex_core::time::now_ms;
use serde::{Deserialize, Serialize};

/// Kapasitas ruang disk kosong minimum pada direktori state/audit (100 MiB).
pub const MIN_FREE_DISK_BYTES: u64 = 100 * 1024 * 1024;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum PreflightTarget {
    Demo,
    Live,
}

impl PreflightTarget {
    pub fn parse(raw: &str) -> Result<Self, String> {
        match raw.trim().to_ascii_lowercase().as_str() {
            "demo" | "testnet" => Ok(Self::Demo),
            "live" | "mainnet" => Ok(Self::Live),
            other => Err(format!(
                "target preflight tidak dikenal '{other}' (gunakan 'demo' atau 'live')"
            )),
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Self::Demo => "demo",
            Self::Live => "live",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "UPPERCASE")]
pub enum CheckVerdict {
    Pass,
    Warn,
    Fail,
}

impl CheckVerdict {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Pass => "PASS",
            Self::Warn => "WARN",
            Self::Fail => "FAIL",
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct PreflightCheck {
    pub id: String,
    pub title: String,
    pub verdict: CheckVerdict,
    pub detail: String,
}

#[derive(Clone, Debug)]
pub struct EnvPresence {
    pub api_key_set: bool,
    pub api_secret_set: bool,
    pub live_confirm_set: bool,
    pub live_confirm_matches: bool,
    pub notify_url_set: bool,
}

impl EnvPresence {
    /// Baca keberadaan variabel environment tanpa menyimpan nilainya di memori laporan (H2).
    pub fn from_process_env() -> Self {
        let key_ok = std::env::var(KEY_ENV)
            .map(|v| !v.trim().is_empty())
            .unwrap_or(false);
        let sec_ok = std::env::var(SECRET_ENV)
            .map(|v| !v.trim().is_empty())
            .unwrap_or(false);
        let (lc_set, lc_match) = match std::env::var(LIVE_CONFIRM_ENV) {
            Ok(v) if !v.trim().is_empty() => (true, v.trim() == LIVE_CONFIRM_VALUE),
            _ => (false, false),
        };
        let notify_ok = std::env::var(NOTIFY_URL_ENV)
            .map(|v| !v.trim().is_empty())
            .unwrap_or(false);
        Self {
            api_key_set: key_ok,
            api_secret_set: sec_ok,
            live_confirm_set: lc_set,
            live_confirm_matches: lc_match,
            notify_url_set: notify_ok,
        }
    }
}

#[derive(Clone, Debug)]
pub struct PreflightInput {
    pub target: PreflightTarget,
    pub notify_test: bool,
    pub engine: EngineConfig,
    pub risk: RiskConfig,
    pub universe: Vec<String>,
    pub enabled_strategies: Vec<(String, String)>,
    pub env: EnvPresence,
    pub config_hash: String,
    pub binary_version: String,
    pub git_commit: String,
    pub allow_local_mock_hosts: bool,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct PreflightReport {
    pub target: PreflightTarget,
    pub config_hash: String,
    pub binary_version: String,
    pub git_commit: String,
    pub checks: Vec<PreflightCheck>,
}

impl PreflightReport {
    pub fn has_failure(&self) -> bool {
        self.checks.iter().any(|c| c.verdict == CheckVerdict::Fail)
    }
}

pub async fn run_preflight<B: PreflightBackend>(
    inp: &PreflightInput,
    backend: &B,
) -> PreflightReport {
    let mut checks = Vec::with_capacity(15);
    checks.push(check_c01_env(inp));
    checks.push(check_c02_mode_arm(inp));
    checks.push(check_c03_clock(inp, backend).await);
    checks.push(check_c04_hosts_and_ws(inp, backend).await);
    checks.push(check_c05_strategy_go(inp));
    checks.push(check_c06_api_restrictions(inp, backend).await);
    checks.push(check_c07_account_mode(backend).await);
    checks.push(check_c08_symbol_configs(inp, backend).await);
    checks.push(check_c09_balance(inp, backend).await);
    checks.push(check_c10_exchange_info_sizing(inp, backend).await);
    checks.push(check_c11_commission_rates(inp, backend).await);
    checks.push(check_c12_listen_key(backend).await);
    checks.push(check_c13_state_fsync_disk(inp, backend).await);
    checks.push(check_c14_kill_and_version(inp));
    checks.push(check_c15_notifier(inp, backend).await);
    PreflightReport {
        target: inp.target,
        config_hash: inp.config_hash.clone(),
        binary_version: inp.binary_version.clone(),
        git_commit: inp.git_commit.clone(),
        checks,
    }
}

pub fn check_c14_kill_and_version(inp: &PreflightInput) -> PreflightCheck {
    let ks = KillSwitch::new(&inp.engine.kill_file);
    let meta = format!(
        "version={} git_commit={}",
        inp.binary_version, inp.git_commit
    );
    if ks.is_engaged() {
        chk(
            "C14",
            "Status file KILL & identitas biner",
            CheckVerdict::Fail,
            format!(
                "file KILL aktif ({}): {} | {meta}",
                ks.path().display(),
                ks.reason().unwrap_or_default()
            ),
        )
    } else if inp.git_commit.trim().is_empty() || inp.git_commit == "unknown" {
        chk(
            "C14",
            "Status file KILL & identitas biner",
            CheckVerdict::Warn,
            format!("KILL tidak aktif namun git_commit tidak terdeteksi | {meta}"),
        )
    } else {
        chk(
            "C14",
            "Status file KILL & identitas biner",
            CheckVerdict::Pass,
            format!("KILL tidak aktif | {meta}"),
        )
    }
}

pub async fn check_c15_notifier<B: PreflightBackend>(
    inp: &PreflightInput,
    backend: &B,
) -> PreflightCheck {
    if !inp.notify_test {
        return if inp.env.notify_url_set {
            chk(
                "C15",
                "Konektivitas notifier (--notify-test)",
                CheckVerdict::Pass,
                format!(
                    "{NOTIFY_URL_ENV} terdeteksi (<masked>); pengiriman uji dilewati tanpa --notify-test"
                ),
            )
        } else {
            chk(
                "C15",
                "Konektivitas notifier (--notify-test)",
                CheckVerdict::Warn,
                format!(
                    "{NOTIFY_URL_ENV} belum diset; tidak ada pengiriman uji tanpa --notify-test"
                ),
            )
        };
    }
    match backend.send_notify_test(now_ms()).await {
        Ok(status) if (200..300).contains(&status) => chk(
            "C15",
            "Konektivitas notifier (--notify-test)",
            CheckVerdict::Pass,
            format!("notifikasi uji terkirim (HTTP {status})"),
        ),
        Ok(status) => chk(
            "C15",
            "Konektivitas notifier (--notify-test)",
            CheckVerdict::Fail,
            format!("webhook notifier mengembalikan status gagal HTTP {status}"),
        ),
        Err(e) => chk(
            "C15",
            "Konektivitas notifier (--notify-test)",
            CheckVerdict::Fail,
            format!("pengiriman notifikasi uji gagal: {e}"),
        ),
    }
}

pub fn format_preflight_table(rep: &PreflightReport) -> String {
    let mut out = String::new();
    out.push_str(&format!(
        "=== APEX PREFLIGHT (target={}, version={}, commit={}, config_hash={}) ===\n",
        rep.target.as_str(),
        rep.binary_version,
        rep.git_commit,
        rep.config_hash
    ));
    for c in &rep.checks {
        out.push_str(&format!(
            "[{:<4}] {:<3} {:<56} | {}\n",
            c.verdict.as_str(),
            c.id,
            c.title,
            c.detail
        ));
    }
    let overall = if rep.has_failure() { "FAIL" } else { "PASS" };
    out.push_str(&format!("OVERALL: {overall}\n"));
    out
}

pub fn detect_git_commit_short() -> String {
    std::process::Command::new("git")
        .args(["rev-parse", "--short=12", "HEAD"])
        .output()
        .ok()
        .filter(|o| o.status.success())
        .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string())
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| "unknown".into())
}
