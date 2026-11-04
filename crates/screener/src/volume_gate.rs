//! Volume gate V-01..V-09 + histeresis VG-02 (keluar setelah 3 gagal beruntun, masuk setelah 2 lolos beruntun).
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(default)]
pub struct VolumeGateConfig {
    pub min_quote_volume_24h: f64,
    pub min_trades_24h: u64,
    pub max_spread_bps: f64,
    pub min_depth_usd_0p1pct: f64,
    pub max_tick_bps: f64,
    pub max_min_order_usd: f64,
    pub max_step_usd: f64,
    pub min_age_days: f64,
    pub max_abs_funding_pct: f64,
    pub fail_out: u32,
    pub pass_in: u32,
}

impl Default for VolumeGateConfig {
    fn default() -> Self {
        VolumeGateConfig {
            min_quote_volume_24h: 20_000_000.0,
            min_trades_24h: 100_000,
            max_spread_bps: 5.0,
            min_depth_usd_0p1pct: 20_000.0,
            max_tick_bps: 5.0,
            max_min_order_usd: 25.0,
            max_step_usd: 5.0,
            min_age_days: 14.0,
            max_abs_funding_pct: 0.1,
            fail_out: 3,
            pass_in: 2,
        }
    }
}

/// Metrik mentah per simbol. `None` = data tidak tersedia → dihitung GAGAL (default deny).
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct GateMetrics {
    pub symbol: String,
    pub price: f64,
    pub quote_volume_24h: Option<f64>,
    pub trades_24h: Option<u64>,
    pub spread_bps: Option<f64>,
    /// min(depth bid, depth ask) dalam ±0,1% dari mid, USD.
    pub depth_usd: Option<f64>,
    pub tick_size: Option<f64>,
    pub step_size: Option<f64>,
    pub min_qty: Option<f64>,
    pub min_notional: Option<f64>,
    pub age_days: Option<f64>,
    pub funding_rate: Option<f64>,
}

impl GateMetrics {
    /// Kode V-xx yang gagal (kosong = lolos).
    pub fn failures(&self, c: &VolumeGateConfig) -> Vec<&'static str> {
        let mut f = Vec::new();
        let p = self.price;
        if !self.quote_volume_24h.is_some_and(|x| x >= c.min_quote_volume_24h) {
            f.push("V-01");
        }
        if !self.trades_24h.is_some_and(|x| x >= c.min_trades_24h) {
            f.push("V-02");
        }
        if !self.spread_bps.is_some_and(|x| x <= c.max_spread_bps) {
            f.push("V-03");
        }
        if !self.depth_usd.is_some_and(|x| x >= c.min_depth_usd_0p1pct) {
            f.push("V-04");
        }
        if !(p > 0.0 && self.tick_size.is_some_and(|t| t / p * 1e4 <= c.max_tick_bps)) {
            f.push("V-05");
        }
        let min_ok = self.min_notional.is_some_and(|m| m <= c.max_min_order_usd) && self.min_qty.is_some_and(|q| q * p <= c.max_min_order_usd);
        if !(p > 0.0 && min_ok) {
            f.push("V-06");
        }
        if !(p > 0.0 && self.step_size.is_some_and(|s| s * p <= c.max_step_usd)) {
            f.push("V-07");
        }
        if !self.age_days.is_some_and(|d| d >= c.min_age_days) {
            f.push("V-08");
        }
        if !self.funding_rate.is_some_and(|r| r.abs() * 100.0 <= c.max_abs_funding_pct) {
            f.push("V-09");
        }
        f
    }
}

