//! Closed-bar stage and Fibonacci context adapted from market_reader_v3_tested.py.
//! Analysis only: never used for order or risk gating.
use apex_core::types::Bar;
use serde::Serialize;

#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct Context {
    pub available_at: i64,
    pub evaluated_at: i64,
    pub stage: String,
    pub leg_direction: String,
    pub leg_start: f64,
    pub leg_end: f64,
    pub fib_618: f64,
    pub fib_786: f64,
    pub in_core: bool,
    pub in_expanded: bool,
    pub origin_breached: bool,
    pub regular_bull_strength: f64,
    pub regular_bear_strength: f64,
    pub hidden_bull_strength: f64,
    pub hidden_bear_strength: f64,
    pub structure_break_up: bool,
    pub structure_break_down: bool,
}

#[derive(Clone, Copy)]
struct Pivot {
    index: usize,
    confirm: usize,
    price: f64,
    high: bool,
}
#[derive(Clone, Copy)]
struct Event {
    confirm: usize,
    price: f64,
    kind: usize,
}

fn pivots(bars: &[Bar], k: usize) -> Vec<Pivot> {
    let mut result = Vec::new();
    for confirm in 2 * k..bars.len() {
        let i = confirm - k;
        let high = bars[i].high;
        let low = bars[i].low;
        if bars[i - k..i].iter().all(|b| high >= b.high)
            && bars[i + 1..=confirm].iter().all(|b| high > b.high)
        {
            result.push(Pivot {
                index: i,
                confirm,
                price: high,
                high: true,
            });
        }
        if bars[i - k..i].iter().all(|b| low <= b.low)
            && bars[i + 1..=confirm].iter().all(|b| low < b.low)
        {
            result.push(Pivot {
                index: i,
                confirm,
                price: low,
                high: false,
            });
        }
    }
    result
}

// 0 regular bull, 1 regular bear, 2 hidden bull, 3 hidden bear.
fn events(piv: &[Pivot], rsi: &[f64], hidden: bool) -> Vec<Event> {
    let mut prev = [None::<Pivot>; 2];
    let mut out = Vec::new();
    for &p in piv {
        let side = usize::from(p.high);
        let old = prev[side];
        prev[side] = Some(p);
        let Some(old) = old else { continue };
        if hidden && p.index - old.index > 40 {
            continue;
        }
        let (a, b) = (rsi[old.index], rsi[p.index]);
        if !a.is_finite() || !b.is_finite() {
            continue;
        }
        let dp = p.price - old.price;
        let dr = b - a;
        let kind = match (p.high, hidden) {
            (false, false) if dp < 0. && dr > 0. => Some(0),
            (true, false) if dp > 0. && dr < 0. => Some(1),
            (false, true) if dp > 0. && dr < 0. => Some(2),
            (true, true) if dp < 0. && dr > 0. => Some(3),
            _ => None,
        };
        if let Some(kind) = kind {
            out.push(Event {
                confirm: p.confirm,
                price: p.price,
                kind,
            });
        }
    }
    out
}

