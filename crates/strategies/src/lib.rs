//! Strategi S1–S5 (STRATEGIES.md). Deterministik, HANYA bar tertutup.
//!
//! Desain anti look-ahead:
//! * Fitur dihitung kausal sekali per seri (`SideSeries`), lalu tiap evaluasi menerima `View`
//!   yang berisi slice TERPOTONG sampai bar t (dan bar 1h terakhir yang sudah tutup pada close bar t).
//!   Strategi secara fisik tidak bisa membaca data > t.
//! * Short = logika long pada seri cermin (harga resiprokal 1/p, delta dibalik), lalu harga dikonversi
//!   balik. Jadi satu implementasi per strategi, simetris dan teruji.
use apex_core::sizing::EntryKind;
use apex_core::types::{Bar, Ms, Side, Tf};
use apex_indicators::{self as ind, Pivot};
use apex_screener::SymState;
use serde::{Deserialize, Serialize};

pub mod strats;
pub use strats::*;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
pub enum StrategyId {
    S1Tfp,
    S2Lsr,
    S3Sbr,
    S4Wsl,
    S5Rmp,
}

impl StrategyId {
    pub const ALL: [StrategyId; 5] = [StrategyId::S1Tfp, StrategyId::S2Lsr, StrategyId::S3Sbr, StrategyId::S4Wsl, StrategyId::S5Rmp];
    pub fn code(&self) -> &'static str {
        match self {
            StrategyId::S1Tfp => "S1_TFP",
            StrategyId::S2Lsr => "S2_LSR",
            StrategyId::S3Sbr => "S3_SBR",
            StrategyId::S4Wsl => "S4_WSL",
            StrategyId::S5Rmp => "S5_RMP",
        }
    }
    pub fn parse(s: &str) -> Option<StrategyId> {
        let u = s.to_ascii_uppercase();
        StrategyId::ALL.into_iter().find(|x| x.code() == u || x.code().starts_with(&format!("{u}_")) || u == x.code()[..2])
    }
}

/// Sinyal entry (harga NYATA, belum dibulatkan tick; sizing & rounding di core).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Signal {
    pub strategy: StrategyId,
    pub symbol: String,
    pub side: Side,
    pub entry: f64,
    pub sl: f64,
    pub kind: EntryKind,
    /// TTL order limit dalam bar 15m.
    pub ttl_bars: u32,
    /// Waktu close bar sinyal (as_of); order baru aktif SETELAH ini.
    pub bar_close_ms: Ms,
    /// Batalkan order pending bila close 15m melewati level ini ke arah rugi (S1: tepi FVG).
    pub cancel_beyond: Option<f64>,
    pub note: String,
}