/// Kedalaman USD dalam ±pct dari mid; input level [[price, qty], ...].
pub fn depth_within(bids: &[(f64, f64)], asks: &[(f64, f64)], pct: f64) -> Option<(f64, f64, f64)> {
    let bb = bids.first()?.0;
    let ba = asks.first()?.0;
    let mid = (bb + ba) / 2.0;
    let lo = mid * (1.0 - pct / 100.0);
    let hi = mid * (1.0 + pct / 100.0);
    let db: f64 = bids.iter().filter(|(p, _)| *p >= lo).map(|(p, q)| p * q).sum();
    let da: f64 = asks.iter().filter(|(p, _)| *p <= hi).map(|(p, q)| p * q).sum();
    Some((db.min(da), (ba - bb) / mid * 1e4, mid))
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct TrackState {
    pub included: bool,
    pub consec_fail: u32,
    pub consec_pass: u32,
    pub last_fail: Vec<String>,
}

/// Histeresis VG-02. Simbol baru mulai `excluded` dan butuh `pass_in` lolos beruntun.
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct VolumeGateTracker {
    pub symbols: BTreeMap<String, TrackState>,
}

impl VolumeGateTracker {
    pub fn update(&mut self, sym: &str, pass: bool, fails: &[&'static str], c: &VolumeGateConfig) -> bool {
        let st = self.symbols.entry(sym.to_string()).or_default();
        st.last_fail = fails.iter().map(|x| x.to_string()).collect();
        if pass {
            st.consec_pass += 1;
            st.consec_fail = 0;
            if !st.included && st.consec_pass >= c.pass_in {
                st.included = true;
            }
        } else {
            st.consec_fail += 1;
            st.consec_pass = 0;
            if st.included && st.consec_fail >= c.fail_out {
                st.included = false;
            }
        }
        st.included
    }
    pub fn included(&self, sym: &str) -> bool {
        self.symbols.get(sym).is_some_and(|s| s.included)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn good() -> GateMetrics {
        GateMetrics {
            symbol: "ETHUSDT".into(),
            price: 3000.0,
            quote_volume_24h: Some(5e9),
            trades_24h: Some(2_000_000),
            spread_bps: Some(0.1),
            depth_usd: Some(1e6),
            tick_size: Some(0.01),
            step_size: Some(0.001),
            min_qty: Some(0.001),
            min_notional: Some(20.0),
            age_days: Some(2000.0),
            funding_rate: Some(0.0001),
        }
    }

    #[test]
    fn each_rule() {
        let c = VolumeGateConfig::default();
        assert!(good().failures(&c).is_empty());
        let mut m = good();
        m.quote_volume_24h = Some(19e6);
        assert_eq!(m.failures(&c), vec!["V-01"]);
        let mut m = good();
        m.trades_24h = None;
        assert_eq!(m.failures(&c), vec!["V-02"]);
        let mut m = good();
        m.spread_bps = Some(6.0);
        m.depth_usd = Some(10_000.0);
        assert_eq!(m.failures(&c), vec!["V-03", "V-04"]);
        let mut m = good();
        m.tick_size = Some(2.0); // 6,7 bps
        assert_eq!(m.failures(&c), vec!["V-05"]);
        let mut m = good();
        m.min_qty = Some(0.01); // $30
        assert_eq!(m.failures(&c), vec!["V-06"]);
        let mut m = good();
        m.step_size = Some(0.002); // $6
        assert_eq!(m.failures(&c), vec!["V-07"]);
        let mut m = good();
        m.age_days = Some(5.0);
        m.funding_rate = Some(-0.0015);
        assert_eq!(m.failures(&c), vec!["V-08", "V-09"]);
    }

    #[test]
    fn hysteresis_vg02() {
        let c = VolumeGateConfig::default();
        let mut t = VolumeGateTracker::default();
        assert!(!t.update("A", true, &[], &c));
        assert!(t.update("A", true, &[], &c));
        assert!(t.update("A", false, &["V-01"], &c));
        assert!(t.update("A", false, &["V-01"], &c));
        assert!(t.update("A", true, &[], &c)); // reset counter gagal
        assert!(t.update("A", false, &["V-01"], &c));
        assert!(t.update("A", false, &["V-01"], &c));
        assert!(!t.update("A", false, &["V-01"], &c)); // 3 gagal beruntun → keluar
        assert!(!t.update("A", true, &[], &c));
        assert!(t.update("A", true, &[], &c));
    }

    #[test]
    fn depth_calc() {
        let (d, spr, _) = depth_within(&[(100.0, 100.0), (99.95, 100.0), (99.0, 1000.0)], &[(100.02, 300.0), (101.0, 5.0)], 0.1).unwrap();
        assert!((d - 19995.0).abs() < 1.0);
        assert!((spr - 2.0).abs() < 0.01);
    }
}
