//! L3 regime per simbol (SCREENER.md RC-1..RC-9) + histeresis 2 bar 1h untuk masuk TREND, 1 bar untuk keluar.
//! Fitur dihitung kausal sekali per seri (`SymbolFeatures::compute`), keputusan dibaca di indeks bar 1h tertutup.
use apex_core::types::{Bar, Ms, Tf};
use apex_indicators as ind;
use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum SymState {
    TrendUp,
    TrendDown,
    Ranging,
    Blocked,
}

impl SymState {
    pub fn as_str(&self) -> &'static str {
        match self {
            SymState::TrendUp => "TREND_UP",
            SymState::TrendDown => "TREND_DOWN",
            SymState::Ranging => "RANGING",
            SymState::Blocked => "BLOCKED",
        }
    }
    pub fn dir(&self) -> i8 {
        match self {
            SymState::TrendUp => 1,
            SymState::TrendDown => -1,
            _ => 0,
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(default)]
pub struct ScreenerConfig {
    pub adx_trend: f64,
    pub adx_strong: f64,
    pub adx_range: f64,
    pub er_trend: f64,
    pub er_range: f64,
    pub chop_trend: f64,
    pub chop_range: f64,
    pub srv_trend: f64,
    pub srv_range: f64,
    pub atr_ratio_trend: f64,
    pub atr_ratio_range: f64,
    pub atr_pct_min: f64,
    pub atr_pct_max: f64,
    pub min_votes: i32,
    pub min_range_flags: usize,
    pub enter_bars: u32,
    pub atr_median_bars: usize,
}

impl Default for ScreenerConfig {
    fn default() -> Self {
        ScreenerConfig {
            adx_trend: 25.0,
            adx_strong: 30.0,
            adx_range: 20.0,
            er_trend: 0.35,
            er_range: 0.25,
            chop_trend: 45.0,
            chop_range: 55.0,
            srv_trend: 0.35,
            srv_range: 0.20,
            atr_ratio_trend: 1.0,
            atr_ratio_range: 0.7,
            atr_pct_min: 0.6,
            atr_pct_max: 3.0,
            min_votes: 3,
            min_range_flags: 2,
            enter_bars: 2,
            atr_median_bars: 720,
        }
    }
}

/// Seri fitur 1h untuk satu simbol (indeks = indeks bar 1h).
#[derive(Clone, Debug, Default)]
pub struct SymbolFeatures {
    pub open_time: Vec<Ms>,
    pub close: Vec<f64>,
    pub adx: Vec<f64>,
    pub er: Vec<f64>,
    pub chop: Vec<f64>,
    pub srv: Vec<f64>,
    pub bft: Vec<i8>,
    pub dec: Vec<i8>,
    pub atr_pct: Vec<f64>,
    pub atr_ratio: Vec<f64>,
    pub mtf_1h: Vec<i8>,
    /// SRV & MTF 4h dari bar 4h TERAKHIR yang sudah tutup pada close bar 1h ini.
    pub srv_4h: Vec<f64>,
    pub mtf_4h: Vec<i8>,
}

fn mtf_series(c: &[f64]) -> Vec<i8> {
    let e21 = ind::ema(c, 21);
    let e55 = ind::ema(c, 55);
    (0..c.len())
        .map(|i| {
            if i < 55 {
                0
            } else if e21[i] > e55[i] && c[i] > e21[i] {
                1
            } else if e21[i] < e55[i] && c[i] < e21[i] {
                -1
            } else {
                0
            }
        })
        .collect()
}

impl SymbolFeatures {
    pub fn compute(h1: &[Bar], cfg: &ScreenerConfig) -> SymbolFeatures {
        let c = ind::closes(h1);
        let atr = ind::atr(h1, 14);
        let atr_pct: Vec<f64> = (0..h1.len()).map(|i| atr[i] / c[i] * 100.0).collect();
        let mut atr_ratio = vec![f64::NAN; h1.len()];
        let m = cfg.atr_median_bars;
        for i in m..h1.len() {
            let med = ind::percentile(&atr_pct[i + 1 - m..=i], 50.0);
            if med > 0.0 {
                atr_ratio[i] = atr_pct[i] / med;
            }
        }
        let h4 = ind::aggregate(h1, Tf::H1, Tf::H4);
        let c4 = ind::closes(&h4);
        let (srv4, _) = ind::srv(&c4, 30, 3);
        let mtf4 = mtf_series(&c4);
        let mut srv_4h = vec![f64::NAN; h1.len()];
        let mut mtf_4h = vec![0i8; h1.len()];
        let mut k = 0usize;
        let mut last: Option<usize> = None;
        for i in 0..h1.len() {
            let close_t = h1[i].open_time + Tf::H1.ms();
            while k < h4.len() && h4[k].open_time + Tf::H4.ms() <= close_t {
                last = Some(k);
                k += 1;
            }
            if let Some(j) = last {
                if j >= 60 {
                    srv_4h[i] = srv4[j];
                }
                mtf_4h[i] = mtf4[j];
            }
        }
        SymbolFeatures {
            open_time: h1.iter().map(|b| b.open_time).collect(),
            adx: ind::adx(h1, 14),
            er: ind::efficiency_ratio(&c, 20),
            chop: ind::chop(h1, 14),
            srv: ind::srv(&c, 30, 3).0,
            bft: ind::bilateral_state(h1, 60, 12.0, 1.5, 1.75),
            dec: ind::decycler_state(h1, 115, 14, 0.1, 3, 0.05, 0.5, 0.45),
            mtf_1h: mtf_series(&c),
            close: c,
            atr_pct,
            atr_ratio,
            srv_4h,
            mtf_4h,
        }
    }
}

/// Keputusan mentah (sebelum histeresis) di bar 1h ke-i.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct SymbolRegime {
    pub raw: SymState,
    pub dir_votes: i32,
    pub range_flags: usize,
    pub trend_ok: bool,
    pub reasons: Vec<String>,
}

