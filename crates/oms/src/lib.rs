//! Apex OMS — order management untuk Binance USDⓈ-M Futures (one-way, CROSSED, 25x).
//!
//! Prinsip:
//! * **Paper = tidak ada panggilan exchange**. Testnet butuh `arm=true`. Live butuh `arm=true`
//!   + `mode=live` + env `APEX_LIVE_CONFIRM=I_UNDERSTAND_LIVE_ORDERS` ([`safety`]).
//! * Key hanya dari env `BINANCE_API_KEY` / `BINANCE_API_SECRET`; tidak pernah di-log ([`creds`]).
//! * SL `STOP_MARKET` + TP `TAKE_PROFIT_MARKET` via `/fapi/v1/algoOrder` (`closePosition`, `MARK_PRICE`)
//!   dipasang setelah fill. User-data stream = sumber kebenaran ([`userstream`]).
//! * Reconcile saat boot: posisi yatim tanpa SL → pasang SL atau flatten.
//! * Close gagal **tidak** menghapus posisi dari buku; kill switch file + CLI.
//! * [`mock::MockExchange`] mengimplementasikan trait yang sama ([`exchange::FuturesExchange`]).
#![cfg_attr(
    not(test),
    deny(clippy::unwrap_used, clippy::expect_used, clippy::panic)
)]

pub mod artatrade_exec;
pub mod binance;
pub mod creds;
pub mod demo_smoke;
pub mod exchange;
pub mod forensics;
pub mod forward_gate;
pub mod invariants;
pub mod killswitch;
pub mod manager;
pub mod mock;
pub mod notifier;
pub mod preflight;
pub mod preflight_checks;
pub mod preflight_http;
pub mod safety;
pub mod store;
pub mod supervision;
pub mod telemetry;
pub mod types;
pub mod userstream;
pub mod yahudi_exec;

pub use artatrade_exec::*;
pub use demo_smoke::*;
pub use exchange::FuturesExchange;
pub use forensics::*;
pub use forward_gate::*;
pub use invariants::*;
pub use manager::{EntryIntent, ManagedTrade, OrderManager, TradeState};
pub use notifier::{HttpWebhookNotifier, NoopNotifier, Notifier, NotifyPayload, RecordingNotifier};
pub use preflight::*;
pub use preflight_http::*;
pub use safety::{authorize, ExecMode, SafetyError};
pub use store::{LoadOutcome, StateStore, StoreError};
pub use supervision::{RestartablePublicSupervisor, Supervised, TaskHealth};
pub use telemetry::*;
pub use types::*;
pub use yahudi_exec::*;
