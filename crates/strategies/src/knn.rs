//! Causal kNN grader: labels are eligible only strictly after trade closure.
use serde::{Deserialize, Serialize};

pub const K: usize = 15;
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct LabeledSample {
    pub features: [f64; 5],
    pub signal_ms: i64,
    pub closed_ms: i64,
    pub win: bool,
}
#[derive(Clone, Debug, PartialEq)]
pub struct Grade {
    pub neighbors: usize,
    pub winners: usize,
    pub pass: bool,
}

fn valid(features: &[f64; 5]) -> bool {
    features
        .iter()
        .all(|x| x.is_finite() && (0.0..=100.0).contains(x))
}

/// None is an explicit unavailable grade, never implicit approval.
pub fn grade(samples: &[LabeledSample], features: [f64; 5], as_of: i64) -> Option<Grade> {
    if !valid(&features) {
        return None;
    }
    let mut eligible: Vec<(f64, usize, &LabeledSample)> = samples
        .iter()
        .enumerate()
        .filter(|(_, x)| x.signal_ms < x.closed_ms && x.closed_ms < as_of && valid(&x.features))
        .map(|(i, x)| {
            (
                x.features
                    .iter()
                    .zip(features)
                    .map(|(a, b)| (a - b) * (a - b))
                    .sum::<f64>(),
                i,
                x,
            )
        })
        .collect();
    if eligible.len() < K {
        return None;
    }
    eligible.sort_by(|a, b| {
        a.0.total_cmp(&b.0)
            .then(a.2.closed_ms.cmp(&b.2.closed_ms))
            .then(a.1.cmp(&b.1))
    });
    let winners = eligible.iter().take(K).filter(|x| x.2.win).count();
    Some(Grade {
        neighbors: K,
        winners,
        pass: winners * 100 >= K * 55,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn excludes_unfinished_and_equal_timestamp_labels() {
        let mut v: Vec<LabeledSample> = (0..15)
            .map(|i| LabeledSample {
                features: [i as f64; 5],
                signal_ms: i,
                closed_ms: i + 1,
                win: i < 9,
            })
            .collect();
        let before = grade(&v, [0.; 5], 20).unwrap();
        assert_eq!(
            (before.neighbors, before.winners, before.pass),
            (15, 9, true)
        );
        v.push(LabeledSample {
            features: [0.; 5],
            signal_ms: 19,
            closed_ms: 20,
            win: false,
        });
        assert_eq!(grade(&v, [0.; 5], 20), Some(before));
    }
    #[test]
    fn threshold_is_nine_wins_out_of_fifteen() {
        let mut v: Vec<LabeledSample> = (0..15)
            .map(|i| LabeledSample {
                features: [1.; 5],
                signal_ms: i,
                closed_ms: i + 1,
                win: i < 8,
            })
            .collect();
        assert!(!grade(&v, [1.; 5], 100).unwrap().pass);
        v[8].win = true;
        assert!(grade(&v, [1.; 5], 100).unwrap().pass);
    }
    #[test]
    fn fails_closed_without_fifteen_valid_closed_labels() {
        let v: Vec<LabeledSample> = (0..14)
            .map(|i| LabeledSample {
                features: [1.; 5],
                signal_ms: i,
                closed_ms: i + 1,
                win: true,
            })
            .collect();
        assert!(grade(&v, [1.; 5], 100).is_none());
        assert!(grade(&v, [f64::NAN; 5], 100).is_none());
    }
}

/// Five 15m percent ranks against the preceding 300 closed bars.
/// Call with the side-specific series; short prices/delta are already mirrored.
pub fn features_at(series: &crate::SideSeries, t: usize) -> Option<[f64; 5]> {
    const WINDOW: usize = 300;
    const ER: usize = 20;
    if t < WINDOW + ER || t >= series.m15.len() {
        return None;
    }
    let start = t - WINDOW - ER;
    let close: Vec<f64> = series.m15[start..=t].iter().map(|b| b.close).collect();
    let er = apex_indicators::efficiency_ratio(&close, ER);
    let f = &series.f15;
    let mut vectors = Vec::with_capacity(WINDOW + 1);
    for i in t - WINDOW..=t {
        let bar = &series.m15[i];
        let prev_volume = f.vol_sma20[i - 1];
        if !bar.close.is_finite()
            || bar.close <= 0.0
            || !bar.volume.is_finite()
            || bar.volume < 0.0
            || !prev_volume.is_finite()
            || prev_volume <= 0.0
        {
            return None;
        }
        let x = [
            er[i - start],
            f.srv[i],
            f.rsi[i],
            f.atr[i] / bar.close * 100.0,
            bar.volume / prev_volume,
        ];
        if x.iter().any(|v| !v.is_finite()) {
            return None;
        }
        vectors.push(x);
    }
    let current = vectors[WINDOW];
    let mut ranks = [0.0; 5];
    for j in 0..5 {
        ranks[j] = vectors[..WINDOW]
            .iter()
            .filter(|x| x[j] <= current[j])
            .count() as f64
            * 100.0
            / WINDOW as f64;
    }
    Some(ranks)
}

#[cfg(test)]
mod feature_tests {
    use super::*;
    use apex_core::types::{Bar, Tf};
    fn bars(n: usize) -> Vec<Bar> {
        (0..n)
            .map(|i| {
                let p = 100.0 + 0.02 * i as f64 + (i as f64 / 7.0).sin();
                let vol = 100.0 + (i % 31) as f64;
                Bar {
                    open_time: i as i64 * Tf::M15.ms(),
                    open: p,
                    high: p + 0.5,
                    low: p - 0.5,
                    close: p,
                    volume: vol,
                    quote_volume: vol * p,
                    trades: 100,
                    taker_buy_base: vol / 2.0,
                }
            })
            .collect()
    }
    #[test]
    fn feature_rank_uses_only_closed_prefix() {
        let m = bars(500);
        let h = apex_indicators::aggregate(&m, Tf::M15, Tf::H1);
        let full = crate::SymbolSeries::new("X", &m, &h);
        let expected = features_at(&full.long, 400).unwrap();
        let prefix = crate::SymbolSeries::new("X", &m[..=400], &h[..100]);
        assert_eq!(features_at(&prefix.long, 400), Some(expected));
        let mut changed = m.clone();
        for b in changed.iter_mut().skip(401) {
            b.open *= 1.2;
            b.high *= 1.2;
            b.low *= 1.2;
            b.close *= 1.2;
            b.volume *= 5.0;
        }
        let hc = apex_indicators::aggregate(&changed, Tf::M15, Tf::H1);
        let mutated = crate::SymbolSeries::new("X", &changed, &hc);
        assert_eq!(features_at(&mutated.long, 400), Some(expected));
    }
}
