//! Timeline regime untuk backtest: Market Reader dengan PROXY Binance (tanpa histori TV):
//! MR-01 BTCUSDT; MR-03 BTCDOMUSDT (cadangan resmi TV-06); MR-04 total = 0,6·BTC + 0,4·median alt (proxy),
//! alt = median chg24 basket alt; MR-07 breadth dari universe; MR-10 proxy ekskursi bar 15m BTC ≥ 1,5%.
//! MR-05/MR-06 (shadow) tidak tersedia historis.
use apex_core::types::{Bar, Ms, Tf};
use apex_indicators as ind;
use apex_market_reader::aggregate::ReaderSnapshot;
use apex_market_reader::readers::{chg24, dir_from_chg, last_closed, shock_proxy_15m, CrowdFlags, TrendFeatures};
use apex_market_reader::{RegimeDecision, RegimeMachine, RegimeState, UniverseScope};
use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum RegimeMode {
    /// Market Reader penuh (proxy) + screener per simbol.
    Reader,
    /// Hanya screener per simbol (diagnostik; BUKAN konfigurasi live).
    ScreenerOnly,
}

pub struct AltInput<'a> {
    pub h1: &'a [Bar],
}

pub struct RegimeTimeline {
    pub times: Vec<Ms>,
    pub decisions: Vec<RegimeDecision>,
}

struct AltFeat {
    ot: Vec<Ms>,
    close: Vec<f64>,
    ema50: Vec<f64>,
    er20: Vec<f64>,
}

impl RegimeTimeline {
    /// `close_times`: waktu close bar 15m (as_of) berurutan. `cap_on_proxy`: terapkan aturan TV-06 (max_pos=1) karena sumber cadangan.
    pub fn build(btc_m15: &[Bar], btc_h1: &[Bar], dom_h1: Option<&[Bar]>, alts: &[AltInput], close_times: &[Ms], cap_on_proxy: bool) -> RegimeTimeline {
        let tf = TrendFeatures::compute(btc_h1, btc_m15);
        let alt: Vec<AltFeat> = alts
            .iter()
            .map(|a| {
                let c = ind::closes(a.h1);
                AltFeat { ot: a.h1.iter().map(|b| b.open_time).collect(), ema50: ind::ema(&c, 50), er20: ind::efficiency_ratio(&c, 20), close: c }
            })
            .collect();
        let dom_ot: Vec<Ms> = dom_h1.map(|d| d.iter().map(|b| b.open_time).collect()).unwrap_or_default();
        let dom_c: Vec<f64> = dom_h1.map(ind::closes).unwrap_or_default();
        let btc15_ot: Vec<Ms> = btc_m15.iter().map(|b| b.open_time).collect();
        let mut m = RegimeMachine::default();
        let mut decisions = Vec::with_capacity(close_times.len());
        for &as_of in close_times {
            let mut reasons = Vec::new();
            let i1 = tf.idx_1h(as_of);
            let i15 = tf.idx_15m(as_of);
            let btc_chg = i1.and_then(|i| chg24(&tf.close_1h, i));
            let dom_chg = last_closed(&dom_ot, Tf::H1, as_of).and_then(|i| if as_of - (dom_ot[i] + Tf::H1.ms()) <= 2 * Tf::H1.ms() { chg24(&dom_c, i) } else { None });
            let mut alt_chg = Vec::new();
            let mut br = Vec::new();
            for a in &alt {
                if let Some(i) = last_closed(&a.ot, Tf::H1, as_of) {
                    if as_of - (a.ot[i] + Tf::H1.ms()) > 2 * Tf::H1.ms() {
                        continue; // simbol stale/belum listing
                    }
                    if let Some(c) = chg24(&a.close, i) {
                        alt_chg.push(c);
                    }
                    br.push((a.close[i], a.ema50[i], a.er20[i]));
                }
            }
            let alt_med = if alt_chg.len() >= 3 { Some(ind::percentile(&alt_chg, 50.0)) } else { None };
            let breadth = apex_market_reader::readers::breadth(&br);
            let data_ok = i1.is_some() && i15.is_some() && btc_chg.is_some() && dom_chg.is_some() && alt_med.is_some() && breadth.is_some()
                && i1.map(|i| tf.adx_1h[i].is_finite() && tf.atr_pctile_1h[i].is_finite()).unwrap_or(false)
                && i15.map(|i| tf.er_15m[i].is_finite() && as_of - (tf.open_time_15m[i] + Tf::M15.ms()) <= Tf::M15.ms()).unwrap_or(false);
            if !data_ok {
                reasons.push("MR wajib tidak lengkap/stale".to_string());
            }
            let shock = last_closed(&btc15_ot, Tf::M15, as_of).map(|i| shock_proxy_15m(&btc_m15[i], 1.5)).unwrap_or(false);
            let (bu, ts) = breadth.unwrap_or((f64::NAN, f64::NAN));
            let total = btc_chg.zip(alt_med).map(|(b, a)| 0.6 * b + 0.4 * a).unwrap_or(0.0);
            let snap = ReaderSnapshot {
                as_of,
                data_ok,
                data_reasons: reasons,
                shock,
                btc_trend: i1.map(|i| tf.trend[i]).unwrap_or(apex_market_reader::Trend::None),
                adx_1h: i1.map(|i| tf.adx_1h[i]).unwrap_or(f64::NAN),
                er_15m: i15.map(|i| tf.er_15m[i]).unwrap_or(f64::NAN),
                atr_pctile_1h: i1.map(|i| tf.atr_pctile_1h[i]).unwrap_or(f64::NAN),
                dom_dir: dir_from_chg(dom_chg.unwrap_or(0.0), 0.25),
                total_dir: dir_from_chg(total, 0.5),
                alt_dir: dir_from_chg(alt_med.unwrap_or(0.0), 0.5),
                breadth_up: bu,
                trend_share: ts,
                dom_fallback: cap_on_proxy,
                optional_missing: false,
                crowd: CrowdFlags::default(),
            };
            decisions.push(m.step(&snap));
        }
        RegimeTimeline { times: close_times.to_vec(), decisions }
    }

    /// Timeline "semua boleh" untuk mode ScreenerOnly.
    pub fn permissive(close_times: &[Ms]) -> RegimeTimeline {
        RegimeTimeline {
            times: close_times.to_vec(),
            decisions: close_times
                .iter()
                .map(|&t| RegimeDecision { as_of: t, state: RegimeState::RiskOnAlts, raw: RegimeState::RiskOnAlts, long_ok: true, short_ok: true, universe: UniverseScope::All, max_positions: 2, reasons: vec!["SCREENER_ONLY".into()] })
                .collect(),
        }
    }
}
