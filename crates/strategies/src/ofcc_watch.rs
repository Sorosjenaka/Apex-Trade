//! Closed-bar S1 order-flow confirmation. A watch is not an exchange order.
use crate::{mirror_bar, Signal, StrategyId};
use apex_core::types::{Bar, Side, Tf};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Outcome {
    Waiting,
    Confirmed,
    Rejected,
    Expired,
}

impl Outcome {
    pub fn as_str(&self) -> &'static str {
        match self {
            Outcome::Waiting => "waiting",
            Outcome::Confirmed => "confirmed",
            Outcome::Rejected => "rejected",
            Outcome::Expired => "expired",
        }
    }
}

/// One FVG setup. The existing order is only submitted after `Confirmed`;
/// this avoids using the touching bar's final delta to pretend an earlier fill was filtered.
#[derive(Clone, Debug)]
pub struct S1Watch {
    pub signal: Signal,
    zone_edge: f64,
    seen: u32,
    last_close_ms: i64,
    finished: bool,
    timeframe: Tf,
}
impl S1Watch {
    pub fn new(signal: Signal) -> Option<Self> {
        Self::new_with_tf(signal, Tf::M15)
    }
    pub fn new_with_tf(signal: Signal, timeframe: Tf) -> Option<Self> {
        if !matches!(timeframe, Tf::M5 | Tf::M15) {
            return None;
        }
        if signal.strategy != StrategyId::S1Tfp
            || signal.kind != apex_core::sizing::EntryKind::LimitMaker
        {
            return None;
        }
        let boundary = signal.cancel_beyond?;
        if !(signal.entry.is_finite() && boundary.is_finite() && signal.entry > 0. && boundary > 0.)
        {
            return None;
        }
        if (signal.side == Side::Long && boundary >= signal.entry)
            || (signal.side == Side::Short && boundary <= signal.entry)
        {
            return None;
        }
        let edge = match signal.side {
            Side::Long => 2. * signal.entry - boundary,
            Side::Short => {
                let v = 2. / signal.entry - 1. / boundary;
                if v <= 0. {
                    return None;
                }
                1. / v
            }
        };
        if !edge.is_finite() || edge <= 0. {
            return None;
        }
        let last_close_ms = signal.bar_close_ms;
        Some(Self {
            signal,
            zone_edge: edge,
            seen: 0,
            last_close_ms,
            finished: false,
            timeframe,
        })
    }
    /// Number of closed bars observed since setup.
    pub fn seen(&self) -> u32 {
        self.seen
    }
    /// Original validity window remaining after observed closed bars.
    pub fn remaining_bars(&self) -> u32 {
        self.signal.ttl_bars.saturating_sub(self.seen)
    }

