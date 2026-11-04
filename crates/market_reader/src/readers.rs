//! Reader individual. Semua input = bar TERTUTUP (RD-00.1).
use apex_core::types::{Bar, Ms, Tf};
use apex_indicators as ind;
use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum Trend {
    Up,
    Down,
    None,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum Dir3 {
    Rising,
    Falling,
    Flat,
}

/// Arah dari perubahan 24h (%) dengan ambang FLAT.
pub fn dir_from_chg(chg_pct: f64, flat_pct: f64) -> Dir3 {
    if chg_pct.abs() < flat_pct {
        Dir3::Flat
    } else if chg_pct > 0.0 {
        Dir3::Rising
    } else {
        Dir3::Falling
    }
}

/// MR-01/MR-02 fitur tren per indeks bar 1h.
#[derive(Clone, Debug, Default)]
pub struct TrendFeatures {
    pub open_time_1h: Vec<Ms>,
    pub trend: Vec<Trend>,
    pub adx_1h: Vec<f64>,
    pub atr_pctile_1h: Vec<f64>,
    pub close_1h: Vec<f64>,
    pub open_time_15m: Vec<Ms>,
    pub er_15m: Vec<f64>,
}

impl TrendFeatures {
    pub fn compute(h1: &[Bar], m15: &[Bar]) -> TrendFeatures {
        let c = ind::closes(h1);
        let (e21, e50, e200) = (ind::ema(&c, 21), ind::ema(&c, 50), ind::ema(&c, 200));
        let trend = (0..h1.len())
            .map(|i| {
                if i < 200 {
                    return Trend::None;
                }
                let slope = e50[i] - e50[i - 24];
                if c[i] > e50[i] && e21[i] > e50[i] && slope > 0.0 && e50[i] > e200[i] {
                    Trend::Up
                } else if c[i] < e50[i] && e21[i] < e50[i] && slope < 0.0 && e50[i] < e200[i] {
                    Trend::Down
                } else {
                    Trend::None
                }
            })
            .collect();
        let atr = ind::atr(h1, 14);
        let atrp: Vec<f64> = (0..h1.len()).map(|i| atr[i] / c[i]).collect();
        TrendFeatures {
            open_time_1h: h1.iter().map(|b| b.open_time).collect(),
            trend,
            adx_1h: ind::adx(h1, 14),
            atr_pctile_1h: ind::percent_rank(&atrp, 720),
            close_1h: c,
            open_time_15m: m15.iter().map(|b| b.open_time).collect(),
            er_15m: ind::efficiency_ratio(&ind::closes(m15), 20),
        }
    }
    /// Indeks bar terakhir yang tersedia (close ≤ as_of).
    pub fn idx_1h(&self, as_of: Ms) -> Option<usize> {
        last_closed(&self.open_time_1h, Tf::H1, as_of)
    }
    pub fn idx_15m(&self, as_of: Ms) -> Option<usize> {
        last_closed(&self.open_time_15m, Tf::M15, as_of)
    }
}

/// Indeks bar terakhir dengan open_time + tf ≤ as_of (binary search).
pub fn last_closed(open_times: &[Ms], tf: Tf, as_of: Ms) -> Option<usize> {
    let n = open_times.partition_point(|&t| t + tf.ms() <= as_of);
    n.checked_sub(1)
}

/// Perubahan 24 bar 1h (%) pada indeks i.
pub fn chg24(c: &[f64], i: usize) -> Option<f64> {
    if i < 24 || c[i - 24] <= 0.0 {
        None
    } else {
        Some((c[i] / c[i - 24] - 1.0) * 100.0)
    }
}

/// MR-05 Positioning (SHADOW): hanya bisa menurunkan max_pos.
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct Positioning {
    pub funding: f64,
    pub ls_ratio: f64,
    pub oi_chg_1h_pct: f64,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct CrowdFlags {
    pub crowded_long: bool,
    pub crowded_short: bool,
    pub oi_flush: bool,
}

impl Positioning {
    pub fn flags(&self) -> CrowdFlags {
        CrowdFlags {
            crowded_long: self.funding > 0.0005 && self.ls_ratio > 2.0,
            crowded_short: self.funding < -0.0003 && self.ls_ratio < 0.7,
            oi_flush: self.oi_chg_1h_pct < -5.0,
        }
    }
}

/// MR-06 Liquidation: notional per bucket 5m, z-score robust vs median 7 hari (2016 bucket).
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct LiquidationTracker {
    pub buckets: std::collections::VecDeque<(Ms, f64)>,
}

impl LiquidationTracker {
    const KEEP: usize = 2016;
    pub fn add(&mut self, ts: Ms, notional: f64) {
        let b = ts - ts.rem_euclid(300_000);
        match self.buckets.back_mut() {
            Some((t, v)) if *t == b => *v += notional,
            _ => {
                if let Some(&(t, _)) = self.buckets.back() {
                    let mut x = t + 300_000;
                    while x < b {
                        self.buckets.push_back((x, 0.0));
                        x += 300_000;
                    }
                }
                self.buckets.push_back((b, notional));
            }
        }
        while self.buckets.len() > Self::KEEP {
            self.buckets.pop_front();
        }
    }
    /// z = (x − median) / (1,4826·MAD) atas bucket sebelumnya. None bila < 1 hari data.
    pub fn zscore(&self) -> Option<f64> {
        if self.buckets.len() < 288 {
            return None;
        }
        let (last, prev) = self.buckets.iter().map(|x| x.1).collect::<Vec<_>>().split_last().map(|(a, b)| (*a, b.to_vec()))?;
        let med = ind::percentile(&prev, 50.0);
        let dev: Vec<f64> = prev.iter().map(|x| (x - med).abs()).collect();
        let mad = ind::percentile(&dev, 50.0) * 1.4826;
        Some((last - med) / mad.max(1.0))
    }
    pub fn cascade(&self) -> bool {
        self.zscore().is_some_and(|z| z >= 4.0)
    }
}

/// MR-10: |ret 5m BTC| ≥ 1,5% dari close 1m (6 close terakhir).
pub fn shock_from_1m(closes_1m: &[f64], thr_pct: f64) -> bool {
    if closes_1m.len() < 6 {
        return false;
    }
    let n = closes_1m.len();
    (closes_1m[n - 1] / closes_1m[n - 6] - 1.0).abs() * 100.0 >= thr_pct
}

/// Proxy backtest MR-10 (tanpa data 1m): ekskursi bar 15m dari open ≥ ambang. Ditandai proxy di laporan.
pub fn shock_proxy_15m(b: &Bar, thr_pct: f64) -> bool {
    ((b.high / b.open - 1.0).max(1.0 - b.low / b.open)) * 100.0 >= thr_pct
}

/// MR-07 breadth dari daftar (close, ema50, er20) per simbol pada bar 1h terakhir.
pub fn breadth(rows: &[(f64, f64, f64)]) -> Option<(f64, f64)> {
    let v: Vec<_> = rows.iter().filter(|r| r.0.is_finite() && r.1.is_finite() && r.2.is_finite()).collect();
    if v.len() < 5 {
        return None;
    }
    let up = v.iter().filter(|r| r.0 > r.1).count() as f64 / v.len() as f64 * 100.0;
    let ts = v.iter().filter(|r| r.2 >= 0.35).count() as f64 / v.len() as f64 * 100.0;
    Some((up, ts))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn last_closed_respects_available_at() {
        let ot = vec![0, 3_600_000, 7_200_000];
        assert_eq!(last_closed(&ot, Tf::H1, 3_599_999), None);
        assert_eq!(last_closed(&ot, Tf::H1, 3_600_000), Some(0));
        assert_eq!(last_closed(&ot, Tf::H1, 10_800_000), Some(2));
    }

    #[test]
    fn crowding_and_shock() {
        assert!(Positioning { funding: 0.0006, ls_ratio: 2.5, oi_chg_1h_pct: 0.0 }.flags().crowded_long);
        assert!(!Positioning { funding: 0.0006, ls_ratio: 1.5, oi_chg_1h_pct: 0.0 }.flags().crowded_long);
        assert!(Positioning { funding: -0.0004, ls_ratio: 0.6, oi_chg_1h_pct: -6.0 }.flags().crowded_short);
        assert!(shock_from_1m(&[100.0, 100.0, 100.0, 100.0, 100.0, 101.6], 1.5));
        assert!(!shock_from_1m(&[100.0, 100.0, 100.0, 100.0, 100.0, 101.4], 1.5));
    }

    #[test]
    fn liquidation_z() {
        let mut t = LiquidationTracker::default();
        for k in 0..400 {
            t.add(k * 300_000, 1000.0 + (k % 7) as f64 * 100.0);
        }
        assert!(!t.cascade());
        t.add(400 * 300_000, 50_000.0);
        assert!(t.cascade());
    }
}
