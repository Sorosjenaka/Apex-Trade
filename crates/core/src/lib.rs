//! Apex Scalper RS — core: tipe dasar, config, sizing (Rules §4.1), gate risk (GR-1..GR-10),
//! waktu UTC, audit log. Dipakai bersama oleh backtest dan engine live (NFR-18).
pub mod audit;
pub mod config;
pub mod filters;
pub mod num;
pub mod risk;
pub mod sizing;
pub mod time;
pub mod types;

pub use config::{EngineConfig, Mode, RiskConfig};
pub use filters::SymbolFilters;
pub use risk::{ExitKind, GateContext, RiskState};
pub use sizing::{size_order, EntryKind, Sizing, SizingInput};
pub use types::{Bar, Side, SkipReason, Tf};