impl Signal {
    pub fn sl_pct(&self) -> f64 {
        (self.entry - self.sl).abs() / self.entry
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(default)]
pub struct StrategyParams {
    pub sl_buffer_atr: f64,
    pub s1_gap_atr: f64,
    pub s1_body_frac: f64,
    pub s1_srv_min: f64,
    pub s1_ttl: u32,
    pub s2_pool_age: usize,
    pub s2_reclaim_bars: usize,
    pub s2_ttl: u32,
    pub s3_retest_bars: usize,
    pub s3_sbpo_min: f64,
    pub s3_market: bool,
    pub s3_ttl: u32,
    pub s4_min_range: usize,
    pub s4_max_range: usize,
    pub s4_min_height: f64,
    pub s4_lps_bars: usize,
    pub s4_ttl: u32,
    pub s5_distrank_min: f64,
    pub s5_apexwave_min: f64,
    pub s5_impulse_atr: f64,
    pub s5_ttl: u32,
    /// Optional K variant. No grade means no entry.
    pub s5_knn_enabled: bool,
    pub sl_min_pct: f64,
    pub sl_max_pct: f64,
}

impl Default for StrategyParams {
    fn default() -> Self {
        StrategyParams {
            sl_buffer_atr: 0.15,
            s1_gap_atr: 0.30,
            s1_body_frac: 0.60,
            s1_srv_min: 0.20,
            s1_ttl: 16,
            s2_pool_age: 10,
            s2_reclaim_bars: 3,
            s2_ttl: 4,
            s3_retest_bars: 8,
            s3_sbpo_min: 0.5,
            s3_market: false,
            s3_ttl: 4,
            s4_min_range: 10,
            s4_max_range: 60,
            s4_min_height: 0.04,
            s4_lps_bars: 24,
            s4_ttl: 8,
            s5_distrank_min: 70.0,
            s5_apexwave_min: 60.0,
            s5_impulse_atr: 2.5,
            s5_ttl: 4,
            s5_knn_enabled: false,
            sl_min_pct: 0.02,
            sl_max_pct: 0.04,
        }
    }
}

/// Cermin resiprokal untuk sisi short.
pub fn mirror_bar(b: &Bar) -> Bar {
    Bar {
        open_time: b.open_time,
        open: 1.0 / b.open,
        high: 1.0 / b.low,
        low: 1.0 / b.high,
        close: 1.0 / b.close,
        volume: b.volume,
        quote_volume: b.quote_volume,
        trades: b.trades,
        taker_buy_base: b.volume - b.taker_buy_base,
    }
}

/// Fitur 15m (kausal).
#[derive(Clone, Debug, Default)]
pub struct F15 {
    pub atr: Vec<f64>,
    pub ema21: Vec<f64>,
    pub rsi: Vec<f64>,
    pub srv: Vec<f64>,
    pub delta: Vec<f64>,
    /// P95 |delta| atas 500 bar SEBELUMNYA (dihitung ulang tiap 16 bar; tetap kausal).
    pub p95_absdelta: Vec<f64>,
    pub p90_absdelta: Vec<f64>,
    /// rata-rata body 20 bar SEBELUMNYA.
    pub body_avg20: Vec<f64>,
    pub vol_sma20: Vec<f64>,
    /// percent rank (100 bar) jumlah delta 3 bar → proxy "Pressure Radar".
    pub pressure: Vec<f64>,
    /// Proxy MSO: EMA5 dari stokastik 20 (−1..1) dan sinyal EMA9.
    pub mso: Vec<f64>,
    pub mso_sig: Vec<f64>,
}

#[derive(Clone, Debug, Default)]
pub struct F1h {
    pub atr: Vec<f64>,
    pub ema20: Vec<f64>,
    pub ema50: Vec<f64>,
    pub ema200: Vec<f64>,
    pub dist_rank: Vec<f64>,
    pub apexwave: Vec<f64>,
    pub dec: Vec<i8>,
    pub vol_sma20: Vec<f64>,
}

fn rolling_pct_prev(v: &[f64], window: usize, p: f64, every: usize) -> Vec<f64> {
    let mut out = vec![f64::NAN; v.len()];
    let mut cur = f64::NAN;
    for i in 0..v.len() {
        if i >= window && (i % every == 0 || cur.is_nan()) {
            cur = ind::percentile(&v[i - window..i], p);
        }
        if i >= window {
            out[i] = cur;
        }
    }
    out
}

impl F15 {
    pub fn compute(b: &[Bar]) -> F15 {
        let c = ind::closes(b);
        let delta: Vec<f64> = b.iter().map(|x| x.delta()).collect();
        let absd: Vec<f64> = delta.iter().map(|x| x.abs()).collect();
        let body: Vec<f64> = b.iter().map(|x| x.body()).collect();
        let bs = ind::sma(&body, 20);
        let body_avg20 = (0..b.len()).map(|i| if i >= 1 { bs[i - 1] } else { f64::NAN }).collect();
        let vol: Vec<f64> = b.iter().map(|x| x.volume).collect();
        let d3: Vec<f64> = (0..b.len()).map(|i| if i >= 2 { delta[i] + delta[i - 1] + delta[i - 2] } else { f64::NAN }).collect();
        let mut stoch = vec![f64::NAN; b.len()];
        for i in 19..b.len() {
            let hh = b[i - 19..=i].iter().map(|x| x.high).fold(f64::MIN, f64::max);
            let ll = b[i - 19..=i].iter().map(|x| x.low).fold(f64::MAX, f64::min);
            stoch[i] = if hh > ll { (c[i] - ll) / (hh - ll) * 2.0 - 1.0 } else { 0.0 };
        }
        let mso = ind::ema(&stoch, 5);
        let mso_sig = ind::ema(&mso, 9);
        F15 {
            atr: ind::atr(b, 14),
            ema21: ind::ema(&c, 21),
            rsi: ind::rsi_wilder(&c, 14),
            srv: ind::srv(&c, 30, 3).0,
            p95_absdelta: rolling_pct_prev(&absd, 500, 95.0, 16),
            p90_absdelta: rolling_pct_prev(&absd, 500, 90.0, 16),
            delta,
            body_avg20,
            vol_sma20: ind::sma(&vol, 20),
            pressure: ind::percent_rank(&d3, 100),
            mso,
            mso_sig,
        }
    }
}

impl F1h {
    pub fn compute(b: &[Bar]) -> F1h {
        let c = ind::closes(b);
        let vol: Vec<f64> = b.iter().map(|x| x.volume).collect();
        F1h {
            atr: ind::atr(b, 14),
            ema20: ind::ema(&c, 20),
            ema50: ind::ema(&c, 50),
            ema200: ind::ema(&c, 200),
            dist_rank: ind::dist_rank(&c, 100, 5),
            apexwave: ind::apexwave_bull(b),
            dec: ind::decycler_state(b, 115, 14, 0.1, 3, 0.05, 0.5, 0.45),
            vol_sma20: ind::sma(&vol, 20),
        }
    }
}

/// Satu sisi (long = asli, short = cermin) dari satu simbol.
#[derive(Clone, Debug)]
pub struct SideSeries {
    pub side: Side,
    pub m15: Vec<Bar>,
    pub h1: Vec<Bar>,
    pub f15: F15,
    pub f1: F1h,
    pub piv15_3: Vec<Pivot>,
    pub piv15_5: Vec<Pivot>,
    pub piv1h_3: Vec<Pivot>,
}

impl SideSeries {
    pub fn new(side: Side, m15: &[Bar], h1: &[Bar]) -> SideSeries {
        let (m, h): (Vec<Bar>, Vec<Bar>) = match side {
            Side::Long => (m15.to_vec(), h1.to_vec()),
            Side::Short => (m15.iter().map(mirror_bar).collect(), h1.iter().map(mirror_bar).collect()),
        };
        let mut p3 = ind::pivots(&m, 3, 3);
        let mut p5 = ind::pivots(&m, 5, 5);
        let mut ph = ind::pivots(&h, 3, 3);
        for v in [&mut p3, &mut p5, &mut ph] {
            v.sort_by_key(|p| (p.confirmed_idx, p.idx));
        }
        SideSeries { side, f15: F15::compute(&m), f1: F1h::compute(&h), m15: m, h1: h, piv15_3: p3, piv15_5: p5, piv1h_3: ph }
    }
}

/// Seri lengkap satu simbol.
#[derive(Clone, Debug)]
pub struct SymbolSeries {
    pub symbol: String,
    pub long: SideSeries,
    pub short: SideSeries,
    entry_tf: Tf,
}

impl SymbolSeries {
    pub fn new(symbol: &str, m15: &[Bar], h1: &[Bar]) -> SymbolSeries {
        Self::new_with_tf(symbol, m15, h1, Tf::M15)
    }
    /// Research-only entry bars may be 5m; higher-timeframe bars must remain 1h.
    pub fn new_with_tf(symbol: &str, entry: &[Bar], h1: &[Bar], entry_tf: Tf) -> SymbolSeries {
        assert!(matches!(entry_tf, Tf::M5 | Tf::M15));
        SymbolSeries { symbol: symbol.to_string(), long: SideSeries::new(Side::Long, entry, h1), short: SideSeries::new(Side::Short, entry, h1), entry_tf }
    }
    pub fn m15(&self) -> &[Bar] {
        &self.long.m15
    }
    pub fn h1(&self) -> &[Bar] {
        &self.long.h1
    }
    /// Indeks bar 1h terakhir yang sudah tutup saat bar 15m ke-t tutup.
    pub fn h1_idx_at(&self, t: usize) -> Option<usize> {
        let close_t = self.long.m15[t].open_time + self.entry_tf.ms();
        self.long.h1.partition_point(|b| b.open_time + Tf::H1.ms() <= close_t).checked_sub(1)
    }
}

/// Slice fitur 15m terpotong sampai t.
pub struct F15View<'a> {
    pub atr: &'a [f64],
    pub ema21: &'a [f64],
    pub rsi: &'a [f64],
    pub srv: &'a [f64],
    pub delta: &'a [f64],
    pub p95_absdelta: &'a [f64],
    pub p90_absdelta: &'a [f64],
    pub body_avg20: &'a [f64],
    pub vol_sma20: &'a [f64],
    pub pressure: &'a [f64],
    pub mso: &'a [f64],
    pub mso_sig: &'a [f64],
}

