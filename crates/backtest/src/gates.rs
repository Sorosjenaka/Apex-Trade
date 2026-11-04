//! Gerbang validasi murni (Rules §4.3). Tidak menjalankan backtest dan tidak menyentuh holdout.
use crate::stats::sample_variance;
use apex_core::types::Ms;
use serde_json::Value;
use std::collections::BTreeMap;

#[derive(Clone, Debug, PartialEq)]
pub struct AnchoredFold {
    pub index: usize,
    pub train_start: Ms,
    pub train_end: Ms,
    pub test_start: Ms,
    pub test_end: Ms,
}

/// Walk-forward anchored: jendela dibagi `n_test + 1` bagian sama.
/// Fold k menguji bagian k (1-based); train = awal jendela sampai awal tes.
/// Bagian pertama hanya train. Tes terakhir menutup sisa pembulatan.
pub fn anchored_folds(start: Ms, end: Ms, n_test: usize) -> Vec<AnchoredFold> {
    if n_test == 0 || end <= start {
        return Vec::new();
    }
    let parts = (n_test + 1) as i64;
    let span = (end - start) / parts;
    if span <= 0 {
        return Vec::new();
    }
    (1..=n_test)
        .map(|k| {
            let k = k as i64;
            let test_start = start + span * k;
            let test_end = if k == n_test as i64 { end } else { start + span * (k + 1) };
            AnchoredFold { index: k as usize, train_start: start, train_end: test_start, test_start, test_end }
        })
        .collect()
}

#[derive(Clone, Debug, PartialEq)]
pub struct WfGate {
    pub positive: usize,
    pub folds: usize,
    pub worst: f64,
    pub pass: bool,
}

/// G-06: ≥ 4 dari 6 fold ber-ekspektansi > 0, fold terburuk ≥ −0,15R.
pub fn walk_forward_gate(expectancies: &[f64]) -> WfGate {
    let positive = expectancies.iter().filter(|e| **e > 0.0).count();
    let worst = expectancies.iter().copied().reduce(f64::min).unwrap_or(0.0);
    WfGate { positive, folds: expectancies.len(), worst, pass: expectancies.len() == 6 && positive >= 4 && worst >= -0.15 }
}

#[derive(Clone, Debug, PartialEq)]
pub struct PlateauGate {
    pub n_neighbors: usize,
    pub positive_frac: f64,
    pub median: f64,
    pub selected: f64,
    pub pass: bool,
}

pub fn median(xs: &[f64]) -> f64 {
    if xs.is_empty() {
        return f64::NAN;
    }
    let mut v = xs.to_vec();
    v.sort_by(|a, b| a.total_cmp(b));
    let n = v.len();
    if n % 2 == 1 {
        v[n / 2]
    } else {
        0.5 * (v[n / 2 - 1] + v[n / 2])
    }
}

/// G-09 pada satu titik: ≥ 80% tetangga ±1 langkah punya ekspektansi > 0,
/// dan median tetangga ≥ 50% ekspektansi titik terpilih.
pub fn plateau_gate(selected: f64, neighbors: &[f64]) -> PlateauGate {
    let n = neighbors.len();
    let pos = neighbors.iter().filter(|e| **e > 0.0).count();
    let frac = if n == 0 { 0.0 } else { pos as f64 / n as f64 };
    let med = median(neighbors);
    let pass = n > 0 && selected.is_finite() && frac + 1e-12 >= 0.80 && med >= 0.5 * selected;
    PlateauGate { n_neighbors: n, positive_frac: frac, median: med, selected, pass }
}

/// Titik grid untuk pemilihan plateau. `fit_*` = jendela pengembangan, `oos_*` = cek kendala, bukan tujuan maksimal.
#[derive(Clone, Debug, PartialEq)]
pub struct SweepPoint {
    pub threshold: f64,
    pub fit_block: f64,
    pub oos_pf: f64,
    pub fit_e: f64,
}

