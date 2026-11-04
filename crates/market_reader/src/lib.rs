//! Market Reader (MARKET_READER.md): reader MR-01..MR-10 → aggregator 6 state (RG-0..RG-5)
//! dengan histeresis RG-6, dwell RG-7, shock hold 30 menit, override crowding RG-8.
pub mod aggregate;
pub mod readers;
pub mod source;

pub use aggregate::{RegimeDecision, RegimeMachine, RegimeState, UniverseScope};
pub use readers::*;
pub use source::{DomSource, SourceSelector, TvSnapshot};
pub mod reversal;
pub mod reversal_context;
