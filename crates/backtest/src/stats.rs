//! Metrik, Monte Carlo bootstrap, PSR / Deflated Sharpe (Bailey & López de Prado).
use crate::engine::Trade;
use apex_core::risk::ExitKind;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct Metrics {
    pub trades: usize,
    pub wins: usize,
    pub losses: usize,
    pub win_rate: f64,
    pub net_usd: f64,
    pub gross_profit: f64,
    pub gross_loss: f64,
    pub profit_factor: f64,
    pub expectancy_r: f64,
    pub total_r: f64,
    pub max_dd_usd: f64,
    pub max_dd_r: f64,
    pub final_balance: f64,
    pub fees_usd: f64,
    pub funding_usd: f64,
    pub longest_loss_streak: usize,
    pub sl_exits: usize,
    pub tp_exits: usize,
    pub liquidations: usize,
    pub max_symbol_profit_share: f64,
    pub max_month_profit_share: f64,
}

impl Metrics {
    pub fn from_trades(tr: &[Trade], start: f64) -> Metrics {
        let mut m = Metrics { trades: tr.len(), final_balance: start, ..Default::default() };
        if tr.is_empty() {
            return m;
        }
        let mut sorted: Vec<&Trade> = tr.iter().collect();
        sorted.sort_by_key(|t| (t.exit_ms, t.entry_ms));
        let (mut eq, mut peak, mut dd) = (start, start, 0.0f64);
        let (mut req, mut rpeak, mut rdd) = (0.0, 0.0, 0.0f64);
        let mut streak = 0;
        for t in &sorted {
            eq += t.pnl_net;
            peak = peak.max(eq);
            dd = dd.max(peak - eq);
            req += t.r;
            rpeak = f64::max(rpeak, req);
            rdd = rdd.max(rpeak - req);
            if t.pnl_net > 0.0 {
                m.wins += 1;
                m.gross_profit += t.pnl_net;
                streak = 0;
            } else {
                m.losses += 1;
                m.gross_loss += -t.pnl_net;
                streak += 1;
                m.longest_loss_streak = m.longest_loss_streak.max(streak);
            }
            m.fees_usd += t.fees;
            m.funding_usd += t.funding;
            match t.exit_kind {
                ExitKind::StopLoss => m.sl_exits += 1,
                ExitKind::TakeProfit => m.tp_exits += 1,
                ExitKind::Liquidation => m.liquidations += 1,
                _ => {}
            }
        }
        m.net_usd = eq - start;
        m.final_balance = eq;
        m.max_dd_usd = dd;
        m.max_dd_r = rdd;
        m.total_r = req;
        m.win_rate = m.wins as f64 / tr.len() as f64;
        m.expectancy_r = req / tr.len() as f64;
        m.profit_factor = if m.gross_loss > 0.0 { m.gross_profit / m.gross_loss } else if m.gross_profit > 0.0 { f64::INFINITY } else { 0.0 };
        // G-10 konsentrasi
        if m.net_usd > 0.0 {
            let mut by_sym: BTreeMap<&str, f64> = BTreeMap::new();
            let mut by_month: BTreeMap<String, f64> = BTreeMap::new();
            for t in tr {
                *by_sym.entry(&t.symbol).or_insert(0.0) += t.pnl_net;
                *by_month.entry(apex_core::time::fmt_ymd(t.exit_ms)[..7].to_string()).or_insert(0.0) += t.pnl_net;
            }
            m.max_symbol_profit_share = by_sym.values().fold(0.0f64, |a, &b| a.max(b)) / m.net_usd;
            m.max_month_profit_share = by_month.values().fold(0.0f64, |a, &b| a.max(b)) / m.net_usd;
        }
        m
    }
}