#[derive(Clone, Debug, PartialEq)]
pub struct PlateauChoice {
    pub admissible: Vec<usize>,
    pub run: Vec<usize>,
    pub adopted_threshold: Option<f64>,
}

fn pf_not_lower(pf: f64, baseline: f64) -> bool {
    if pf.is_nan() || baseline.is_nan() || (pf.is_infinite() && pf.is_sign_negative()) {
        return false;
    }
    if baseline.is_infinite() && baseline.is_sign_positive() {
        return pf.is_infinite() && pf.is_sign_positive();
    }
    if pf.is_infinite() && pf.is_sign_positive() {
        return true;
    }
    pf + 1e-9 >= baseline
}

/// Median grid. Jumlah genap: ambil yang lebih dekat ke baseline (bukan yang PF-nya paling tinggi).
pub fn adopt_threshold(thresholds: &[f64], baseline: f64) -> Option<f64> {
    if thresholds.is_empty() {
        return None;
    }
    let mut t = thresholds.to_vec();
    t.sort_by(|a, b| a.total_cmp(b));
    let n = t.len();
    Some(if n % 2 == 1 {
        t[n / 2]
    } else {
        let a = t[n / 2 - 1];
        let b = t[n / 2];
        if (a - baseline).abs() <= (b - baseline).abs() { a } else { b }
    })
}

/// Plateau = run kontigu terpanjang yang lolos tiga syarat sekaligus:
/// blok bar FIT < 87%, PF OOS tidak lebih rendah dari baseline, dan G-09 pada ekspektansi FIT tetangga.
/// Ambang yang dilaporkan adalah median run itu, bukan titik PF tertinggi.
pub fn select_plateau(points: &[SweepPoint], baseline_threshold: f64, baseline_oos_pf: f64) -> PlateauChoice {
    let n = points.len();
    let mut mask = vec![false; n];
    for i in 0..n {
        if !(points[i].fit_block < 0.87) || !pf_not_lower(points[i].oos_pf, baseline_oos_pf) {
            continue;
        }
        let mut neigh = Vec::new();
        if i > 0 {
            neigh.push(points[i - 1].fit_e);
        }
        if i + 1 < n {
            neigh.push(points[i + 1].fit_e);
        }
        mask[i] = plateau_gate(points[i].fit_e, &neigh).pass;
    }
    let mut runs: Vec<Vec<usize>> = Vec::new();
    let mut cur = Vec::new();
    for (i, ok) in mask.iter().copied().enumerate() {
        if ok {
            cur.push(i);
        } else if !cur.is_empty() {
            runs.push(std::mem::take(&mut cur));
        }
    }
    if !cur.is_empty() {
        runs.push(cur);
    }
    let contains_base = |run: &[usize]| run.iter().any(|&i| (points[i].threshold - baseline_threshold).abs() < 1e-9);
    let mut best: Option<Vec<usize>> = None;
    for run in runs {
        let take = match &best {
            None => true,
            Some(b) if run.len() != b.len() => run.len() > b.len(),
            Some(b) => {
                let (rb, bb) = (contains_base(&run), contains_base(b));
                if rb != bb {
                    rb
                } else {
                    let rt = adopt_threshold(&run.iter().map(|&i| points[i].threshold).collect::<Vec<_>>(), baseline_threshold).unwrap_or(baseline_threshold);
                    let bt = adopt_threshold(&b.iter().map(|&i| points[i].threshold).collect::<Vec<_>>(), baseline_threshold).unwrap_or(baseline_threshold);
                    (rt - baseline_threshold).abs() < (bt - baseline_threshold).abs()
                }
            }
        };
        if take {
            best = Some(run);
        }
    }
    let run = best.unwrap_or_default();
    let admissible: Vec<usize> = mask.iter().enumerate().filter(|(_, ok)| **ok).map(|(i, _)| i).collect();
    let adopted_threshold = adopt_threshold(&run.iter().map(|&i| points[i].threshold).collect::<Vec<_>>(), baseline_threshold);
    PlateauChoice { admissible, run, adopted_threshold }
}

