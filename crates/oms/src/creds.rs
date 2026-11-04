//! Kredensial HANYA dari env proses (BINANCE_API_KEY / BINANCE_API_SECRET).
//! Tidak membaca file .env apa pun. Debug/Display diredaksi.
use hmac::{Hmac, Mac};
use sha2::Sha256;

pub const KEY_ENV: &str = "BINANCE_API_KEY";
pub const SECRET_ENV: &str = "BINANCE_API_SECRET";

#[derive(Clone)]
pub struct Credentials {
    api_key: String,
    secret: String,
}

impl std::fmt::Debug for Credentials {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("Credentials { api_key: \"***\", secret: \"***\" }")
    }
}

impl Credentials {
    pub fn new(api_key: String, secret: String) -> Self {
        Credentials { api_key, secret }
    }
    pub fn from_env() -> anyhow::Result<Self> {
        let k = std::env::var(KEY_ENV).map_err(|_| anyhow::anyhow!("env {KEY_ENV} kosong"))?;
        let s = std::env::var(SECRET_ENV).map_err(|_| anyhow::anyhow!("env {SECRET_ENV} kosong"))?;
        if k.trim().is_empty() || s.trim().is_empty() {
            anyhow::bail!("env {KEY_ENV}/{SECRET_ENV} kosong");
        }
        Ok(Credentials { api_key: k.trim().to_string(), secret: s.trim().to_string() })
    }
    pub fn api_key(&self) -> &str {
        &self.api_key
    }
    /// HMAC-SHA256 hex dari query string.
    pub fn sign(&self, payload: &str) -> String {
        let mut mac = Hmac::<Sha256>::new_from_slice(self.secret.as_bytes()).expect("hmac key");
        mac.update(payload.as_bytes());
        hex::encode(mac.finalize().into_bytes())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn binance_doc_signature_vector() {
        // Vektor contoh dari dokumentasi Binance (key/secret publik dokumentasi, bukan milik user).
        let c = Credentials::new("dummy".into(), "NhqPtmdSJYdKjVHjA7PZj4Mge3R5YNiP1e3UZjInClVN65XAbvqqM6A7H5fATj0j".into());
        let q = "symbol=LTCBTC&side=BUY&type=LIMIT&timeInForce=GTC&quantity=1&price=0.1&recvWindow=5000&timestamp=1499827319559";
        assert_eq!(c.sign(q), "c8db56825ae71d6d79447849e617115f4a920fa2acdcab2b053c4b2838bd6b71");
        assert!(!format!("{c:?}").contains("NhqP"));
    }
}
