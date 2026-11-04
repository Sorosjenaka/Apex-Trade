//! Data layer: Binance USDⓈ-M PUBLIC REST/WS (tanpa key), TradingView scanner + chart WS (anonim),
//! CSV store + recorder. Tidak ada endpoint bertanda tangan di crate ini.
pub mod csvio;
pub mod rest;
pub mod tv;
pub mod ws;

pub use csvio::{load_funding, load_klines, save_klines, FundingRow, Recorder};
pub use rest::PublicClient;

/// Pasang provider kripto rustls (ring) sekali per proses — tanpa aws-lc/openssl (aman build Windows MSVC).
pub fn init_tls() {
    let _ = rustls::crypto::ring::default_provider().install_default();
}
