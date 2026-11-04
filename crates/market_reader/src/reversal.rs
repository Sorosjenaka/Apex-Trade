//! MR-09 building blocks. This module is not wired into trading gates.
use apex_core::types::Bar;

/// Confirm divergences only after the right-hand pivot window has closed.
pub fn divergence_events(bars: &[Bar], rsi: &[f64], k: usize, max_gap: usize) -> Vec<(usize, i8)> {
    if k == 0 || bars.len() != rsi.len() || bars.len() < 2 * k + 1 {
        return Vec::new();
    }
    let mut highs: Vec<usize> = Vec::new();
    let mut lows: Vec<usize> = Vec::new();
    let mut events = Vec::new();
    for confirmed in 2 * k..bars.len() {
        let i = confirmed - k;
        let window = &bars[i - k..=confirmed];
        let ph = window.iter().all(|b| b.high <= bars[i].high);
        let pl = window.iter().all(|b| b.low >= bars[i].low);
        if ph {
            if let Some(&previous) = highs.last() {
                if i - previous <= max_gap
                    && bars[i].high > bars[previous].high
                    && rsi[i] < rsi[previous]
                {
                    events.push((confirmed, -1));
                }
            }
            highs.push(i);
        }
        if pl {
            if let Some(&previous) = lows.last() {
                if i - previous <= max_gap
                    && bars[i].low < bars[previous].low
                    && rsi[i] > rsi[previous]
                {
                    events.push((confirmed, 1));
                }
            }
            lows.push(i);
        }
    }
    events
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn divergence_requires_confirmed_pivot_and_is_prefix_stable() {
        let lows = [9., 7., 9., 8., 6., 9., 10.];
        let bars: Vec<Bar> = lows
            .iter()
            .enumerate()
            .map(|(i, &lo)| Bar {
                open_time: i as i64,
                open: 10.,
                high: 11.,
                low: lo,
                close: 10.,
                volume: 1.,
                quote_volume: 10.,
                trades: 1,
                taker_buy_base: 0.5,
            })
            .collect();
        let rsi = [50., 20., 50., 50., 30., 50., 50.];
        assert!(divergence_events(&bars[..5], &rsi[..5], 1, 40).is_empty());
        assert_eq!(
            divergence_events(&bars[..6], &rsi[..6], 1, 40),
            vec![(5, 1)]
        );
        assert_eq!(divergence_events(&bars, &rsi, 1, 40), vec![(5, 1)]);
    }
}

/// Parameters and events for one already-closed timeframe.
struct Pack {
    ends: Vec<i64>,
    close: Vec<f64>,
    e21: Vec<f64>,
    e50: Vec<f64>,
    rsi: Vec<f64>,
    ratio: Vec<f64>,
    events: Vec<(usize, i8)>,
    weight: f64,
    decay: f64,
}

fn rsi_source(close: &[f64]) -> Vec<f64> {
    let mut out = vec![50.0; close.len()];
    let mut gain = 0.0;
    let mut loss = 0.0;
    for i in 1..close.len() {
        let d = close[i] - close[i - 1];
        if i == 1 {
            gain = d.max(0.0);
            loss = (-d).max(0.0);
        } else {
            gain = (gain * 13.0 + d.max(0.0)) / 14.0;
            loss = (loss * 13.0 + (-d).max(0.0)) / 14.0;
        }
        if loss > 0.0 {
            out[i] = 100.0 - 100.0 / (1.0 + gain / loss);
        }
    }
    out
}

pub(crate) fn aggregate_hours(bars: &[Bar], hours: usize) -> Vec<Bar> {
    const H: i64 = 3_600_000;
    let width = H * hours as i64;
    let mut out = Vec::new();
    let mut i = 0;
    while i + hours <= bars.len() {
        let start = bars[i].open_time;
        if start.rem_euclid(width) != 0
            || (0..hours).any(|j| bars[i + j].open_time != start + j as i64 * H)
        {
            i += 1;
            continue;
        }
        let group = &bars[i..i + hours];
        let mut b = group[0];
        b.high = group
            .iter()
            .map(|x| x.high)
            .fold(f64::NEG_INFINITY, f64::max);
        b.low = group.iter().map(|x| x.low).fold(f64::INFINITY, f64::min);
        b.close = group[hours - 1].close;
        b.volume = group.iter().map(|x| x.volume).sum();
        b.quote_volume = group.iter().map(|x| x.quote_volume).sum();
        b.trades = group.iter().map(|x| x.trades).sum();
        b.taker_buy_base = group.iter().map(|x| x.taker_buy_base).sum();
        out.push(b);
        i += hours;
    }
    out
}

fn make_pack(
    btc: &[Bar],
    eth: &[Bar],
    hours: usize,
    k: usize,
    weight: f64,
    decay: f64,
) -> Option<Pack> {
    let b = aggregate_hours(btc, hours);
    let e = aggregate_hours(eth, hours);
    if b.len() < 2 * k + 1
        || b.len() != e.len()
        || b.iter()
            .zip(&e)
            .any(|(x, y)| x.open_time != y.open_time || x.close <= 0.0)
    {
        return None;
    }
    let close: Vec<f64> = b.iter().map(|x| x.close).collect();
    let rsi = rsi_source(&close);
    Some(Pack {
        ends: b
            .iter()
            .map(|x| x.open_time + hours as i64 * 3_600_000)
            .collect(),
        e21: apex_indicators::ema(&close, 21),
        e50: apex_indicators::ema(&close, 50),
        ratio: b.iter().zip(e).map(|(x, y)| y.close / x.close).collect(),
        events: divergence_events(&b, &rsi, k, 40),
        close,
        rsi,
        weight,
        decay,
    })
}