pub struct F1View<'a> {
    pub atr: &'a [f64],
    pub ema20: &'a [f64],
    pub ema50: &'a [f64],
    pub ema200: &'a [f64],
    pub dist_rank: &'a [f64],
    pub apexwave: &'a [f64],
    pub dec: &'a [i8],
    pub vol_sma20: &'a [f64],
}

/// Pandangan bar entry tertutup (long asli / short cermin), dengan konteks 1h.
pub struct View<'a> {
    pub symbol: &'a str,
    pub side: Side,
    pub m15: &'a [Bar],
    pub f15: F15View<'a>,
    pub h1: &'a [Bar],
    pub f1: F1View<'a>,
    pub piv15_3: &'a [Pivot],
    pub piv15_5: &'a [Pivot],
    pub piv1h_3: &'a [Pivot],
    /// Screener state simbol searah sisi (TREND_UP untuk long, TREND_DOWN untuk short).
    pub trend_ok: bool,
    pub as_of: Ms,
    pub entry_tf: Tf,
}

impl<'a> View<'a> {
    pub fn t(&self) -> usize {
        self.m15.len() - 1
    }
    pub fn h(&self) -> Option<usize> {
        self.h1.len().checked_sub(1)
    }
}

fn cut<T>(v: &[T], n: usize) -> &[T] {
    &v[..n.min(v.len())]
}