/// Generator deterministik (SplitMix64) — tanpa dependensi RNG, hasil identik lintas OS.
pub struct SplitMix(pub u64);
impl SplitMix {
    pub fn next_u64(&mut self) -> u64 {
        self.0 = self.0.wrapping_add(0x9E3779B97F4A7C15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58476D1CE4E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D049BB133111EB);
        z ^ (z >> 31)
    }
    pub fn below(&mut self, n: usize) -> usize {
        (self.next_u64() % n as u64) as usize
    }
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct MonteCarlo {
    pub paths: usize,
    pub trades_per_path: usize,
    pub p95_max_dd_r: f64,
    pub median_final_r: f64,
    pub p05_final_r: f64,
    /// P(equity < floor) dengan risk $1 ≈ 1R dari start_balance.
    pub p_below_floor: f64,
}

/// G-07: bootstrap urutan R trade.
pub fn monte_carlo(rs: &[f64], trades_per_path: usize, paths: usize, start: f64, floor: f64, seed: u64) -> MonteCarlo {
    if rs.is_empty() || trades_per_path == 0 {
        return MonteCarlo { paths, trades_per_path, ..Default::default() };
    }
    let mut rng = SplitMix(seed);
    let mut dds = Vec::with_capacity(paths);
    let mut finals = Vec::with_capacity(paths);
    let mut below = 0usize;
    for _ in 0..paths {
        let (mut eq, mut peak, mut dd, mut hit) = (0.0f64, 0.0f64, 0.0f64, false);
        for _ in 0..trades_per_path {
            eq += rs[rng.below(rs.len())];
            peak = peak.max(eq);
            dd = dd.max(peak - eq);
            if start + eq < floor {
                hit = true;
            }
        }
        if hit {
            below += 1;
        }
        dds.push(dd);
        finals.push(eq);
    }
    MonteCarlo {
        paths,
        trades_per_path,
        p95_max_dd_r: apex_indicators::percentile(&dds, 95.0),
        median_final_r: apex_indicators::percentile(&finals, 50.0),
        p05_final_r: apex_indicators::percentile(&finals, 5.0),
        p_below_floor: below as f64 / paths as f64,
    }
}

pub fn norm_cdf(x: f64) -> f64 {
    // Abramowitz-Stegun 7.1.26 via erf
    let t = 1.0 / (1.0 + 0.3275911 * (x.abs() / std::f64::consts::SQRT_2));
    let y = 1.0 - (((((1.061405429 * t - 1.453152027) * t) + 1.421413741) * t - 0.284496736) * t + 0.254829592) * t * (-(x * x) / 2.0).exp();
    if x >= 0.0 {
        0.5 * (1.0 + y)
    } else {
        0.5 * (1.0 - y)
    }
}

/// Invers CDF normal (Acklam).
pub fn norm_inv(p: f64) -> f64 {
    let a = [-3.969683028665376e1, 2.209460984245205e2, -2.759285104469687e2, 1.383577518672690e2, -3.066479806614716e1, 2.506628277459239];
    let b = [-5.447609879822406e1, 1.615858368580409e2, -1.556989798598866e2, 6.680131188771972e1, -1.328068155288572e1];
    let c = [-7.784894002430293e-3, -3.223964580411365e-1, -2.400758277161838, -2.549732539343734, 4.374664141464968, 2.938163982698783];
    let d = [7.784695709041462e-3, 3.224671290700398e-1, 2.445134137142996, 3.754408661907416];
    let pl = 0.02425;
    if p <= 0.0 {
        return f64::NEG_INFINITY;
    }
    if p >= 1.0 {
        return f64::INFINITY;
    }
    if p < pl {
        let q = (-2.0 * p.ln()).sqrt();
        (((((c[0] * q + c[1]) * q + c[2]) * q + c[3]) * q + c[4]) * q + c[5]) / ((((d[0] * q + d[1]) * q + d[2]) * q + d[3]) * q + 1.0)
    } else if p <= 1.0 - pl {
        let q = p - 0.5;
        let r = q * q;
        (((((a[0] * r + a[1]) * r + a[2]) * r + a[3]) * r + a[4]) * r + a[5]) * q / (((((b[0] * r + b[1]) * r + b[2]) * r + b[3]) * r + b[4]) * r + 1.0)
    } else {
        -norm_inv(1.0 - p)
    }
}

fn moments(x: &[f64]) -> (f64, f64, f64, f64) {
    let n = x.len() as f64;
    let m = x.iter().sum::<f64>() / n;
    let v = x.iter().map(|a| (a - m).powi(2)).sum::<f64>() / n;
    let sd = v.sqrt();
    if sd == 0.0 {
        return (m, 0.0, 0.0, 3.0);
    }
    let sk = x.iter().map(|a| ((a - m) / sd).powi(3)).sum::<f64>() / n;
    let ku = x.iter().map(|a| ((a - m) / sd).powi(4)).sum::<f64>() / n;
    (m, sd, sk, ku)
}

/// Probabilistic Sharpe Ratio terhadap benchmark `sr_star` (per-trade).
pub fn psr(rets: &[f64], sr_star: f64) -> f64 {
    if rets.len() < 3 {
        return 0.0;
    }
    let (m, sd, sk, ku) = moments(rets);
    if sd == 0.0 {
        return 0.0;
    }
    let sr = m / sd;
    let den = (1.0 - sk * sr + (ku - 1.0) / 4.0 * sr * sr).max(1e-12).sqrt();
    norm_cdf((sr - sr_star) * ((rets.len() - 1) as f64).sqrt() / den)
}

/// Deflated Sharpe: PSR dengan SR* = E[max SR] dari `n_trials` trial (variansi SR antar trial `var_sr`).
pub fn dsr(rets: &[f64], n_trials: usize, var_sr: f64) -> f64 {
    let n = n_trials.max(2) as f64;
    let g = 0.5772156649;
    let sr0 = var_sr.max(0.0).sqrt() * ((1.0 - g) * norm_inv(1.0 - 1.0 / n) + g * norm_inv(1.0 - 1.0 / (n * std::f64::consts::E)));
    psr(rets, sr0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn normal_fns() {
        assert!((norm_cdf(0.0) - 0.5).abs() < 1e-7);
        assert!((norm_cdf(1.96) - 0.975).abs() < 1e-3);
        assert!((norm_inv(0.975) - 1.95996).abs() < 1e-4);
    }

    #[test]
    fn psr_dsr_monotone() {
        let good: Vec<f64> = (0..200).map(|i| if i % 3 == 0 { -1.0 } else { 1.0 }).collect();
        let bad: Vec<f64> = (0..200).map(|i| if i % 3 == 0 { 2.0 } else { -1.0 }).collect();
        assert!(psr(&good, 0.0) > 0.99);
        assert!(psr(&bad, 0.0) < 0.6);
        assert!(dsr(&good, 100, 0.01) <= psr(&good, 0.0));
    }

    #[test]
    fn mc_deterministic() {
        let rs = vec![2.0, -1.0, -1.0, 2.0, -1.0];
        let a = monte_carlo(&rs, 100, 2000, 60.0, 40.0, 7);
        let b = monte_carlo(&rs, 100, 2000, 60.0, 40.0, 7);
        assert_eq!(a.p95_max_dd_r, b.p95_max_dd_r);
        assert!(a.median_final_r > 0.0);
    }
}
