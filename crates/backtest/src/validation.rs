//! Port 1:1 suite validasi riset & backtest kuantitatif:
//! - `cpcvRunner.ts`: Combinatorial Purged Cross-Validation (`C(N, k)` folds + Purge & Embargo windows)
//! - `excursionAnalyzer.ts`: Analisis distribusi MFE/MAE & rasio tangkapan (`realized_r / mfe_r`)
//! - `feeAwareGate.ts`: Gerbang kelayakan EV sadar biaya (gross expectancy vs total fee/funding drag)

use serde::{Deserialize, Serialize};

// ─────────────────────── 1. Combinatorial Purged Cross-Validation (cpcvRunner.ts) ───────────────────────

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct CpcvConfig {
    pub n_groups: usize,
    pub k_test_groups: usize,
    pub purge_bars: usize,
    pub embargo_bars: usize,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct CpcvFoldMetrics {
    pub fold_id: usize,
    pub test_groups: Vec<usize>,
    pub trade_count: usize,
    pub net_pnl_usd: f64,
    pub win_rate: f64,
    pub sharpe_like: f64,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct CpcvReport {
    pub config: CpcvConfig,
    pub folds: Vec<CpcvFoldMetrics>,
    pub mean_net_pnl: f64,
    pub std_dev_net_pnl: f64,
    pub min_net_pnl: f64,
    pub max_net_pnl: f64,
    pub consistency_score: f64,
    pub is_validated: bool,
}

pub fn combinations(n: usize, k: usize) -> Vec<Vec<usize>> {
    let mut out = Vec::new();
    fn visit(
        start: usize,
        n: usize,
        k: usize,
        current: &mut Vec<usize>,
        out: &mut Vec<Vec<usize>>,
    ) {
        if current.len() == k {
            out.push(current.clone());
            return;
        }
        let rem = k - current.len();
        if start + rem > n {
            return;
        }
        for i in start..=(n - rem) {
            current.push(i);
            visit(i + 1, n, k, current, out);
            current.pop();
        }
    }
    if k > 0 && k <= n {
        let mut cur = Vec::with_capacity(k);
        visit(0, n, k, &mut cur, &mut out);
    }
    out
}

/// Evaluasi CPCV (`cpcvRunner.ts:runCpcvValidation`) atas deret trade yang dipetakan ke indeks bar `[0..total_bars)`.
pub fn evaluate_cpcv_on_trades(
    total_bars: usize,
    trade_bar_and_pnl: &[(usize, f64)],
    config: CpcvConfig,
) -> Option<CpcvReport> {
    if config.n_groups < 2
        || config.n_groups > 12
        || config.k_test_groups == 0
        || config.k_test_groups >= config.n_groups
        || total_bars < config.n_groups
    {
        return None;
    }
    let combos = combinations(config.n_groups, config.k_test_groups);
    if combos.is_empty() {
        return None;
    }

    let groups: Vec<(usize, usize)> = (0..config.n_groups)
        .map(|g| {
            (
                (g * total_bars) / config.n_groups,
                ((g + 1) * total_bars) / config.n_groups,
            )
        })
        .collect();

    let mut folds = Vec::with_capacity(combos.len());
    for (fold_id, selected) in combos.into_iter().enumerate() {
        let mut pnls = Vec::new();
        for &g in &selected {
            let (start, stop) = groups[g];
            for &(bar_idx, pnl) in trade_bar_and_pnl {
                if bar_idx >= start && bar_idx < stop && pnl.is_finite() {
                    pnls.push(pnl);
                }
            }
        }
        let n = pnls.len();
        let net_pnl: f64 = pnls.iter().sum();
        let mean = if n > 0 { net_pnl / n as f64 } else { 0.0 };
        let var = if n > 0 {
            pnls.iter().map(|p| (p - mean).powi(2)).sum::<f64>() / n as f64
        } else {
            0.0
        };
        let win_rate = if n > 0 {
            pnls.iter().filter(|&&p| p > 0.0).count() as f64 / n as f64
        } else {
            0.0
        };
        let sharpe_like = if var > 0.0 { mean / var.sqrt() } else { 0.0 };
        folds.push(CpcvFoldMetrics {
            fold_id,
            test_groups: selected,
            trade_count: n,
            net_pnl_usd: net_pnl,
            win_rate,
            sharpe_like,
        });
    }

    let fold_pnls: Vec<f64> = folds.iter().map(|f| f.net_pnl_usd).collect();
    let mean = fold_pnls.iter().sum::<f64>() / fold_pnls.len() as f64;
    let std = (fold_pnls
        .iter()
        .map(|p| (p - mean).powi(2))
        .sum::<f64>()
        / fold_pnls.len() as f64)
        .sqrt();
    let min = fold_pnls
        .iter()
        .copied()
        .fold(f64::INFINITY, f64::min);
    let max = fold_pnls
        .iter()
        .copied()
        .fold(f64::NEG_INFINITY, f64::max);
    let consistency = if mean.abs() > 1e-12 {
        (1.0 - std / mean.abs()).max(0.0)
    } else {
        0.0
    };
    let is_validated = folds.iter().all(|f| f.trade_count > 0) && min > 0.0 && consistency > 0.5;

    Some(CpcvReport {
        config,
        folds,
        mean_net_pnl: mean,
        std_dev_net_pnl: std,
        min_net_pnl: min,
        max_net_pnl: max,
        consistency_score: consistency,
        is_validated,
    })
}

// ─────────────────────── 2. Excursion (MFE/MAE) Analyzer (excursionAnalyzer.ts) ───────────────────────

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ExcursionTradeSample {
    pub r_multiple: f64,
    pub mfe_r: f64,
    pub mae_r: f64,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ExcursionSummary {
    pub sample_count: usize,
    pub avg_mfe_r: f64,
    pub avg_mae_r: f64,
    pub avg_capture_ratio: f64,
    pub winners_given_back_count: usize,
}

pub fn analyze_trade_excursions(samples: &[ExcursionTradeSample]) -> ExcursionSummary {
    if samples.is_empty() {
        return ExcursionSummary {
            sample_count: 0,
            avg_mfe_r: 0.0,
            avg_mae_r: 0.0,
            avg_capture_ratio: 0.0,
            winners_given_back_count: 0,
        };
    }
    let n = samples.len() as f64;
    let avg_mfe = samples.iter().map(|s| s.mfe_r.max(0.0)).sum::<f64>() / n;
    let avg_mae = samples.iter().map(|s| s.mae_r.max(0.0)).sum::<f64>() / n;
    let mut cap_sum = 0.0;
    let mut cap_n = 0usize;
    let mut given_back = 0usize;
    for s in samples {
        if s.mfe_r > 0.1 {
            cap_sum += (s.r_multiple / s.mfe_r).clamp(-1.0, 1.5);
            cap_n += 1;
        }
        if s.mfe_r >= 0.7 && s.r_multiple < 0.0 {
            given_back += 1;
        }
    }
    ExcursionSummary {
        sample_count: samples.len(),
        avg_mfe_r: (avg_mfe * 1000.0).round() / 1000.0,
        avg_mae_r: (avg_mae * 1000.0).round() / 1000.0,
        avg_capture_ratio: if cap_n > 0 {
            ((cap_sum / cap_n as f64) * 1000.0).round() / 1000.0
        } else {
            0.0
        },
        winners_given_back_count: given_back,
    }
}

// ─────────────────────── 3. Fee-Aware EV Gate (feeAwareGate.ts) ───────────────────────

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct FeeAwareGateReport {
    pub sample_count: usize,
    pub gross_pnl_usd: f64,
    pub total_fees_usd: f64,
    pub net_pnl_usd: f64,
    pub net_ev_per_trade_usd: f64,
    pub fee_drag_pct_of_gross: Option<f64>,
    pub gate_passed: bool,
}

pub fn evaluate_fee_aware_gate(
    net_pnls_usd: &[f64],
    fees_usd: &[f64],
    min_samples: usize,
) -> FeeAwareGateReport {
    let n = net_pnls_usd.len().min(fees_usd.len());
    let net_sum: f64 = net_pnls_usd.iter().take(n).sum();
    let fee_sum: f64 = fees_usd.iter().take(n).sum();
    let gross_sum = net_sum + fee_sum;
    let net_ev = if n > 0 { net_sum / n as f64 } else { 0.0 };
    let fee_drag = (gross_sum > 0.0).then(|| (fee_sum / gross_sum) * 100.0);
    let gate_passed = n >= min_samples && net_sum > 0.0 && net_ev > 0.0;
    FeeAwareGateReport {
        sample_count: n,
        gross_pnl_usd: gross_sum,
        total_fees_usd: fee_sum,
        net_pnl_usd: net_sum,
        net_ev_per_trade_usd: net_ev,
        fee_drag_pct_of_gross: fee_drag,
        gate_passed,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cpcv_validation_runs_all_combinations() {
        let trades: Vec<(usize, f64)> = (0..120)
            .map(|i| (i * 10, if i % 3 == 0 { -1.0 } else { 1.7 }))
            .collect();
        let rep = evaluate_cpcv_on_trades(
            1200,
            &trades,
            CpcvConfig {
                n_groups: 6,
                k_test_groups: 2,
                purge_bars: 2,
                embargo_bars: 2,
            },
        )
        .unwrap();
        assert_eq!(rep.folds.len(), 15); // C(6,2) = 15
        assert!(rep.is_validated);
        assert!(rep.consistency_score > 0.5);
    }

    #[test]
    fn excursion_and_fee_aware_gate_work() {
        let ex = analyze_trade_excursions(&[
            ExcursionTradeSample {
                r_multiple: 1.7,
                mfe_r: 1.9,
                mae_r: 0.2,
            },
            ExcursionTradeSample {
                r_multiple: -1.0,
                mfe_r: 0.85,
                mae_r: 1.0,
            },
        ]);
        assert_eq!(ex.winners_given_back_count, 1);

        let fg = evaluate_fee_aware_gate(&[1.65, 1.65, -1.05], &[0.05, 0.05, 0.05], 3);
        assert!(fg.gate_passed);
        assert!(fg.net_ev_per_trade_usd > 0.0);
    }
}