fn piv_cut(p: &[Pivot], t: usize) -> &[Pivot] {
    &p[..p.partition_point(|x| x.confirmed_idx <= t)]
}

pub fn make_view<'a>(sym: &'a SymbolSeries, s: &'a SideSeries, t: usize, state: SymState) -> View<'a> {
    let n = t + 1;
    let hn = sym.h1_idx_at(t).map(|h| h + 1).unwrap_or(0);
    let f = &s.f15;
    let g = &s.f1;
    let want = match s.side {
        Side::Long => SymState::TrendUp,
        Side::Short => SymState::TrendDown,
    };
    View {
        symbol: &sym.symbol,
        side: s.side,
        m15: cut(&s.m15, n),
        f15: F15View {
            atr: cut(&f.atr, n),
            ema21: cut(&f.ema21, n),
            rsi: cut(&f.rsi, n),
            srv: cut(&f.srv, n),
            delta: cut(&f.delta, n),
            p95_absdelta: cut(&f.p95_absdelta, n),
            p90_absdelta: cut(&f.p90_absdelta, n),
            body_avg20: cut(&f.body_avg20, n),
            vol_sma20: cut(&f.vol_sma20, n),
            pressure: cut(&f.pressure, n),
            mso: cut(&f.mso, n),
            mso_sig: cut(&f.mso_sig, n),
        },
        h1: cut(&s.h1, hn),
        f1: F1View {
            atr: cut(&g.atr, hn),
            ema20: cut(&g.ema20, hn),
            ema50: cut(&g.ema50, hn),
            ema200: cut(&g.ema200, hn),
            dist_rank: cut(&g.dist_rank, hn),
            apexwave: cut(&g.apexwave, hn),
            dec: cut(&g.dec, hn),
            vol_sma20: cut(&g.vol_sma20, hn),
        },
        piv15_3: piv_cut(&s.piv15_3, t),
        piv15_5: piv_cut(&s.piv15_5, t),
        piv1h_3: piv_cut(&s.piv1h_3, hn.saturating_sub(1)),
        trend_ok: state == want,
        as_of: s.m15[t].open_time + sym.entry_tf.ms(),
        entry_tf: sym.entry_tf,
    }
}

/// Sinyal dalam ruang sisi (long-logic). Dikonversi ke harga nyata oleh `to_real`.
#[derive(Clone, Debug, PartialEq)]
pub struct RawSignal {
    pub entry: f64,
    pub sl: f64,
    pub market: bool,
    pub ttl: u32,
    pub cancel_beyond: Option<f64>,
    pub note: String,
}

pub fn to_real(id: StrategyId, v: &View, r: RawSignal) -> Signal {
    let conv = |x: f64| match v.side {
        Side::Long => x,
        Side::Short => 1.0 / x,
    };
    Signal {
        strategy: id,
        symbol: v.symbol.to_string(),
        side: v.side,
        entry: conv(r.entry),
        sl: conv(r.sl),
        kind: if r.market { EntryKind::Market } else { EntryKind::LimitMaker },
        ttl_bars: r.ttl,
        bar_close_ms: v.as_of,
        cancel_beyond: r.cancel_beyond.map(conv),
        note: r.note,
    }
}

/// Generate candidates (not executable orders) in priority order for both sides.
/// Callers must apply risk gates and, when enabled, the stateful S5 kNN grader.
pub fn evaluate(sym: &SymbolSeries, t: usize, state: SymState, ids: &[StrategyId], p: &StrategyParams) -> Vec<Signal> {
    let mut out = Vec::new();
    for s in [&sym.long, &sym.short] {
        let v = make_view(sym, s, t, state);
        if !v.trend_ok {
            continue;
        }
        for &id in ids {
            if let Some(r) = run_one(id, &v, p) {
                let sig = to_real(id, &v, r);
                if sig.entry.is_finite() && sig.sl.is_finite() && sig.entry > 0.0 && sig.sl > 0.0 {
                    out.push(sig);
                }
            }
        }
    }
    out
}

pub fn run_one(id: StrategyId, v: &View, p: &StrategyParams) -> Option<RawSignal> {
    match id {
        StrategyId::S1Tfp => s1_tfp(v, p),
        StrategyId::S2Lsr => s2_lsr(v, p),
        StrategyId::S3Sbr => s3_sbr(v, p),
        StrategyId::S4Wsl => s4_wsl(v, p),
        StrategyId::S5Rmp => s5_rmp(v, p),
    }
}
pub mod ofcc_watch;
pub mod knn;
