//! TV-04..TV-07: pemilihan sumber dominance/total (TradingView primer, Binance cadangan).
use apex_core::types::Ms;
use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum DomSource {
    Tv,
    Fallback,
    Unavailable,
}

/// Nilai scanner TV terakhir (CRYPTOCAP:*).
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct TvSnapshot {
    pub fetched_at: Ms,
    pub btc_d: Option<f64>,
    pub usdt_d: Option<f64>,
    pub total: Option<f64>,
    pub total2: Option<f64>,
    pub total3: Option<f64>,
    pub btc_d_chg24: Option<f64>,
    pub total_chg24: Option<f64>,
    pub total3_chg24: Option<f64>,
    pub usdt_d_chg24: Option<f64>,
}

impl TvSnapshot {
    /// TV-05 sanity.
    pub fn sane(&self) -> Result<(), String> {
        let (d, u, t, t2, t3) = (self.btc_d.ok_or("btc_d")?, self.usdt_d.ok_or("usdt_d")?, self.total.ok_or("total")?, self.total2.ok_or("total2")?, self.total3.ok_or("total3")?);
        if !(30.0..=80.0).contains(&d) {
            return Err(format!("BTC.D {d} di luar [30,80]"));
        }
        if !(1.0..=20.0).contains(&u) {
            return Err(format!("USDT.D {u} di luar [1,20]"));
        }
        if !(0.5e12..=10e12).contains(&t) {
            return Err(format!("TOTAL {t} di luar [$0,5T,$10T]"));
        }
        if !(t3 < t2 && t2 < t) {
            return Err("urutan TOTAL3 < TOTAL2 < TOTAL gagal".into());
        }
        Ok(())
    }
    /// TV-04: scanner stale bila umur > 180 s.
    pub fn fresh(&self, now: Ms) -> bool {
        now - self.fetched_at <= 180_000
    }
}

/// TV-06/TV-07: TV OK → TV; TV gagal → cadangan; kembali ke TV hanya setelah 3 poll OK beruntun.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct SourceSelector {
    pub current: DomSource,
    pub ok_streak: u32,
}

impl Default for SourceSelector {
    fn default() -> Self {
        SourceSelector { current: DomSource::Tv, ok_streak: 3 }
    }
}

impl SourceSelector {
    pub fn update(&mut self, tv_ok: bool, fallback_ok: bool) -> DomSource {
        if tv_ok {
            self.ok_streak += 1;
        } else {
            self.ok_streak = 0;
        }
        self.current = if tv_ok && (self.current == DomSource::Tv || self.ok_streak >= 3) {
            DomSource::Tv
        } else if fallback_ok {
            DomSource::Fallback
        } else if tv_ok {
            DomSource::Tv
        } else {
            DomSource::Unavailable
        };
        self.current
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tv_sanity() {
        let mut s = TvSnapshot { fetched_at: 0, btc_d: Some(58.0), usdt_d: Some(5.0), total: Some(3.5e12), total2: Some(1.5e12), total3: Some(1.0e12), ..Default::default() };
        assert!(s.sane().is_ok());
        s.btc_d = Some(85.0);
        assert!(s.sane().is_err());
        s.btc_d = Some(58.0);
        s.total3 = Some(2.0e12);
        assert!(s.sane().is_err());
        assert!(s.fresh(180_000) && !s.fresh(180_001));
    }

    #[test]
    fn fallback_and_return_after_three() {
        let mut s = SourceSelector::default();
        assert_eq!(s.update(true, true), DomSource::Tv);
        assert_eq!(s.update(false, true), DomSource::Fallback);
        assert_eq!(s.update(true, true), DomSource::Fallback);
        assert_eq!(s.update(true, true), DomSource::Fallback);
        assert_eq!(s.update(true, true), DomSource::Tv);
        assert_eq!(s.update(false, false), DomSource::Unavailable);
    }
}