/// `bars` must be ordered and contiguous. Only observations closed by `as_of` are read.
/// Returns None during 200-bar warmup, stale/malformed data, or missing confirmed leg.
pub fn closed_context(bars: &[Bar], hours: i64, as_of: i64) -> Option<Context> {
    let half_life = match hours {
        1 => 6.0,
        2 => 12.0,
        4 => 24.0,
        24 => 48.0,
        _ => return None,
    };
    let width = hours.checked_mul(3_600_000)?;
    let n = bars.partition_point(|b| b.open_time.checked_add(width).is_some_and(|t| t <= as_of));
    let bars = &bars[..n];
    if bars
        .last()
        .and_then(|b| b.open_time.checked_add(width))
        .is_none_or(|last| as_of - last >= width)
    {
        return None;
    }
    if bars.len() < 200
        || bars.iter().any(|b| !b.is_valid())
        || bars
            .windows(2)
            .any(|w| w[1].open_time - w[0].open_time != width)
    {
        return None;
    }
    let closes: Vec<f64> = bars.iter().map(|b| b.close).collect();
    let rsi = apex_indicators::rsi_wilder(&closes, 14);
    if !rsi[n - 1].is_finite() {
        return None;
    }
    let p5 = pivots(bars, 5);
    let p3 = pivots(bars, 3);
    let mut ev = events(&p5, &rsi, false);
    ev.extend(events(&p3, &rsi, true));
    ev.sort_by_key(|e| (e.confirm, e.kind));
    let mut last_h = None::<Pivot>;
    let mut last_l = None::<Pivot>;
    let mut dir = 0i8;
    let mut start = 0.0;
    let mut end = 0.0;
    let mut event_cursor = 0usize;
    let mut pivot_cursor = 0usize;
    let mut latest = [None::<(usize, f64, bool)>; 4];
    for (i, bar) in bars.iter().enumerate() {
        let begin = pivot_cursor;
        while pivot_cursor < p5.len() && p5[pivot_cursor].confirm == i {
            let p = p5[pivot_cursor];
            if p.high {
                last_h = Some(p)
            } else {
                last_l = Some(p)
            };
            pivot_cursor += 1;
        }
        if dir == 0 {
            if let (Some(h), Some(l)) = (last_h, last_l) {
                if h.index > l.index {
                    dir = 1;
                    start = l.price;
                    end = h.price
                } else {
                    dir = -1;
                    start = h.price;
                    end = l.price
                }
            }
        }
        for p in &p5[begin..pivot_cursor] {
            if p.high {
                if dir == 1 && p.price > end {
                    end = p.price
                } else if dir == -1 && p.price > start {
                    dir = 1;
                    start = end;
                    end = p.price
                }
            } else if dir == -1 && p.price < end {
                end = p.price
            } else if dir == 1 && p.price < start {
                dir = -1;
                start = end;
                end = p.price
            }
        }
        let mut fresh = [false; 4];
        while event_cursor < ev.len() && ev[event_cursor].confirm == i {
            let e = ev[event_cursor];
            latest[e.kind] = Some((i, e.price, false));
            fresh[e.kind] = true;
            event_cursor += 1;
        }
        for kind in 0..4 {
            if let Some((_, price, invalid)) = &mut latest[kind] {
                if !fresh[kind]
                    && ((kind == 0 || kind == 2) && bar.close < *price
                        || (kind == 1 || kind == 3) && bar.close > *price)
                {
                    *invalid = true;
                }
            }
        }
    }
    if dir == 0 {
        return None;
    }
    let mut strength = [0.0; 4];
    for (kind, item) in latest.iter().enumerate() {
        if let Some((i, _, invalid)) = item {
            let event_close = bars[*i].open_time + width;
            let age = (as_of - event_close) as f64 / 3_600_000.0;
            if !invalid && age >= 0. && age <= 4.0 * half_life {
                strength[kind] = (-std::f64::consts::LN_2 * age / half_life).exp();
            }
        }
    }
    let up = strength[0] >= 0.25;
    let down = strength[1] >= 0.25;
    let close = bars[n - 1].close;
    let stage = if up && down {
        "mixed_reversal_evidence"
    } else if up {
        if last_h.is_some_and(|p| close > p.price) {
            "candidate_reversal_up_with_structure_break_present"
        } else {
            "candidate_reversal_up"
        }
    } else if down {
        if last_l.is_some_and(|p| close < p.price) {
            "candidate_reversal_down_with_structure_break_present"
        } else {
            "candidate_reversal_down"
        }
    } else if strength[2] >= 0.25 && strength[3] >= 0.25 {
        "mixed_continuation_evidence"
    } else if strength[2] >= 0.25 {
        "candidate_continuation_up"
    } else if strength[3] >= 0.25 {
        "candidate_continuation_down"
    } else {
        "no_active_reversal"
    };
    let a = end + (start - end) * 0.618;
    let b = end + (start - end) * 0.786;
    let lo = a.min(b);
    let hi = a.max(b);
    let tolerance = close * 0.015;
    Some(Context {
        available_at: bars[n - 1].open_time + width,
        evaluated_at: as_of,
        stage: stage.into(),
        leg_direction: if dir == 1 { "up" } else { "down" }.into(),
        leg_start: start,
        leg_end: end,
        fib_618: a,
        fib_786: b,
        in_core: close >= lo && close <= hi,
        in_expanded: close >= lo - tolerance && close <= hi + tolerance,
        origin_breached: if dir == 1 {
            close < start
        } else {
            close > start
        },
        regular_bull_strength: strength[0],
        regular_bear_strength: strength[1],
        hidden_bull_strength: strength[2],
        hidden_bear_strength: strength[3],
        structure_break_up: last_h.is_some_and(|p| close > p.price),
        structure_break_down: last_l.is_some_and(|p| close < p.price),
    })
}