pub fn g02_pass(trades: usize, min_trades: usize) -> bool {
    trades >= min_trades
}
pub fn g03_pass(expectancy_r: f64) -> bool {
    expectancy_r >= 0.10
}
pub fn g04_pass(pf: f64) -> bool {
    pf >= 1.25
}
pub fn g05_pass(expectancy_r: f64, pf: f64) -> bool {
    expectancy_r >= 0.03 && pf >= 1.10
}
pub fn g07_pass(p95_max_dd_r: f64, p_below_floor: f64) -> bool {
    p95_max_dd_r <= 15.0 && p_below_floor <= 0.05
}
pub fn g08_pass(dsr_value: f64) -> bool {
    dsr_value >= 0.95
}
pub fn g10_pass(net_usd: f64, max_symbol_share: f64, max_month_share: f64) -> bool {
    net_usd > 0.0 && max_symbol_share <= 0.40 && max_month_share <= 0.35
}

/// G-11: ekspektansi > 0 dan PF ≥ 1,10. Pemanggil yang menjalankan holdout; fungsi ini tidak membuka data.
pub fn g11_pass(expectancy_r: f64, pf: f64) -> bool {
    expectancy_r > 0.0 && pf >= 1.10
}

/// Ukuran path Monte Carlo = jumlah trade setara 1 tahun (Rules G-07).
pub fn trades_per_year(n_trades: usize, span_ms: i64) -> usize {
    const YEAR_MS: f64 = 365.25 * 24.0 * 3_600_000.0;
    if n_trades == 0 || span_ms <= 0 {
        return n_trades.max(1);
    }
    let years = span_ms as f64 / YEAR_MS;
    if years <= 0.0 {
        return n_trades;
    }
    ((n_trades as f64) / years).round().max(1.0) as usize
}

/// Baris ledger yang benar-benar memuat strategi ini. Baris strategi lain tidak dihitung.
pub fn count_strategy_trials(ledger: &str, strategy: &str) -> usize {
    ledger.lines().filter(|line| line_has_strategy(line, strategy)).count()
}

fn line_has_strategy(line: &str, strategy: &str) -> bool {
    let line = line.trim();
    if line.is_empty() {
        return false;
    }
    let Ok(v) = serde_json::from_str::<Value>(line) else {
        return false;
    };
    v.get("strategies").and_then(|x| x.as_array()).is_some_and(|a| a.iter().any(|s| s.as_str() == Some(strategy)))
}

/// N = trial tercatat untuk strategi ini (ledger lama + run sesi ini, tanpa dobel) + 40 trial `apex_legacy`.
pub fn honest_n_trials(prior_ledger: usize, session_runs: usize) -> usize {
    prior_ledger.saturating_add(session_runs).saturating_add(40)
}

/// Porsi bar `CHOP_NO_TRADE`.
pub fn chop_block_rate(bars: &BTreeMap<String, u64>) -> f64 {
    let total: u64 = bars.values().sum();
    if total == 0 {
        return 0.0;
    }
    bars.get("CHOP_NO_TRADE").copied().unwrap_or(0) as f64 / total as f64
}

/// Porsi bar yang bukan state boleh-trading.
pub fn non_trade_rate(bars: &BTreeMap<String, u64>) -> f64 {
    let total: u64 = bars.values().sum();
    if total == 0 {
        return 0.0;
    }
    let trade: u64 = ["RISK_OFF", "RISK_ON_ALTS", "BTC_LED"].iter().map(|k| bars.get(*k).copied().unwrap_or(0)).sum();
    1.0 - trade as f64 / total as f64
}

