//! Backtest event-driven TANPA look-ahead (Rules §4.3, architecture §backtest).
//!
//! Urutan per bar 15m [T, T+15m):
//! 1. funding untuk posisi terbuka (fundingTime ∈ [T, T+15m)),
//! 2. exit intrabar posisi terbuka: SL-first (SL & TP dalam bar yang sama → SL), gap → fill di open,
//! 3. order pending: TTL / fill limit hanya bila harga MENEMBUS ≥ 1 tick; bar fill: SL boleh kena, TP tidak,
//! 4. likuidasi cross atas SELURUH wallet (ekstrem adverse semua posisi serentak, konservatif),
//! 5. pada close T+15m: regime/screener/strategi dievaluasi dari bar tertutup → order baru aktif mulai bar berikutnya.
//!    MARKET dieksekusi di open bar berikutnya (+slip, fee taker).
pub mod context;
pub mod engine;
pub mod stats;

pub use context::{RegimeMode, RegimeTimeline};
pub use engine::{run_backtest, screener_states, BacktestConfig, BacktestReport, MarketInput, SymbolInput, Trade};
pub use stats::{dsr, monte_carlo, psr, Metrics, MonteCarlo};

use apex_core::types::Ms;

pub const HOLDOUT_START: &str = "2026-05-01";
pub const HOLDOUT_END_EXCL: &str = "2026-11-01";

/// True bila [start, end) beririsan dengan holdout terkunci 2026-05-01..2026-10-31.
pub fn overlaps_holdout(start: Ms, end: Ms) -> bool {
    let hs = apex_core::time::parse_ymd(HOLDOUT_START).unwrap_or(i64::MAX);
    let he = apex_core::time::parse_ymd(HOLDOUT_END_EXCL).unwrap_or(i64::MAX);
    start < he && end > hs
}

/// Hash parameter yang stabil lintas versi Rust/OS (SHA-256 JSON).
pub fn param_hash(v: &serde_json::Value) -> String {
    use sha2::{Digest, Sha256};
    hex::encode(Sha256::digest(v.to_string().as_bytes()))[..16].to_string()
}

/// Kunci holdout: tolak tanpa `unlock`; tolak bila hash parameter yang sama sudah pernah dijalankan di holdout.
pub fn holdout_guard(start: Ms, end: Ms, unlock: bool, hash: &str, ledger: &std::path::Path) -> anyhow::Result<bool> {
    if !overlaps_holdout(start, end) {
        return Ok(false);
    }
    if !unlock {
        anyhow::bail!("rentang menyentuh HOLDOUT terkunci {HOLDOUT_START}..2026-10-31. Pakai --unlock-holdout (sekali jalan per versi parameter, G-11).");
    }
    if let Ok(s) = std::fs::read_to_string(ledger) {
        if s.lines().any(|l| l.contains(&format!("\"param_hash\":\"{hash}\""))) {
            anyhow::bail!("holdout sudah pernah dijalankan untuk param_hash {hash} — G-11 sekali jalan; ditolak.");
        }
    }
    Ok(true)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn holdout_lock() {
        let d = |s| apex_core::time::parse_ymd(s).unwrap();
        assert!(!overlaps_holdout(d("2025-01-01"), d("2026-05-01")));
        assert!(overlaps_holdout(d("2026-04-01"), d("2026-05-02")));
        assert!(overlaps_holdout(d("2026-10-31"), d("2026-12-01")));
        assert!(!overlaps_holdout(d("2026-11-01"), d("2026-12-01")));
        let p = std::env::temp_dir().join(format!("apex_hl_{}.jsonl", std::process::id()));
        let _ = std::fs::remove_file(&p);
        assert!(holdout_guard(d("2026-05-01"), d("2026-06-01"), false, "abc", &p).is_err());
        assert!(holdout_guard(d("2026-05-01"), d("2026-06-01"), true, "abc", &p).unwrap());
        std::fs::write(&p, "{\"param_hash\":\"abc\"}\n").unwrap();
        assert!(holdout_guard(d("2026-05-01"), d("2026-06-01"), true, "abc", &p).is_err());
        assert!(!holdout_guard(d("2025-05-01"), d("2025-06-01"), false, "abc", &p).unwrap());
        let _ = std::fs::remove_file(&p);
    }
}