/// Aggregate complete UTC-aligned 1h candles before calculating 1h/2h/4h context.
pub fn hourly_context(bars: &[Bar], hours: usize, as_of: i64) -> Option<Context> {
    if !matches!(hours, 1 | 2 | 4) {
        return None;
    }
    let aggregated = super::reversal::aggregate_hours(bars, hours);
    closed_context(&aggregated, hours as i64, as_of)
}

#[cfg(test)]
mod tests {
    use super::*;
    const H: i64 = 3_600_000;
    fn bars(n: usize) -> Vec<Bar> {
        (0..n)
            .map(|i| {
                let p = 100.0
                    + 0.03 * i as f64
                    + 4.0 * (i as f64 / 5.0).sin()
                    + 2.0 * (i as f64 / 17.0).sin();
                Bar {
                    open_time: i as i64 * H,
                    open: p,
                    high: p + 1.,
                    low: p - 1.,
                    close: p,
                    volume: 10.,
                    quote_volume: 10. * p,
                    trades: 10,
                    taker_buy_base: 5.,
                }
            })
            .collect()
    }
    #[test]
    fn golden_context_matches_tested_python_fixture() {
        let x = closed_context(&bars(320), 1, 320 * H).unwrap();
        assert_eq!(x.stage, "no_active_reversal");
        assert_eq!(x.leg_direction, "up");
        assert!((x.leg_start - 95.71122108638431).abs() < 1e-10);
        assert!((x.leg_end - 113.67789678643871).abs() < 1e-10);
        assert!((x.fib_618 - 102.5744912038051).abs() < 1e-10);
        assert!((x.fib_786 - 99.55608968619595).abs() < 1e-10);
        assert!(!x.in_core && !x.in_expanded);
    }
    #[test]
    fn stage_matches_python_event_and_structure_cases() {
        let data = bars(800);
        for (i, stage) in [
            (199, "no_active_reversal"),
            (200, "candidate_continuation_down"),
            (264, "candidate_reversal_down"),
            (272, "candidate_reversal_down_with_structure_break_present"),
            (467, "candidate_continuation_up"),
            (625, "candidate_reversal_up"),
            (634, "candidate_reversal_up_with_structure_break_present"),
        ] {
            assert_eq!(
                closed_context(&data, 1, (i + 1) as i64 * H).unwrap().stage,
                stage,
                "bar {i}"
            );
        }
        let down = closed_context(&data, 1, 273 * H).unwrap();
        assert!((down.regular_bear_strength - 0.396850262992).abs() < 1e-10);
        assert!(down.structure_break_down);
        let up = closed_context(&data, 1, 635 * H).unwrap();
        assert!((up.regular_bull_strength - 0.353553390593).abs() < 1e-10);
        assert!(up.structure_break_up);
    }
    #[test]
    fn higher_timeframes_wait_for_their_own_close() {
        let data = bars(1200);
        for hours in [2usize, 4] {
            let as_of = 901 * H;
            let context = hourly_context(&data, hours, as_of).unwrap();
            assert!(context.available_at <= as_of);
            assert_eq!(hourly_context(&data[..901], hours, as_of), Some(context));
        }
        let mut daily = bars(220);
        for (i, bar) in daily.iter_mut().enumerate() {
            bar.open_time = i as i64 * 24 * H;
        }
        let now = 215 * 24 * H + H;
        let result = closed_context(&daily, 24, now).unwrap();
        assert_eq!(result.available_at, 215 * 24 * H);
        assert_eq!(closed_context(&daily[..215], 24, now), Some(result));
    }

    #[test]
    fn incomplete_future_and_missing_bars_are_never_used() {
        let b = bars(320);
        let prefix = closed_context(&b[..=250], 1, 251 * H).unwrap();
        assert_eq!(closed_context(&b, 1, 251 * H), Some(prefix.clone()));
        let mut changed = b.clone();
        for x in changed.iter_mut().skip(251) {
            x.open *= 1.4;
            x.high *= 1.4;
            x.low *= 1.4;
            x.close *= 1.4;
        }
        assert_eq!(closed_context(&changed, 1, 251 * H), Some(prefix));
        let mut gap = b;
        gap.remove(210);
        assert!(closed_context(&gap, 1, 320 * H).is_none());
        assert!(closed_context(&bars(199), 1, 199 * H).is_none());
        assert!(closed_context(&bars(220), 1, 221 * H).is_none());
    }
}
