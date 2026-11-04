//! Screener: allowlist crypto-only (BLACKLIST.md), volume gate (VOLUME_GATE.md),
//! regime per simbol anti-sideways (SCREENER.md L3 RC-1..RC-9) + histeresis.
pub mod allowlist;
pub mod regime;
pub mod volume_gate;

pub use allowlist::{classify_symbol, AllowDecision, Allowlist};
pub use regime::{ScreenerConfig, ScreenerMachine, SymState, SymbolFeatures, SymbolRegime};
pub use volume_gate::{GateMetrics, VolumeGateConfig, VolumeGateTracker};
