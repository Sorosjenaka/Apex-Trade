//! Allowlist crypto-native default-deny (BLACKLIST.md BL-R0..BL-R7).
use serde_json::Value;
use std::collections::BTreeSet;

pub const COMMODITY_TOKENS: &[&str] = &["PAXG", "XAUT"];
pub const STABLECOINS: &[&str] = &["BUSD", "DAI", "EUR", "EURI", "FDUSD", "PYUSD", "RLUSD", "TUSD", "USD1", "USDC", "USDE", "USDP"];

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum AllowDecision {
    Allow,
    Deny { rule: &'static str },
}

fn s<'a>(v: &'a Value, k: &str) -> &'a str {
    v.get(k).and_then(|x| x.as_str()).unwrap_or("")
}

/// Klasifikasi satu elemen `symbols[]` exchangeInfo. Default DENY.
pub fn classify_symbol(v: &Value) -> AllowDecision {
    let ct = s(v, "contractType");
    let ut = s(v, "underlyingType");
    let subs: Vec<&str> = v
        .get("underlyingSubType")
        .and_then(|x| x.as_array())
        .map(|a| a.iter().filter_map(|x| x.as_str()).collect())
        .unwrap_or_default();
    let base = s(v, "baseAsset");
    if ct == "TRADIFI_PERPETUAL" {
        return AllowDecision::Deny { rule: "BL-R1" };
    }
    if subs.iter().any(|x| x.eq_ignore_ascii_case("TradFi")) {
        return AllowDecision::Deny { rule: "BL-R2" };
    }
    if ut == "INDEX" {
        return AllowDecision::Deny { rule: "BL-R3" };
    }
    if ut == "PREMARKET" || subs.iter().any(|x| x.eq_ignore_ascii_case("Pre-Market")) {
        return AllowDecision::Deny { rule: "BL-R4" };
    }
    if ut != "COIN" {
        return AllowDecision::Deny { rule: "BL-R5" };
    }
    if COMMODITY_TOKENS.contains(&base) {
        return AllowDecision::Deny { rule: "BL-R6" };
    }
    if STABLECOINS.contains(&base) {
        return AllowDecision::Deny { rule: "BL-R7" };
    }
    if ct != "PERPETUAL" || s(v, "quoteAsset") != "USDT" || s(v, "status") != "TRADING" {
        return AllowDecision::Deny { rule: "BL-R0" };
    }
    AllowDecision::Allow
}

#[derive(Clone, Debug, Default)]
pub struct Allowlist {
    pub symbols: BTreeSet<String>,
}

impl Allowlist {
    /// Dari `/fapi/v1/exchangeInfo` (BL-R8: re-evaluasi start + tiap 1 jam).
    pub fn from_exchange_info(ei: &Value) -> (Allowlist, Vec<(String, &'static str)>) {
        let mut a = Allowlist::default();
        let mut denied = Vec::new();
        for sym in ei.get("symbols").and_then(|x| x.as_array()).into_iter().flatten() {
            let name = s(sym, "symbol").to_string();
            match classify_symbol(sym) {
                AllowDecision::Allow => {
                    a.symbols.insert(name);
                }
                AllowDecision::Deny { rule } => denied.push((name, rule)),
            }
        }
        (a, denied)
    }
    /// Dari `config/blacklist.json` (snapshot) — fallback offline.
    pub fn from_blacklist_json(v: &Value) -> Allowlist {
        let symbols = v
            .get("crypto_allowlist_usdt")
            .and_then(|x| x.as_array())
            .map(|a| a.iter().filter_map(|x| x.as_str().map(String::from)).collect())
            .unwrap_or_default();
        Allowlist { symbols }
    }
    pub fn contains(&self, s: &str) -> bool {
        self.symbols.contains(s)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn sym(name: &str, base: &str, ct: &str, ut: &str, subs: &[&str]) -> Value {
        json!({"symbol": name, "baseAsset": base, "quoteAsset": "USDT", "status": "TRADING", "contractType": ct, "underlyingType": ut, "underlyingSubType": subs})
    }

    #[test]
    fn crypto_allowed_others_denied() {
        assert_eq!(classify_symbol(&sym("ETHUSDT", "ETH", "PERPETUAL", "COIN", &["Layer-1"])), AllowDecision::Allow);
        assert_eq!(classify_symbol(&sym("TSLAUSDT", "TSLA", "TRADIFI_PERPETUAL", "EQUITY", &["TradFi"])), AllowDecision::Deny { rule: "BL-R1" });
        assert_eq!(classify_symbol(&sym("XAUUSDT", "XAU", "PERPETUAL", "COMMODITY", &["TradFi"])), AllowDecision::Deny { rule: "BL-R2" });
        assert_eq!(classify_symbol(&sym("BTCDOMUSDT", "BTCDOM", "PERPETUAL", "INDEX", &[])), AllowDecision::Deny { rule: "BL-R3" });
        assert_eq!(classify_symbol(&sym("XUSDT", "X", "PERPETUAL", "PREMARKET", &[])), AllowDecision::Deny { rule: "BL-R4" });
        assert_eq!(classify_symbol(&sym("NEWUSDT", "NEW", "PERPETUAL", "KR_EQUITY", &[])), AllowDecision::Deny { rule: "BL-R5" });
        assert_eq!(classify_symbol(&sym("PAXGUSDT", "PAXG", "PERPETUAL", "COIN", &[])), AllowDecision::Deny { rule: "BL-R6" });
        assert_eq!(classify_symbol(&sym("USDCUSDT", "USDC", "PERPETUAL", "COIN", &[])), AllowDecision::Deny { rule: "BL-R7" });
        assert_eq!(classify_symbol(&sym("ETHUSDT_260327", "ETH", "CURRENT_QUARTER", "COIN", &[])), AllowDecision::Deny { rule: "BL-R0" });
        // tipe baru tak dikenal → default deny
        assert!(matches!(classify_symbol(&json!({"symbol":"ZZZUSDT"})), AllowDecision::Deny { .. }));
    }

    /// Semua entri blacklist di config/blacklist.json harus ter-deny oleh aturan kode, dan
    /// semua entri allowlist harus Allow (paritas dengan snapshot plan).
    #[test]
    fn parity_with_blacklist_json() {
        let p = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../config/blacklist.json");
        let v: Value = serde_json::from_str(&std::fs::read_to_string(p).unwrap()).unwrap();
        let mut n = 0;
        for e in v["blacklist"].as_array().unwrap() {
            if e["quoteAsset"] != "USDT" {
                continue;
            }
            let mut x = e.clone();
            x["status"] = json!("TRADING");
            assert!(matches!(classify_symbol(&x), AllowDecision::Deny { .. }), "{}", e["symbol"]);
            n += 1;
        }
        assert_eq!(n, 240);
        let a = Allowlist::from_blacklist_json(&v);
        assert_eq!(a.symbols.len(), 516);
        assert!(a.contains("ETHUSDT") && !a.contains("TSLAUSDT"));
    }
}