    /// Bar MUST be closed at `close_ms` and later than setup. Returns terminal outcomes once.
    pub fn observe(&mut self, bar: &Bar, close_ms: i64, p90_abs_delta: f64) -> Outcome {
        if self.finished {
            return Outcome::Expired;
        }
        if close_ms <= self.last_close_ms {
            return Outcome::Waiting;
        }
        let tf_ms = self.timeframe.ms();
        if close_ms != self.last_close_ms + tf_ms
            || bar.open_time + tf_ms != close_ms
            || !bar.is_valid()
            || !bar.volume.is_finite()
            || !bar.taker_buy_base.is_finite()
            || bar.volume <= 0.0
            || bar.taker_buy_base < 0.0
            || bar.taker_buy_base > bar.volume
        {
            self.finished = true;
            return Outcome::Rejected;
        }
        self.last_close_ms = close_ms;
        self.seen += 1;
        if self.seen > self.signal.ttl_bars {
            self.finished = true;
            return Outcome::Expired;
        }
        let boundary = self.signal.cancel_beyond.unwrap();
        let invalid = match self.signal.side {
            Side::Long => bar.close < boundary,
            Side::Short => bar.close > boundary,
        };
        if invalid {
            self.finished = true;
            return Outcome::Rejected;
        }
        let touched = match self.signal.side {
            Side::Long => bar.low <= self.zone_edge,
            Side::Short => bar.high >= self.zone_edge,
        };
        if !touched {
            return Outcome::Waiting;
        }
        self.finished = true;
        let b = if self.signal.side == Side::Short {
            mirror_bar(bar)
        } else {
            *bar
        };
        if !p90_abs_delta.is_finite()
            || p90_abs_delta <= 0.
            || (b.delta() <= -p90_abs_delta && b.close_pos() <= 0.25)
        {
            Outcome::Rejected
        } else {
            Outcome::Confirmed
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use apex_core::sizing::EntryKind;
    fn sig(side: Side) -> Signal {
        Signal {
            strategy: StrategyId::S1Tfp,
            symbol: "ETHUSDT".into(),
            side,
            entry: 100.,
            sl: if side == Side::Long { 97. } else { 103. },
            kind: EntryKind::LimitMaker,
            ttl_bars: 2,
            bar_close_ms: 0,
            cancel_beyond: Some(if side == Side::Long { 98. } else { 102. }),
            note: String::new(),
        }
    }
    fn bar(open_time: i64, low: f64, high: f64, close: f64, buy: f64) -> Bar {
        Bar {
            open_time,
            open: 100.,
            high,
            low,
            close,
            volume: 100.,
            quote_volume: 10000.,
            trades: 100,
            taker_buy_base: buy,
        }
    }
    #[test]
    fn waits_for_close_and_rejects_sell_pressure() {
        let mut w = S1Watch::new(sig(Side::Long)).unwrap();
        let b = bar(0, 99., 101., 99.1, 30.);
        assert_eq!(w.observe(&b, 0, 10.), Outcome::Waiting);
        assert_eq!(w.observe(&b, 900_000, 10.), Outcome::Rejected);
    }
    #[test]
    fn confirms_only_after_touch_and_rejects_gap() {
        let mut w = S1Watch::new(sig(Side::Long)).unwrap();
        let far = bar(0, 102.5, 104., 103., 50.);
        let mut far = far;
        far.open = 103.;
        assert_eq!(w.observe(&far, 900_000, 10.), Outcome::Waiting);
        assert_eq!(w.observe(&far, 900_000, 10.), Outcome::Waiting);
        let touching = bar(900_000, 99., 103., 101., 60.);
        assert_eq!(w.observe(&touching, 1_800_000, 10.), Outcome::Confirmed);
        let mut w = S1Watch::new(sig(Side::Long)).unwrap();
        assert_eq!(w.observe(&touching, 1_800_000, 10.), Outcome::Rejected);
    }
    #[test]
    fn short_detects_opposing_buy_pressure() {
        let mut w = S1Watch::new(sig(Side::Short)).unwrap();
        let b = bar(0, 99., 101., 100.9, 70.);
        assert_eq!(w.observe(&b, 900_000, 10.), Outcome::Rejected);
    }
    #[test]
    fn invalid_flow_never_confirms() {
        let mut w = S1Watch::new(sig(Side::Long)).unwrap();
        let mut b = bar(0, 99., 103., 101., 60.);
        b.taker_buy_base = f64::NAN;
        assert_eq!(w.observe(&b, 900_000, 10.), Outcome::Rejected);
    }

    #[test]
    fn five_minute_watch_requires_a_closed_five_minute_bar() {
        let mut w = S1Watch::new_with_tf(sig(Side::Long), Tf::M5).unwrap();
        let b = bar(0, 99., 103., 101., 60.);
        assert_eq!(w.observe(&b, 0, 10.), Outcome::Waiting);
        assert_eq!(w.observe(&b, 300_000, 10.), Outcome::Confirmed);
        assert!(S1Watch::new_with_tf(sig(Side::Long), Tf::H1).is_none());
    }

    #[test]
    fn five_minute_view_never_sees_unclosed_hour() {
        let bars: Vec<_> = (0..13).map(|i| bar(i * Tf::M5.ms(), 99., 103., 101., 60.)).collect();
        let hours = vec![bar(0, 99., 103., 101., 60.), bar(Tf::H1.ms(), 99., 103., 101., 60.)];
        let series = crate::SymbolSeries::new_with_tf("ETHUSDT", &bars, &hours, Tf::M5);
        assert_eq!(series.h1_idx_at(10), None);
        assert_eq!(series.h1_idx_at(11), Some(0));
        let view = crate::make_view(&series, &series.long, 10, apex_screener::SymState::TrendUp);
        assert!(view.h1.is_empty());
        assert_eq!(view.as_of, 11 * Tf::M5.ms());
    }
}