/// Shadow-only reversal score at each closed BTC 1h bar. Uses no unfinished HTF candles.
/// Missing or unaligned data returns None; this function never changes any trading gate.
pub fn shadow_scores(btc: &[Bar], eth: &[Bar]) -> Option<Vec<(i64, f64, f64)>> {
    const H: i64 = 3_600_000;
    if btc.len() < 48
        || btc.len() != eth.len()
        || btc
            .iter()
            .zip(eth)
            .any(|(a, b)| a.open_time != b.open_time || !a.is_valid() || !b.is_valid())
        || btc.windows(2).any(|w| w[1].open_time - w[0].open_time != H)
    {
        return None;
    }
    let packs = [
        make_pack(btc, eth, 1, 3, 10., 6.)?,
        make_pack(btc, eth, 2, 3, 20., 12.)?,
        make_pack(btc, eth, 4, 3, 30., 24.)?,
        make_pack(btc, eth, 24, 2, 30., 48.)?,
    ];
    let mut out = Vec::new();
    let mut sm_up = 0.;
    let mut sm_down = 0.;
    let mut init = false;
    for bar in btc {
        let now = bar.open_time + H;
        let ids: Vec<Option<usize>> = packs
            .iter()
            .map(|p| p.ends.partition_point(|&x| x <= now).checked_sub(1))
            .collect();
        let h = &packs[0];
        let Some(ih) = ids[0] else { continue };
        let mut up = 0.;
        let mut down = 0.;
        for (p, idx) in packs.iter().zip(&ids) {
            let Some(idx) = *idx else { continue };
            for &(event, dir) in &p.events {
                if event > idx {
                    break;
                }
                let age = (now - p.ends[event]) as f64 / H as f64;
                let confirmed = if dir > 0 {
                    h.close[ih] > h.e21[ih]
                } else {
                    h.close[ih] < h.e21[ih]
                };
                let w = p.weight * (-age / p.decay).exp() * if confirmed { 1.0 } else { 0.5 };
                if dir > 0 {
                    up += w;
                } else {
                    down += w;
                }
            }
            if p.e21[idx] > p.e50[idx] {
                up += 3.;
            } else if p.e21[idx] < p.e50[idx] {
                down += 3.;
            }
        }
        let p4 = &packs[2];
        if let Some(i4) = ids[2] {
            if let Some(old_idx) = p4
                .ends
                .partition_point(|&x| x <= now - 25 * H)
                .checked_sub(1)
            {
                if i4 >= 24 {
                    let ratio_mom = p4.ratio[i4] - p4.ratio[i4 - 24];
                    let old = p4.close[old_idx];
                    if ratio_mom > 0. && p4.close[i4] <= old {
                        up += 12.;
                    }
                    if ratio_mom < 0. && p4.close[i4] >= old {
                        down += 12.;
                    }
                }
            }
            if p4.rsi[i4] < 32. {
                up += 10.;
            } else if p4.rsi[i4] > 68. {
                down += 10.;
            }
        }
        let pd = &packs[3];
        if let Some(id) = ids[3] {
            if pd.rsi[id] < 30. {
                up += 8.;
            } else if pd.rsi[id] > 70. {
                down += 8.;
            }
        }
        if !init {
            sm_up = up;
            sm_down = down;
            init = true;
        } else {
            sm_up += (up - sm_up) * 2. / 7.;
            sm_down += (down - sm_down) * 2. / 7.;
        }
        out.push((now, sm_up.clamp(0., 100.), sm_down.clamp(0., 100.)));
    }
    Some(out)
}

#[cfg(test)]
mod score_tests {
    use super::*;
    fn bars(n: usize, base: f64) -> Vec<Bar> {
        (0..n)
            .map(|i| {
                let c = base * (1. + (i as f64 / 13.).sin() * 0.03 + i as f64 * 0.00001);
                Bar {
                    open_time: i as i64 * 3_600_000,
                    open: c,
                    high: c * 1.005,
                    low: c * 0.995,
                    close: c,
                    volume: 10.,
                    quote_volume: 10. * c,
                    trades: 10,
                    taker_buy_base: 5.,
                }
            })
            .collect()
    }
    #[test]
    fn score_prefix_and_future_mutation_match() {
        let btc = bars(720, 100.);
        let eth = bars(720, 10.);
        let full = shadow_scores(&btc, &eth).unwrap();
        let prefix = shadow_scores(&btc[..500], &eth[..500]).unwrap();
        assert_eq!(&full[..prefix.len()], &prefix);
        let mut future = btc.clone();
        for b in future.iter_mut().skip(500) {
            b.open *= 1.1;
            b.high *= 1.1;
            b.low *= 1.1;
            b.close *= 1.1;
        }
        let changed = shadow_scores(&future, &eth).unwrap();
        assert_eq!(&changed[..prefix.len()], &prefix);
        assert!(prefix
            .iter()
            .all(|(_, up, dn)| (0. ..=100.).contains(up) && (0. ..=100.).contains(dn)));
    }
    #[test]
    fn missing_bars_fail_closed() {
        let btc = bars(300, 100.);
        let mut eth = bars(300, 10.);
        eth.remove(100);
        assert!(shadow_scores(&btc, &eth).is_none());
    }
}