/// Variansi SR antar trial. Kurang dari 2 trial → 0,01 (cadangan terdokumentasi, bukan angka trial palsu).
pub fn dsr_variance(trial_srs: &[f64]) -> f64 {
    let v = sample_variance(trial_srs);
    if trial_srs.len() >= 2 && v > 0.0 {
        v
    } else {
        0.01
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn anchored_six_cover_the_window() {
        let folds = anchored_folds(0, 700, 6);
        assert_eq!(folds.len(), 6);
        assert_eq!(folds[0].train_start, 0);
        assert_eq!(folds[0].test_start, 100);
        assert_eq!(folds[0].test_end, 200);
        assert_eq!(folds[5].train_end, 600);
        assert_eq!(folds[5].test_start, 600);
        assert_eq!(folds[5].test_end, 700);
        for w in folds.windows(2) {
            assert_eq!(w[0].test_end, w[1].test_start);
        }
    }

    #[test]
    fn walk_forward_and_plateau_rules() {
        assert!(walk_forward_gate(&[0.1, 0.2, -0.1, 0.05, 0.01, -0.14]).pass);
        assert!(!walk_forward_gate(&[0.1, 0.2, -0.1, -0.01, -0.02, 0.2]).pass);
        assert!(!walk_forward_gate(&[0.2, -0.2, 0.2, 0.2, 0.2, 0.2]).pass);
        let ok = plateau_gate(0.10, &[0.08, 0.09, 0.11]);
        assert!(ok.pass);
        assert!(!plateau_gate(0.20, &[0.01, 0.02]).pass); // median 0.015 < 0.10
        assert!(!plateau_gate(0.10, &[]).pass);
    }

    #[test]
    fn plateau_picks_median_not_best_pf() {
        let points = vec![
            SweepPoint { threshold: 10.0, fit_block: 0.50, oos_pf: 1.30, fit_e: 0.10 },
            SweepPoint { threshold: 15.0, fit_block: 0.55, oos_pf: 1.28, fit_e: 0.11 },
            SweepPoint { threshold: 20.0, fit_block: 0.60, oos_pf: 1.25, fit_e: 0.10 },
            SweepPoint { threshold: 25.0, fit_block: 0.70, oos_pf: 1.22, fit_e: 0.09 },
            SweepPoint { threshold: 30.0, fit_block: 0.92, oos_pf: 1.40, fit_e: 0.20 },
        ];
        let c = select_plateau(&points, 20.0, 1.25);
        assert_eq!(c.admissible, vec![0, 1, 2]);
        assert_eq!(c.adopted_threshold, Some(15.0));
    }

    #[test]
    fn gates_and_honest_trials() {
        assert!(g11_pass(0.01, 1.10));
        assert!(!g11_pass(0.0, 1.50));
        assert!(!g11_pass(0.2, 1.09));
        assert!(g05_pass(0.03, 1.10));
        assert!(!g05_pass(0.02, 1.20));
        assert!(g07_pass(15.0, 0.05));
        assert!(!g07_pass(15.1, 0.01));
        assert_eq!(trades_per_year(160, (365.25 * 24.0 * 3_600_000.0) as i64), 160);
        let year = (365.25 * 24.0 * 3_600_000.0) as i64;
        assert_eq!(trades_per_year(100, year * 2), 50);
        let ledger = "{\"strategies\":[\"S1\"]}\n{\"strategies\":[\"S2\",\"S3\"]}\nnot json\n{\"strategies\":[\"S1\",\"S5\"]}\n";
        assert_eq!(count_strategy_trials(ledger, "S1"), 2);
        assert_eq!(count_strategy_trials(ledger, "S2"), 1);
        assert_eq!(count_strategy_trials(ledger, "S4"), 0);
        assert_eq!(honest_n_trials(2, 3), 45);
        let mut bars = BTreeMap::new();
        bars.insert("CHOP_NO_TRADE".into(), 87);
        bars.insert("RISK_ON_ALTS".into(), 13);
        assert!((chop_block_rate(&bars) - 0.87).abs() < 1e-12);
        assert!((non_trade_rate(&bars) - 0.87).abs() < 1e-12);
        assert!((dsr_variance(&[0.2, 0.4]) - 0.02).abs() < 1e-12);
        assert_eq!(dsr_variance(&[0.2]), 0.01);
    }
}