/// `mtf_15m`: vote MTF 15m pada bar 15m tertutup terakhir (≤ close bar 1h ini); 0 bila tidak ada.
pub fn classify(f: &SymbolFeatures, i: usize, mtf_15m: i8, cfg: &ScreenerConfig) -> SymbolRegime {
    let mut reasons = Vec::new();
    let (adx, er, chop, srv, ar, atrp) = (f.adx[i], f.er[i], f.chop[i], f.srv[i], f.atr_ratio[i], f.atr_pct[i]);
    if [adx, er, chop, srv, ar, atrp, f.srv_4h[i]].iter().any(|x| !x.is_finite()) || i < 3 {
        return SymbolRegime { raw: SymState::Blocked, dir_votes: 0, range_flags: 0, trend_ok: false, reasons: vec!["warmup/data".into()] };
    }
    // RC-9: ATR% di luar band → BLOCKED (SL 2–4% tidak realistis).
    if atrp < cfg.atr_pct_min || atrp > cfg.atr_pct_max {
        return SymbolRegime { raw: SymState::Blocked, dir_votes: 0, range_flags: 0, trend_ok: false, reasons: vec![format!("RC-9 atr%={atrp:.2}")] };
    }
    let mtf_sum = mtf_15m as i32 + f.mtf_1h[i] as i32 + f.mtf_4h[i] as i32;
    let mtf_major = if mtf_sum >= 2 { 1 } else if mtf_sum <= -2 { -1 } else { 0 };
    let mut rf = 0;
    if adx < cfg.adx_range {
        rf += 1;
        reasons.push("RC-1".into());
    }
    if er < cfg.er_range {
        rf += 1;
        reasons.push("RC-2".into());
    }
    if chop >= cfg.chop_range {
        rf += 1;
        reasons.push("RC-3".into());
    }
    if srv.abs() < cfg.srv_range {
        rf += 1;
        reasons.push("RC-4".into());
    }
    if mtf_major == 0 {
        rf += 1;
        reasons.push("RC-7".into());
    }
    if ar < cfg.atr_ratio_range {
        rf += 1;
        reasons.push("RC-8".into());
    }
    let adx_ok = (adx >= cfg.adx_trend && adx > f.adx[i - 3]) || adx >= cfg.adx_strong;
    let trend_ok = adx_ok && (er >= cfg.er_trend || chop <= cfg.chop_trend) && ar >= cfg.atr_ratio_trend;
    let srv_vote = if srv >= cfg.srv_trend { 1 } else if srv <= -cfg.srv_trend { -1 } else { 0 };
    let votes = srv_vote + f.bft[i] as i32 + f.dec[i] as i32 + mtf_major;
    let raw = if rf >= cfg.min_range_flags {
        SymState::Ranging
    } else if trend_ok && votes >= cfg.min_votes && f.srv_4h[i] >= 0.0 {
        SymState::TrendUp
    } else if trend_ok && votes <= -cfg.min_votes && f.srv_4h[i] <= 0.0 {
        SymState::TrendDown
    } else {
        SymState::Ranging
    };
    SymbolRegime { raw, dir_votes: votes, range_flags: rf, trend_ok, reasons }
}

/// Histeresis: TREND butuh `enter_bars` bar 1h beruntun dengan raw TREND arah sama; keluar langsung (1 bar).
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct ScreenerMachine {
    pub state: Option<SymState>,
    pub pending: Option<SymState>,
    pub pending_n: u32,
}

impl ScreenerMachine {
    pub fn step(&mut self, raw: SymState, cfg: &ScreenerConfig) -> SymState {
        let cur = self.state.unwrap_or(SymState::Ranging);
        let next = match raw {
            SymState::TrendUp | SymState::TrendDown => {
                if cur == raw {
                    self.pending = None;
                    self.pending_n = 0;
                    raw
                } else {
                    if self.pending == Some(raw) {
                        self.pending_n += 1;
                    } else {
                        self.pending = Some(raw);
                        self.pending_n = 1;
                    }
                    if self.pending_n >= cfg.enter_bars {
                        self.pending = None;
                        self.pending_n = 0;
                        raw
                    } else if cur.dir() != 0 {
                        SymState::Ranging // trend berlawanan/terputus → keluar dulu (1 bar)
                    } else {
                        cur
                    }
                }
            }
            other => {
                self.pending = None;
                self.pending_n = 0;
                other
            }
        };
        self.state = Some(next);
        next
    }
    pub fn current(&self) -> SymState {
        self.state.unwrap_or(SymState::Ranging)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hysteresis_enter_two_exit_one() {
        let c = ScreenerConfig::default();
        let mut m = ScreenerMachine::default();
        assert_eq!(m.step(SymState::TrendUp, &c), SymState::Ranging);
        assert_eq!(m.step(SymState::TrendUp, &c), SymState::TrendUp);
        assert_eq!(m.step(SymState::TrendUp, &c), SymState::TrendUp);
        assert_eq!(m.step(SymState::Ranging, &c), SymState::Ranging);
        assert_eq!(m.step(SymState::TrendUp, &c), SymState::Ranging);
        assert_eq!(m.step(SymState::Blocked, &c), SymState::Blocked);
        assert_eq!(m.step(SymState::TrendDown, &c), SymState::Blocked);
        assert_eq!(m.step(SymState::TrendDown, &c), SymState::TrendDown);
        // flip langsung ke arah berlawanan: keluar dulu
        assert_eq!(m.step(SymState::TrendUp, &c), SymState::Ranging);
        assert_eq!(m.step(SymState::TrendUp, &c), SymState::TrendUp);
    }

    fn bars(n: usize, drift: f64) -> Vec<Bar> {
        let mut p = 100.0;
        let mut s = 99u64;
        (0..n)
            .map(|i| {
                s ^= s << 13;
                s ^= s >> 7;
                s ^= s << 17;
                let r = ((s >> 11) as f64 / (1u64 << 53) as f64 - 0.5) * 0.012 + drift;
                let o = p;
                p *= 1.0 + r;
                Bar { open_time: i as i64 * 3_600_000, open: o, high: o.max(p) * 1.003, low: o.min(p) * 0.997, close: p, volume: 1.0, quote_volume: p, trades: 1, taker_buy_base: 0.5 }
            })
            .collect()
    }

    #[test]
    fn steady_uptrend_not_down_and_noise_not_trend() {
        let c = ScreenerConfig::default();
        let up = bars(1400, 0.0025);
        let f = SymbolFeatures::compute(&up, &c);
        let downs = (900..1400).filter(|&i| classify(&f, i, 1, &c).raw == SymState::TrendDown).count();
        assert_eq!(downs, 0);
        let noise = bars(1400, 0.0);
        let f = SymbolFeatures::compute(&noise, &c);
        let trends = (900..1400).filter(|&i| classify(&f, i, 0, &c).raw.dir() != 0).count();
        assert!(trends < 100, "noise trend count {trends}");
    }

    #[test]
    fn features_prefix_causal() {
        let c = ScreenerConfig::default();
        let b = bars(1300, 0.001);
        let full = SymbolFeatures::compute(&b, &c);
        let cut = 1000;
        let part = SymbolFeatures::compute(&b[..=cut], &c);
        let a = classify(&full, cut, 0, &c);
        let p = classify(&part, cut, 0, &c);
        assert_eq!(a.raw, p.raw);
        assert_eq!(a.dir_votes, p.dir_votes);
        assert_eq!(full.srv_4h[cut].to_bits(), part.srv_4h[cut].to_bits());
    }
}
