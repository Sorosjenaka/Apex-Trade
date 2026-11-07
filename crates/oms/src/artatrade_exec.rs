//! Port 1:1 sistem Risk Sizing, SL/TP Menu, Progressive Policy, 21-Agent Mesh & Council,
//! Directional Battle Arena, Trend-Reversal Exit, Binance IP Rate-Limit Guard, Mirror Reconciler,
//! serta Learning/Probability/Payoff/5-Whys Forensics ArtaTrade
//! — khusus mode SCALP (tanpa strategi SWING `RSW`).
#![allow(
    clippy::neg_cmp_op_on_partial_ord,
    clippy::too_many_arguments,
    clippy::unnecessary_sort_by
)]

use std::collections::HashMap;

use apex_core::types::Side;
use serde::{Deserialize, Serialize};

fn side_str(side: Side) -> &'static str {
    match side {
        Side::Long => "LONG",
        Side::Short => "SHORT",
    }
}

// ─────────────────────── 1. Structural Risk Sizing, Fixed Risk Levels & SL/TP Menu ───────────────────────

pub const DEFAULT_RISK_USD: f64 = 1.00;
pub const SCALP_TP1_R: f64 = 1.70;
pub const SCALP_SL_BAND_MIN: f64 = 0.02; // 2%
pub const SCALP_SL_BAND_MAX: f64 = 0.04; // 4%
pub const SCALP_SL_BUFFER_ATR: f64 = 0.46;
pub const SCALP_MAX_MARGIN_USD: f64 = 4.80;
pub const MAX_LEVERAGE_SYSTEM: u32 = 25;

fn quantize_step_floor(val: f64, step: f64) -> f64 {
    if !(step > 0.0) {
        return val;
    }
    let steps = (val / step + 1e-9).floor();
    ((steps * step) * 1e8).round() / 1e8
}

fn round_to_tick(val: f64, tick: f64) -> f64 {
    if !(tick > 0.0) {
        return val;
    }
    let steps = (val / tick).round();
    ((steps * tick) * 1e8).round() / 1e8
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct FixedRiskLevels {
    pub sl_price: f64,
    pub tp_price: f64,
    pub risk_usd: f64,
    pub tp_usd: f64,
}

/// Penentuan level SL & TP presisi dari `qty` final agar risiko tepat `<= $1.00` dan profit `1.7R` (`$1.70`)
/// (`riskEngine.ts:fixedRiskLevels`).
pub fn fixed_risk_levels(
    entry_price: f64,
    qty: f64,
    side: Side,
    tick_size: f64,
    risk_usd: f64,
    tp_usd: f64,
) -> Option<FixedRiskLevels> {
    if !(entry_price > 0.0)
        || !(qty > 0.0)
        || !(tick_size > 0.0)
        || !(risk_usd > 0.0)
        || !(tp_usd > 0.0)
    {
        return None;
    }
    let mut sl_dist = quantize_step_floor(risk_usd / qty, tick_size);
    let tp_dist = round_to_tick(tp_usd / qty, tick_size);
    if !(sl_dist > 0.0) || !(tp_dist > 0.0) {
        return None;
    }
    let calc_sl = |dist: f64| match side {
        Side::Long => round_to_tick(entry_price - dist, tick_size),
        Side::Short => round_to_tick(entry_price + dist, tick_size),
    };
    let mut sl_price = calc_sl(sl_dist);
    let mut eff_risk = ((qty * (entry_price - sl_price).abs()) * 1e6).round() / 1e6;
    let mut guard = 0;
    while eff_risk > risk_usd + 1e-9 && sl_dist > tick_size && guard < 5 {
        guard += 1;
        sl_dist = ((sl_dist - tick_size) * 1e8).round() / 1e8;
        sl_price = calc_sl(sl_dist);
        eff_risk = ((qty * (entry_price - sl_price).abs()) * 1e6).round() / 1e6;
    }
    if eff_risk > risk_usd + 1e-6 {
        return None;
    }
    let tp_price = match side {
        Side::Long => round_to_tick(entry_price + tp_dist, tick_size),
        Side::Short => round_to_tick(entry_price - tp_dist, tick_size),
    };
    let eff_tp = ((qty * (tp_price - entry_price).abs()) * 1e6).round() / 1e6;
    Some(FixedRiskLevels {
        sl_price,
        tp_price,
        risk_usd: eff_risk,
        tp_usd: eff_tp,
    })
}

#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct StructuralRiskSizingResult {
    pub ok: bool,
    pub qty: f64,
    pub leverage: u32,
    pub margin_usd: f64,
    pub notional_usd: f64,
    pub risk_at_structure_usd: f64,
    pub reason: Option<&'static str>,
}

/// Sizing berbasis risiko struktur `$1.00` dengan plafon margin `$4.80` (`SCALP_MAX_MARGIN_USD`) dan leverage
/// otomatis hingga `25x` (`riskEngine.ts:structuralRiskSizing`).
pub fn structural_risk_sizing(
    entry_price: f64,
    structural_sl_price: f64,
    step_size: f64,
    target_risk_usd: f64,
    margin_cap_usd: f64,
    max_leverage: u32,
) -> StructuralRiskSizingResult {
    let struct_dist = (entry_price - structural_sl_price).abs();
    if !(entry_price > 0.0) || !(struct_dist > 0.0) || !(step_size > 0.0) || !(margin_cap_usd > 0.0)
    {
        return StructuralRiskSizingResult {
            ok: false,
            qty: 0.0,
            leverage: 1,
            margin_usd: 0.0,
            notional_usd: 0.0,
            risk_at_structure_usd: 0.0,
            reason: Some("SIZING_INPUT_INVALID"),
        };
    }
    let raw_qty = target_risk_usd / struct_dist;
    let mut qty = quantize_step_floor(raw_qty, step_size);
    if !(qty > 0.0) {
        return StructuralRiskSizingResult {
            ok: false,
            qty: 0.0,
            leverage: 1,
            margin_usd: 0.0,
            notional_usd: 0.0,
            risk_at_structure_usd: 0.0,
            reason: Some("QTY_BELOW_STEP_SIZE"),
        };
    }
    let lev_cap = max_leverage.clamp(1, MAX_LEVERAGE_SYSTEM);
    let mut notional = qty * entry_price;
    let needed_lev = ((notional / margin_cap_usd).ceil() as u32).max(1);
    let leverage = needed_lev.min(lev_cap);
    if notional / (leverage as f64) > margin_cap_usd + 1e-6 {
        let max_notional = margin_cap_usd * (leverage as f64);
        qty = quantize_step_floor(max_notional / entry_price, step_size);
        notional = qty * entry_price;
    }
    if !(qty > 0.0) {
        return StructuralRiskSizingResult {
            ok: false,
            qty: 0.0,
            leverage,
            margin_usd: 0.0,
            notional_usd: 0.0,
            risk_at_structure_usd: 0.0,
            reason: Some("MARGIN_CAP_QTY_ZERO"),
        };
    }
    let margin_usd = ((notional / leverage as f64) * 1e4).round() / 1e4;
    let risk_at_structure_usd = ((qty * struct_dist) * 1e4).round() / 1e4;
    StructuralRiskSizingResult {
        ok: true,
        qty,
        leverage,
        margin_usd,
        notional_usd: (notional * 1e4).round() / 1e4,
        risk_at_structure_usd,
        reason: None,
    }
}

#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct SlMenuVariant {
    pub id: &'static str, // "A" | "B"
    pub label: &'static str,
    pub sl_price: f64,
    pub sl_pct: f64,
    pub r_distance: f64,
    pub basis: &'static str,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct TpMenuVariant {
    pub id: &'static str, // "A"
    pub label: &'static str,
    pub r: f64,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct SlTpMenu {
    pub entry: f64,
    pub side: Side,
    pub mode: &'static str,
    pub sl: Vec<SlMenuVariant>,
    pub tp: Vec<TpMenuVariant>,
    pub default_sl_id: &'static str,
    pub default_tp_id: &'static str,
}

/// Bangun Menu SL/TP deterministik (`setupStrategies.ts:buildSlTpMenu`) untuk mode SCALP:
/// - `A`: SL struktural di-clamp ke band `[2%, 4%]`
/// - `B`: SL struktural + buffer `0.46 * ATR` di-clamp ke band `[2%, 4%]`
/// - `TP A`: `1.7R`
pub fn build_sl_tp_menu_scalp(
    entry: f64,
    structural_sl: f64,
    side: Side,
    atr_pct: f64,
) -> Option<SlTpMenu> {
    if !(entry > 0.0) || !entry.is_finite() || !(structural_sl > 0.0) {
        return None;
    }
    let raw_pct =
        ((entry - structural_sl).abs() / entry).clamp(SCALP_SL_BAND_MIN, SCALP_SL_BAND_MAX);
    let sl_a = match side {
        Side::Long => entry * (1.0 - raw_pct),
        Side::Short => entry * (1.0 + raw_pct),
    };
    let r_a = (entry - sl_a).abs();
    let mut sl_variants = vec![SlMenuVariant {
        id: "A",
        label: "struktural",
        sl_price: (sl_a * 1e8).round() / 1e8,
        sl_pct: (raw_pct * 1e6).round() / 1e6,
        r_distance: (r_a * 1e8).round() / 1e8,
        basis: "STRUCTURAL",
    }];

    let eff_atr_pct = if atr_pct > 0.0 && atr_pct.is_finite() {
        atr_pct
    } else {
        0.004
    };
    let buf_pct =
        (raw_pct + SCALP_SL_BUFFER_ATR * eff_atr_pct).clamp(SCALP_SL_BAND_MIN, SCALP_SL_BAND_MAX);
    if buf_pct > raw_pct + 1e-6 {
        let sl_b = match side {
            Side::Long => entry * (1.0 - buf_pct),
            Side::Short => entry * (1.0 + buf_pct),
        };
        let r_b = (entry - sl_b).abs();
        sl_variants.push(SlMenuVariant {
            id: "B",
            label: "struktural + buffer ATR",
            sl_price: (sl_b * 1e8).round() / 1e8,
            sl_pct: (buf_pct * 1e6).round() / 1e6,
            r_distance: (r_b * 1e8).round() / 1e8,
            basis: "STRUCTURAL_ATR_BUFFER",
        });
    }

    Some(SlTpMenu {
        entry,
        side,
        mode: "SCALP",
        sl: sl_variants,
        tp: vec![TpMenuVariant {
            id: "A",
            label: "1.7R",
            r: SCALP_TP1_R,
        }],
        default_sl_id: "A",
        default_tp_id: "A",
    })
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct LiveLadderPlan {
    pub tp1_qty: f64,
    pub tp2_qty: f64,
    pub runner_qty: f64,
    pub tp_max_price: Option<f64>,
}

/// Rencana ladder proyeksi (`liveLadder.ts:planLiveLadder`) `40% / 30% / 30%` untuk overlay UI.
pub fn plan_live_ladder(
    qty: f64,
    step_size: f64,
    entry_price: f64,
    tp1_price: f64,
    side: Side,
) -> Option<LiveLadderPlan> {
    if !(qty > 0.0) || !(entry_price > 0.0) || !(tp1_price > 0.0) {
        return None;
    }
    let beyond = match side {
        Side::Long => tp1_price > entry_price,
        Side::Short => tp1_price < entry_price,
    };
    if !beyond {
        return None;
    }
    let tp1_qty = quantize_step_floor(qty * 0.4, step_size);
    let tp2_qty = quantize_step_floor(qty * 0.3, step_size);
    let runner_qty = ((qty - tp1_qty - tp2_qty) * 1e8).round() / 1e8;
    if !(tp1_qty > 0.0) || !(tp2_qty > 0.0) || !(runner_qty > 0.0) {
        return None;
    }
    Some(LiveLadderPlan {
        tp1_qty,
        tp2_qty,
        runner_qty,
        tp_max_price: Some(entry_price + (tp1_price - entry_price) * 3.0),
    })
}

// ─────────────────────── 2. Progressive Exploration Curriculum (progressivePolicy.ts) ───────────────────────

#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct ProgressiveThresholds {
    pub level: u8,
    pub min_score: u32,
    pub min_knn: f64,
    pub review_floors: [u32; 4],
    pub max_review_chop: f64,
    pub min_review_adx: f64,
    pub max_strict_chop: f64,
    pub min_strict_adx: f64,
    pub allow_sideways_review: bool,
    pub require_strict_plan: bool,
    pub allow_macro_override: bool,
    pub allow_exploration_go: bool,
    pub exploration_min_confidence: u32,
    pub directive: &'static str,
}

pub fn progressive_thresholds_for(level: u8) -> ProgressiveThresholds {
    match level {
        0 => ProgressiveThresholds {
            level: 0,
            min_score: 45,
            min_knn: 0.0,
            review_floors: [40, 42, 44, 45],
            max_review_chop: 75.0,
            min_review_adx: 10.0,
            max_strict_chop: 70.0,
            min_strict_adx: 12.0,
            allow_sideways_review: true,
            require_strict_plan: true,
            allow_macro_override: true,
            allow_exploration_go: true,
            exploration_min_confidence: 55,
            directive:
                "MODE EKSPLORASI L0: nilai SEMUA kandidat MTF segar + volume OK. Ambang lunak 45.",
        },
        1 => ProgressiveThresholds {
            level: 1,
            min_score: 50,
            min_knn: 0.0,
            review_floors: [45, 47, 49, 50],
            max_review_chop: 70.0,
            min_review_adx: 12.0,
            max_strict_chop: 65.0,
            min_strict_adx: 14.0,
            allow_sideways_review: true,
            require_strict_plan: true,
            allow_macro_override: true,
            allow_exploration_go: true,
            exploration_min_confidence: 60,
            directive:
                "MODE EKSPLORASI L1: ambang lunak 50. Pilih yang konfluensinya paling meyakinkan.",
        },
        2 => ProgressiveThresholds {
            level: 2,
            min_score: 55,
            min_knn: 0.40,
            review_floors: [50, 52, 54, 55],
            max_review_chop: 65.0,
            min_review_adx: 15.0,
            max_strict_chop: 62.0,
            min_strict_adx: 16.0,
            allow_sideways_review: true,
            require_strict_plan: true,
            allow_macro_override: false,
            allow_exploration_go: true,
            exploration_min_confidence: 65,
            directive:
                "MODE TRANSISI L2: ambang 55. Mulai selektif; sideways hanya bila bukti tren kuat.",
        },
        3 => ProgressiveThresholds {
            level: 3,
            min_score: 60,
            min_knn: 0.50,
            review_floors: [55, 57, 59, 60],
            max_review_chop: 62.0,
            min_review_adx: 18.0,
            max_strict_chop: 60.0,
            min_strict_adx: 18.0,
            allow_sideways_review: false,
            require_strict_plan: true,
            allow_macro_override: false,
            allow_exploration_go: false,
            exploration_min_confidence: 70,
            directive: "MODE PENGETATAN L3: ambang 60. Kualitas di atas kuantitas.",
        },
        4 => ProgressiveThresholds {
            level: 4,
            min_score: 64,
            min_knn: 0.55,
            review_floors: [58, 60, 62, 64],
            max_review_chop: 60.0,
            min_review_adx: 20.0,
            max_strict_chop: 58.3,
            min_strict_adx: 20.0,
            allow_sideways_review: false,
            require_strict_plan: true,
            allow_macro_override: false,
            allow_exploration_go: false,
            exploration_min_confidence: 75,
            directive: "MODE HAMPIR DISIPLIN L4: ambang 64. Hanya setup rapi yang GO.",
        },
        _ => ProgressiveThresholds {
            level: 5,
            min_score: 68,
            min_knn: 0.55,
            review_floors: [55, 60, 64, 68],
            max_review_chop: 65.0,
            min_review_adx: 15.0,
            max_strict_chop: 58.3,
            min_strict_adx: 20.0,
            allow_sideways_review: false,
            require_strict_plan: true,
            allow_macro_override: false,
            allow_exploration_go: false,
            exploration_min_confidence: 80,
            directive: "MODE DISIPLIN L5: ambang 68 & KNN 55%. Slot kosong bukan alasan override.",
        },
    }
}

pub fn resolve_auto_progressive_level(
    verified_trades: usize,
    winrate: Option<f64>,
    expectancy_usd: Option<f64>,
) -> u8 {
    let by_count: u8 = if verified_trades >= 1000 {
        5
    } else if verified_trades >= 500 {
        4
    } else if verified_trades >= 200 {
        3
    } else if verified_trades >= 100 {
        2
    } else if verified_trades >= 30 {
        1
    } else {
        0
    };
    let Some(wr) = winrate.filter(|v| v.is_finite()) else {
        return 0;
    };
    let by_quality: u8 = match expectancy_usd.filter(|v| v.is_finite()) {
        None => {
            if wr >= 0.57 {
                2
            } else if wr >= 0.52 {
                1
            } else {
                0
            }
        }
        Some(ex) => {
            if wr >= 0.57 && ex > 0.10 {
                5
            } else if wr >= 0.55 && ex > 0.05 {
                4
            } else if wr >= 0.50 && ex > 0.0 {
                3
            } else if wr >= 0.47 && ex > -0.05 {
                2
            } else if wr >= 0.42 {
                1
            } else {
                0
            }
        }
    };
    by_count.min(by_quality)
}

// ─────────────────────── 3. 21-Agent Mesh Catalog, 3-Agent Council & Battle Arena ───────────────────────

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct AgentMeshNode {
    pub id: &'static str,
    pub name: &'static str,
    pub role: &'static str,
    pub status: &'static str, // "ACTIVE" | "IDLE" | "STANDBY"
}

pub const ALL_21_AGENTS: &[AgentMeshNode] = &[
    AgentMeshNode {
        id: "macro-scout",
        name: "Vega",
        role: "Pengamat matriks 6 kondisi BTC.D x BTC Price sebelum entry alt",
        status: "ACTIVE",
    },
    AgentMeshNode {
        id: "universe-curator",
        name: "Sena",
        role: "Kurator Universe Simbol & Blacklist Crypto-Only",
        status: "ACTIVE",
    },
    AgentMeshNode {
        id: "regime-analyst",
        name: "Rhea",
        role: "Analis Regime Pasar & Volatilitas (ZLEMA + Shield)",
        status: "ACTIVE",
    },
    AgentMeshNode {
        id: "scanner-supervisor",
        name: "Bima",
        role: "Koordinator Worker Scanner Pool (7-Gate SCALP)",
        status: "ACTIVE",
    },
    AgentMeshNode {
        id: "setup-judge",
        name: "Arka",
        role: "Hakim Penilai Setup, Konfluensi & Pilihan SL/TP Menu",
        status: "ACTIVE",
    },
    AgentMeshNode {
        id: "risk-officer",
        name: "Wardha",
        role: "Perwira Kepatuhan Risiko $1 & Margin Cap $4.80",
        status: "ACTIVE",
    },
    AgentMeshNode {
        id: "trade-supervisor",
        name: "Nadira",
        role: "Supervisor Eksekusi TP/SL/Timeout & Trend-Flip Exit",
        status: "ACTIVE",
    },
    AgentMeshNode {
        id: "watchdog",
        name: "Raksa",
        role: "Pengawas Infrastruktur, Rate-Limit & Failover Host",
        status: "ACTIVE",
    },
    AgentMeshNode {
        id: "post-mortem",
        name: "Sesha",
        role: "Evaluator Bedah Kasus Loss & Klasifikasi Mistake",
        status: "IDLE",
    },
    AgentMeshNode {
        id: "strategy-tuner",
        name: "Laras",
        role: "Peneliti Penyetel Parameter Bounded <=10%",
        status: "STANDBY",
    },
    AgentMeshNode {
        id: "backtest-auditor",
        name: "Varuna",
        role: "Auditor Sertifikasi Gate Backtest & CPCV",
        status: "STANDBY",
    },
    AgentMeshNode {
        id: "narrator",
        name: "Kirana",
        role: "Pencerita Ringkasan Tim & Buletin Harian",
        status: "STANDBY",
    },
    AgentMeshNode {
        id: "trade-evaluator",
        name: "Mira",
        role: "Evaluator Hasil Trade & Feedback Loop (5-Why)",
        status: "IDLE",
    },
    AgentMeshNode {
        id: "market-context",
        name: "Sari",
        role: "Analis Konteks Pasar Cepat (Ringkasan Pasar)",
        status: "ACTIVE",
    },
    AgentMeshNode {
        id: "signal-analyst",
        name: "Tala",
        role: "Analis Sinyal Cepat (Evaluasi Sinyal 24 Jam)",
        status: "ACTIVE",
    },
    AgentMeshNode {
        id: "funnel-analyst",
        name: "Giri",
        role: "Analis Funnel Cepat (Kualitas Gate & Penolakan)",
        status: "ACTIVE",
    },
    AgentMeshNode {
        id: "thesis-long",
        name: "Arya",
        role: "Advokat Tesis LONG di Arena Tesis",
        status: "STANDBY",
    },
    AgentMeshNode {
        id: "thesis-short",
        name: "Bayu",
        role: "Advokat Tesis SHORT di Arena Tesis",
        status: "STANDBY",
    },
    AgentMeshNode {
        id: "thesis-referee",
        name: "Ratih",
        role: "Juri Kualitas Tesis Arena (Anti-Looping)",
        status: "STANDBY",
    },
    AgentMeshNode {
        id: "drex",
        name: "Drex",
        role: "Agen Analisis Visual & Shadow Judge",
        status: "ACTIVE",
    },
    AgentMeshNode {
        id: "panel-ai",
        name: "Panel AI",
        role: "Sintesis 5 Panel Mikrostruktur & Orderflow",
        status: "ACTIVE",
    },
];

#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct CouncilVote {
    pub agent_id: &'static str,
    pub agent_name: &'static str,
    pub stance: &'static str, // "GO" | "HOLD" | "NO_GO"
    pub confidence_score: f64,
    pub reasoning: String,
}

pub const DEFAULT_COMP_SCORE: u32 = 72;
pub const DEFAULT_KNN_PROB: f64 = 0.55;
pub const DEFAULT_ATR_PCT: f64 = 0.008;
pub const DEFAULT_SL_PCT: f64 = 0.025;
pub const DEFAULT_ARENA_TREND_SCORE: f64 = 72.0;

#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct CouncilDecision {
    /// Dewan benar-benar memblokir eksekusi bila `final_action != "EXECUTE"`.
    pub enforced: bool,
    pub final_action: &'static str, // "EXECUTE" | "REJECT"
    pub consensus_level: f64,
    pub votes: Vec<CouncilVote>,
    pub summary_narrative: String,
    pub sl_choice: &'static str,
    pub tp_choice: &'static str,
    /// Daftar nama input yang jatuh ke nilai default deterministik.
    pub inputs_defaulted: Vec<&'static str>,
}

/// Sidang Dewan 3 Agen Pra-Entry (`Vega` macro-scout, `Arka` setup-judge, `Wardha` risk-officer)
/// — aturan deterministik (`if/else`), bukan agen otonom:
/// - Hard veto bila ada 1 vote `NO_GO` dengan `confidence > 70`
/// - `EXECUTE` bila `goVotes >= 2` dan `avgConfidence >= 60.0`
pub fn convene_deterministic_council(
    symbol: &str,
    side: Side,
    macro_allowed: bool,
    macro_state: &str,
    composite_score: u32,
    knn_prob: Option<f64>,
    sl_pct: f64,
    sl_atr_ratio: f64,
    cvd_aligned: bool,
) -> CouncilDecision {
    // 1. Vega (macro-scout)
    let (vega_stance, vega_conf, vega_reason) = if !macro_allowed {
        (
            "NO_GO",
            82.0,
            format!(
                "Makro {} memblokir {} pada altcoin {}.",
                macro_state,
                side_str(side),
                symbol
            ),
        )
    } else {
        (
            "GO",
            76.0,
            format!(
                "Matriks makro {} mengizinkan eksposur {} {}.",
                macro_state,
                side_str(side),
                symbol
            ),
        )
    };

    // 2. Arka (setup-judge)
    let knn_val = knn_prob.unwrap_or(DEFAULT_KNN_PROB);
    let (arka_stance, arka_conf, arka_reason) =
        if composite_score >= 60 && cvd_aligned && knn_val >= 0.45 {
            let c = (composite_score as f64 * 0.7 + knn_val * 30.0).clamp(62.0, 95.0);
            (
                "GO",
                c,
                format!(
                    "Skor konfluensi {}/100, KNN {:.0}%, dan CVD searah {}.",
                    composite_score,
                    knn_val * 100.0,
                    side_str(side)
                ),
            )
        } else if composite_score < 50 || !cvd_aligned {
            (
                "HOLD",
                55.0,
                format!(
                    "Skor {}/100 atau CVD belum konfluen penuh (KNN {:.0}%).",
                    composite_score,
                    knn_val * 100.0
                ),
            )
        } else {
            (
                "GO",
                64.0,
                format!(
                    "Setup memenuhi lantai konfluensi ({}/100, KNN {:.0}%).",
                    composite_score,
                    knn_val * 100.0
                ),
            )
        };

    // 3. Wardha (risk-officer) — aturan 5 Nov 2026: band SL SCALP 2%-4% normal walau rasio ATR 0.5x-1.5x;
    // hanya veto bila rasio < 0.50x atau SL di luar band [1.2%, 4.2%].
    let (wardha_stance, wardha_conf, wardha_reason, sl_choice) =
        if sl_atr_ratio > 0.0 && sl_atr_ratio < 0.50 {
            (
                "NO_GO",
                85.0,
                format!(
                    "Rasio SL/ATR {:.2}x < 0.50x berada di dalam noise intrabar.",
                    sl_atr_ratio
                ),
                "B",
            )
        } else if !(0.012..=0.045).contains(&sl_pct) {
            (
                "HOLD",
                65.0,
                format!(
                    "Jarak SL {:.2}% berada di luar pita optimal SCALP.",
                    sl_pct * 100.0
                ),
                "A",
            )
        } else {
            let choice = if sl_atr_ratio < 0.85 { "B" } else { "A" };
            (
                "GO",
                80.0,
                format!(
                    "Risiko terkunci $1.00, jarak SL {:.2}% ({:.2}x ATR), varian MENU_SL_{}.",
                    sl_pct * 100.0,
                    sl_atr_ratio,
                    choice
                ),
                choice,
            )
        };

    let votes = vec![
        CouncilVote {
            agent_id: "macro-scout",
            agent_name: "Vega",
            stance: vega_stance,
            confidence_score: vega_conf,
            reasoning: vega_reason,
        },
        CouncilVote {
            agent_id: "setup-judge",
            agent_name: "Arka",
            stance: arka_stance,
            confidence_score: arka_conf,
            reasoning: arka_reason,
        },
        CouncilVote {
            agent_id: "risk-officer",
            agent_name: "Wardha",
            stance: wardha_stance,
            confidence_score: wardha_conf,
            reasoning: wardha_reason,
        },
    ];

    let has_hard_veto = votes
        .iter()
        .any(|v| v.stance == "NO_GO" && v.confidence_score > 70.0);
    let avg_conf = votes.iter().map(|v| v.confidence_score).sum::<f64>() / votes.len() as f64;
    let go_votes = votes.iter().filter(|v| v.stance == "GO").count();
    let hold_votes = votes.iter().filter(|v| v.stance == "HOLD").count();
    let nogo_votes = votes.iter().filter(|v| v.stance == "NO_GO").count();

    let (final_action, summary) = if has_hard_veto {
        let vetoes: Vec<String> = votes
            .iter()
            .filter(|v| v.stance == "NO_GO")
            .map(|v| format!("{}: {}", v.agent_name, v.reasoning))
            .collect();
        (
            "REJECT",
            format!("VETO AKTIF. Alasan: {}", vetoes.join("; ")),
        )
    } else if go_votes >= 2 && avg_conf >= 60.0 {
        (
            "EXECUTE",
            format!(
                "Konsensus GO ({}/3). Rata-rata keyakinan: {:.1}% (ambang 60%).",
                go_votes, avg_conf
            ),
        )
    } else {
        (
            "REJECT",
            format!(
                "Terlalu banyak keraguan ({} GO, {} HOLD, {} NO_GO). Rata-rata keyakinan: {:.1}% (ambang 60%).",
                go_votes, hold_votes, nogo_votes, avg_conf
            ),
        )
    };

    CouncilDecision {
        enforced: true,
        final_action,
        consensus_level: (avg_conf * 10.0).round() / 10.0,
        votes,
        summary_narrative: summary,
        sl_choice,
        tp_choice: "A",
        inputs_defaulted: if knn_prob.is_none() {
            vec!["knn_prob"]
        } else {
            Vec::new()
        },
    }
}

#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct DirectionBattleDecision {
    /// Arena bersifat advisory saja (`enforced = false`); hasilnya dicatat tanpa memveto order.
    pub enforced: bool,
    pub symbol: String,
    pub status: &'static str, // "GO" | "REJECT"
    pub winner: Option<Side>,
    pub margin: i32,
    pub long_score: i32,
    pub short_score: i32,
    pub trend_side: Option<Side>,
    pub reasons: Vec<String>,
}

/// Arena Pertarungan Arah LONG vs SHORT (`directionalBattleArena.ts:runDirectionBattle`).
pub fn run_deterministic_direction_battle(
    symbol: &str,
    trend_score: f64,
    bias_bullish: bool,
    macro_long_ok: bool,
    macro_short_ok: bool,
    cvd_delta: f64,
) -> DirectionBattleDecision {
    let t_side = if trend_score >= 55.0 && bias_bullish {
        Some(Side::Long)
    } else if trend_score >= 55.0 && !bias_bullish {
        Some(Side::Short)
    } else {
        None
    };

    let mut long_score: i32 = 0;
    let mut short_score: i32 = 0;

    if bias_bullish {
        long_score += (trend_score.round() as i32).clamp(40, 90);
        short_score -= 40;
    } else {
        short_score += (trend_score.round() as i32).clamp(40, 90);
        long_score -= 40;
    }
    if macro_long_ok {
        long_score += 65;
    } else {
        long_score -= 70;
    }
    if macro_short_ok {
        short_score += 65;
    } else {
        short_score -= 70;
    }
    if cvd_delta > 0.0 {
        long_score += 60;
        short_score -= 30;
    } else if cvd_delta < 0.0 {
        short_score += 60;
        long_score -= 30;
    }

    let margin = (long_score - short_score).abs();
    let winner = if long_score > short_score {
        Some(Side::Long)
    } else if short_score > long_score {
        Some(Side::Short)
    } else {
        None
    };

    let mut reasons = Vec::new();
    if winner.is_none() {
        reasons.push("NO_DIRECTIONAL_CONSENSUS".into());
    }
    if let (Some(ts), Some(w)) = (t_side, winner) {
        if ts != w {
            reasons.push(format!("TREND_CONTRADICTION_{}", side_str(ts)));
        }
    }
    let macro_ok = match winner {
        Some(Side::Long) => macro_long_ok,
        Some(Side::Short) => macro_short_ok,
        None => false,
    };
    if !macro_ok {
        reasons.push("MACRO_NOT_ALLOWED".into());
    }

    let status = if winner.is_some() && margin >= 120 && reasons.is_empty() {
        "GO"
    } else {
        "REJECT"
    };

    DirectionBattleDecision {
        enforced: false,
        symbol: symbol.to_uppercase(),
        status,
        winner: if status == "GO" { winner } else { None },
        margin,
        long_score,
        short_score,
        trend_side: t_side,
        reasons,
    }
}

/// Masukan mentah (dengan `Option` jujur) untuk evaluasi Dewan + Arena sehingga fallback default tercatat di audit.
pub struct CouncilArenaInputs<'a> {
    pub symbol: &'a str,
    pub side: Side,
    pub entry: f64,
    pub sl: f64,
    pub atr_14: Option<f64>,
    pub macro_allowed: bool,
    pub macro_state: &'a str,
    pub composite_score: Option<u32>,
    pub knn_prob: Option<f64>,
    pub cvd_aligned: bool,
    pub arena_trend_score: Option<f64>,
    pub macro_long_ok: bool,
    pub macro_short_ok: bool,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct CouncilArenaEvaluation {
    pub sl_tp_menu: Option<SlTpMenu>,
    pub council: CouncilDecision,
    pub battle_arena: DirectionBattleDecision,
    pub inputs_defaulted: Vec<&'static str>,
}

/// Evaluasi Dewan (enforced=true) dan Arena (enforced=false) dengan melacak setiap input yang jatuh ke nilai default.
/// Nilai default dipertahankan persis sama dengan kode lama agar keputusan entry 100% identik (behavior-preserving).
pub fn evaluate_council_and_arena(inp: &CouncilArenaInputs<'_>) -> CouncilArenaEvaluation {
    let mut defaulted: Vec<&'static str> = Vec::new();
    let atr_pct = match inp.atr_14.filter(|&a| inp.entry > 0.0 && a > 0.0) {
        Some(a) => a / inp.entry,
        None => {
            defaulted.push("atr_pct");
            DEFAULT_ATR_PCT
        }
    };
    let sl_pct = if inp.entry > 0.0 {
        (inp.entry - inp.sl).abs() / inp.entry
    } else {
        defaulted.push("sl_pct");
        DEFAULT_SL_PCT
    };
    let comp_score = match inp.composite_score {
        Some(s) => s,
        None => {
            defaulted.push("composite_score");
            DEFAULT_COMP_SCORE
        }
    };
    if inp.knn_prob.is_none() {
        defaulted.push("knn_prob");
    }
    let trend_score = match inp.arena_trend_score {
        Some(ts) => ts,
        None => {
            defaulted.push("arena_trend_score");
            DEFAULT_ARENA_TREND_SCORE
        }
    };
    let sl_menu = build_sl_tp_menu_scalp(inp.entry, inp.sl, inp.side, atr_pct);
    let sl_atr_ratio = if atr_pct > 0.0 { sl_pct / atr_pct } else { 1.0 };
    let mut council = convene_deterministic_council(
        inp.symbol,
        inp.side,
        inp.macro_allowed,
        inp.macro_state,
        comp_score,
        inp.knn_prob,
        sl_pct,
        sl_atr_ratio,
        inp.cvd_aligned,
    );
    council.inputs_defaulted = defaulted.clone();
    let battle_arena = run_deterministic_direction_battle(
        inp.symbol,
        trend_score,
        inp.side == Side::Long,
        inp.macro_long_ok,
        inp.macro_short_ok,
        if inp.side == Side::Long { 1.0 } else { -1.0 },
    );
    CouncilArenaEvaluation {
        sl_tp_menu: sl_menu,
        council,
        battle_arena,
        inputs_defaulted: defaulted,
    }
}

// ─────────────────────── 4. Trend-Reversal Exit Engine (trendReversalEngine.ts) ───────────────────────

pub const STRONG_ADX: f64 = 25.0;
pub const MIN_TREND_SCORE_EXIT: f64 = 60.0;
pub const MIN_CONFIDENCE_EXIT: f64 = 0.65;
pub const TREND_FLIP_CONFIRMATION_MIN_MS: i64 = 60_000;
pub const TREND_FLIP_CONFIRMATION_MAX_GAP_MS: i64 = 10 * 60_000;
pub const TREND_FLIP_CONFIRMATION_MIN_GAP_MS: i64 = 30_000;

#[derive(Clone, Debug)]
struct PendingTrendFlip {
    direction_bullish: bool,
    first_at_ms: i64,
    last_at_ms: i64,
    observations: u32,
}

/// Guard konfirmasi 2 pembacaan terpisah waktu (`>= 60s` total, `>= 30s` jeda) sebelum eksekusi
/// Trend-Reversal Exit (`trendReversalEngine.ts:TrendFlipConfirmationGuard`).
#[derive(Clone, Debug, Default)]
pub struct TrendFlipConfirmationGuard {
    pending: HashMap<String, PendingTrendFlip>,
}

impl TrendFlipConfirmationGuard {
    pub fn observe(
        &mut self,
        symbol: &str,
        position_side: Side,
        new_trend_bullish: Option<bool>,
        now_ms: i64,
    ) -> bool {
        let key = format!("{}|{}", symbol.to_uppercase(), side_str(position_side));
        let Some(bullish) = new_trend_bullish else {
            self.pending.remove(&key);
            return false;
        };
        let against = match position_side {
            Side::Long => !bullish,
            Side::Short => bullish,
        };
        if !against {
            self.pending.remove(&key);
            return false;
        }
        if let Some(prev) = self.pending.get_mut(&key) {
            if prev.direction_bullish != bullish
                || now_ms - prev.last_at_ms > TREND_FLIP_CONFIRMATION_MAX_GAP_MS
                || now_ms < prev.last_at_ms
            {
                *prev = PendingTrendFlip {
                    direction_bullish: bullish,
                    first_at_ms: now_ms,
                    last_at_ms: now_ms,
                    observations: 1,
                };
                return false;
            }
            if now_ms - prev.last_at_ms < TREND_FLIP_CONFIRMATION_MIN_GAP_MS {
                return false;
            }
            prev.last_at_ms = now_ms;
            prev.observations += 1;
            if prev.observations >= 2 && now_ms - prev.first_at_ms >= TREND_FLIP_CONFIRMATION_MIN_MS
            {
                self.pending.remove(&key);
                return true;
            }
            false
        } else {
            self.pending.insert(
                key,
                PendingTrendFlip {
                    direction_bullish: bullish,
                    first_at_ms: now_ms,
                    last_at_ms: now_ms,
                    observations: 1,
                },
            );
            false
        }
    }

    pub fn clear(&mut self, symbol: &str, position_side: Side) {
        let key = format!("{}|{}", symbol.to_uppercase(), side_str(position_side));
        self.pending.remove(&key);
    }
}

pub fn classify_flip_strength(
    zlema_is_flip: bool,
    trend_score: f64,
    adx: f64,
    confidence: f64,
) -> &'static str {
    let mut points = 0;
    if zlema_is_flip {
        points += 2;
    }
    if trend_score >= 70.0 {
        points += 2;
    } else if trend_score >= MIN_TREND_SCORE_EXIT {
        points += 1;
    }
    if adx >= STRONG_ADX {
        points += 2;
    } else if adx >= 20.0 {
        points += 1;
    }
    if confidence >= 0.80 {
        points += 2;
    } else if confidence >= MIN_CONFIDENCE_EXIT {
        points += 1;
    }
    if points >= 6 {
        "STRONG"
    } else if points >= 3 {
        "MODERATE"
    } else {
        "WEAK"
    }
}

pub fn decide_trend_exit(
    against_position: bool,
    strength: &str,
    be_moved: bool,
    unrealized_net_usd: f64,
) -> (bool, &'static str) {
    if !against_position {
        return (false, "HOLD");
    }
    if strength == "STRONG" {
        return (true, "EXIT_NOW");
    }
    if strength == "MODERATE" {
        if be_moved || unrealized_net_usd <= 0.0 {
            return (true, "EXIT_NOW");
        }
        return (false, "WATCH");
    }
    (false, "HOLD")
}

// ─────────────────────── 5. Binance IP Rate-Limit Priority Guard (binanceRateGuard.ts) ───────────────────────

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum RatePriority {
    Critical,
    Normal,
    Bulk,
}

impl RatePriority {
    pub fn as_str(&self) -> &'static str {
        match self {
            RatePriority::Critical => "critical",
            RatePriority::Normal => "normal",
            RatePriority::Bulk => "bulk",
        }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct WeightBreakdownRow {
    pub label: String,
    pub weight: u32,
    pub requests: u32,
    pub priority: &'static str,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct RateGuardSnapshot {
    pub used_weight: u32,
    pub budget_per_min: u32,
    pub critical_reserve_weight: u32,
    pub bulk_budget_per_min: u32,
    pub cooldown_remaining_ms: i64,
    pub banned: bool,
    pub violation_streak: u32,
    pub suppressed_requests: u32,
    pub suppressed_critical: u32,
    pub suppressed_normal: u32,
    pub suppressed_bulk: u32,
    pub weight_breakdown: Vec<WeightBreakdownRow>,
}

#[derive(Clone, Debug)]
struct HitRecord {
    at_ms: i64,
    weight: u32,
    label: String,
    priority: RatePriority,
}

#[derive(Clone, Debug)]
pub struct BinanceRateGuard {
    budget_per_min: u32,
    critical_reserve_weight: u32,
    bulk_budget_per_min: u32,
    hits: Vec<HitRecord>,
    blocked_until_ms: i64,
    banned: bool,
    violation_streak: u32,
    suppressed_requests: u32,
    suppressed_critical: u32,
    suppressed_normal: u32,
    suppressed_bulk: u32,
}

impl Default for BinanceRateGuard {
    fn default() -> Self {
        Self::new(2100, 600)
    }
}

impl BinanceRateGuard {
    pub fn new(budget_per_min: u32, critical_reserve_weight: u32) -> Self {
        let bulk = budget_per_min
            .saturating_sub(critical_reserve_weight)
            .max(100);
        Self {
            budget_per_min,
            critical_reserve_weight,
            bulk_budget_per_min: bulk,
            hits: Vec::new(),
            blocked_until_ms: 0,
            banned: false,
            violation_streak: 0,
            suppressed_requests: 0,
            suppressed_critical: 0,
            suppressed_normal: 0,
            suppressed_bulk: 0,
        }
    }

    fn prune(&mut self, now_ms: i64) {
        self.hits.retain(|h| now_ms - h.at_ms <= 60_000);
        if now_ms >= self.blocked_until_ms {
            self.banned = false;
        }
    }

    pub fn try_acquire(
        &mut self,
        label: &str,
        weight: u32,
        priority: RatePriority,
        now_ms: i64,
    ) -> bool {
        self.prune(now_ms);
        if now_ms < self.blocked_until_ms {
            self.record_suppressed(priority);
            return false;
        }
        let used: u32 = self.hits.iter().map(|h| h.weight).sum();
        let limit = match priority {
            RatePriority::Bulk => self.bulk_budget_per_min,
            RatePriority::Normal | RatePriority::Critical => self.budget_per_min,
        };
        if used + weight > limit {
            self.record_suppressed(priority);
            return false;
        }
        self.hits.push(HitRecord {
            at_ms: now_ms,
            weight,
            label: label.to_string(),
            priority,
        });
        true
    }

    fn record_suppressed(&mut self, priority: RatePriority) {
        self.suppressed_requests += 1;
        match priority {
            RatePriority::Critical => self.suppressed_critical += 1,
            RatePriority::Normal => self.suppressed_normal += 1,
            RatePriority::Bulk => self.suppressed_bulk += 1,
        }
    }

    pub fn report_violation(&mut self, http_status: u16, now_ms: i64) {
        self.violation_streak = (self.violation_streak + 1).min(4);
        let mult = 1_i64 << (self.violation_streak - 1);
        let cooldown = if http_status == 418 {
            self.banned = true;
            (15 * 60_000 * mult).min(2 * 60 * 60_000)
        } else {
            (60_000 * mult).min(15 * 60_000)
        };
        self.blocked_until_ms = now_ms + cooldown;
    }

    pub fn snapshot(&mut self, now_ms: i64) -> RateGuardSnapshot {
        self.prune(now_ms);
        let used_weight: u32 = self.hits.iter().map(|h| h.weight).sum();
        let mut by_label: HashMap<String, (u32, u32, &'static str)> = HashMap::new();
        for h in &self.hits {
            let e = by_label
                .entry(h.label.clone())
                .or_insert((0, 0, h.priority.as_str()));
            e.0 += h.weight;
            e.1 += 1;
        }
        let mut breakdown: Vec<WeightBreakdownRow> = by_label
            .into_iter()
            .map(|(label, (weight, requests, priority))| WeightBreakdownRow {
                label,
                weight,
                requests,
                priority,
            })
            .collect();
        breakdown.sort_by(|a, b| b.weight.cmp(&a.weight));
        breakdown.truncate(12);

        RateGuardSnapshot {
            used_weight,
            budget_per_min: self.budget_per_min,
            critical_reserve_weight: self.critical_reserve_weight,
            bulk_budget_per_min: self.bulk_budget_per_min,
            cooldown_remaining_ms: (self.blocked_until_ms - now_ms).max(0),
            banned: self.banned,
            violation_streak: self.violation_streak,
            suppressed_requests: self.suppressed_requests,
            suppressed_critical: self.suppressed_critical,
            suppressed_normal: self.suppressed_normal,
            suppressed_bulk: self.suppressed_bulk,
            weight_breakdown: breakdown,
        }
    }
}

// ─────────────────────── 6. Exchange vs Local DB Mirror Reconciler (liveMirror.ts) ───────────────────────

#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct MirrorPositionDiff {
    pub symbol: String,
    pub status: &'static str, // "COCOK" | "HANYA_DB" | "HANYA_EXCHANGE" | "QTY_MISMATCH"
    pub db_qty: Option<f64>,
    pub exchange_qty: Option<f64>,
    pub note: &'static str,
}

pub fn reconcile_mirror_positions(
    db_positions: &[(String, f64)],
    exchange_positions: &[(String, f64)],
) -> Vec<MirrorPositionDiff> {
    let mut out = Vec::new();
    let db_map: HashMap<String, f64> = db_positions
        .iter()
        .map(|(s, q)| (s.to_uppercase(), *q))
        .collect();
    let ex_map: HashMap<String, f64> = exchange_positions
        .iter()
        .map(|(s, q)| (s.to_uppercase(), *q))
        .collect();

    let mut all_syms: Vec<String> = db_map.keys().chain(ex_map.keys()).cloned().collect();
    all_syms.sort();
    all_syms.dedup();

    for sym in all_syms {
        match (db_map.get(&sym).copied(), ex_map.get(&sym).copied()) {
            (Some(dq), Some(eq)) => {
                if (dq.abs() - eq.abs()).abs() <= 1e-6 {
                    out.push(MirrorPositionDiff {
                        symbol: sym,
                        status: "COCOK",
                        db_qty: Some(dq),
                        exchange_qty: Some(eq),
                        note: "DB dan exchange konsisten",
                    });
                } else {
                    out.push(MirrorPositionDiff {
                        symbol: sym,
                        status: "QTY_MISMATCH",
                        db_qty: Some(dq),
                        exchange_qty: Some(eq),
                        note: "ANOMALI: ukuran posisi DB vs exchange berbeda",
                    });
                }
            }
            (Some(dq), None) => out.push(MirrorPositionDiff {
                symbol: sym,
                status: "HANYA_DB",
                db_qty: Some(dq),
                exchange_qty: None,
                note: "ANOMALI: posisi tercatat di DB tapi tidak ada di exchange",
            }),
            (None, Some(eq)) => out.push(MirrorPositionDiff {
                symbol: sym,
                status: "HANYA_EXCHANGE",
                db_qty: None,
                exchange_qty: Some(eq),
                note: "ANOMALI: posisi terbuka di exchange tanpa catatan DB",
            }),
            (None, None) => {}
        }
    }
    out
}

// ─────────────────────── 7. Probability, Payoff, 5-Whys Forensics & Strategy Tuner ───────────────────────

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct WilsonInterval {
    pub lower_pct: f64,
    pub upper_pct: f64,
}

/// Interval kepercayaan 95% Wilson (`probabilityDiagnostics.ts:wilson95`).
pub fn wilson95(wins: usize, n: usize) -> Option<WilsonInterval> {
    if n == 0 || wins > n {
        return None;
    }
    let z = 1.959_963_984_540_054_f64;
    let nf = n as f64;
    let p = wins as f64 / nf;
    let z2 = z * z;
    let denom = 1.0 + z2 / nf;
    let center = (p + z2 / (2.0 * nf)) / denom;
    let radius = z * ((p * (1.0 - p) / nf) + z2 / (4.0 * nf * nf)).sqrt() / denom;
    Some(WilsonInterval {
        lower_pct: ((100.0 * (center - radius).max(0.0)) * 100.0).round() / 100.0,
        upper_pct: ((100.0 * (center + radius).min(1.0)) * 100.0).round() / 100.0,
    })
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ObservedProbabilitySummary {
    pub sample_count: usize,
    pub wins: usize,
    pub observed_win_rate_pct: Option<f64>,
    pub wilson95_pct: Option<WilsonInterval>,
    pub break_even_win_rate_pct: Option<f64>,
    pub observed_edge_pct_points: Option<f64>,
    pub average_win_usd: Option<f64>,
    pub average_loss_usd: Option<f64>,
    pub net_pnl_usd: f64,
    pub profit_factor: Option<f64>,
}

/// Ringkasan probabilitas & payoff deskriptif (`probabilityDiagnostics.ts` + `payoffDiagnostics.ts`).
pub fn summarize_observed_probability(pnls_usd: &[f64]) -> ObservedProbabilitySummary {
    let valid: Vec<f64> = pnls_usd.iter().copied().filter(|x| x.is_finite()).collect();
    let n = valid.len();
    let wins: Vec<f64> = valid.iter().copied().filter(|&x| x > 0.0).collect();
    let losses: Vec<f64> = valid.iter().copied().filter(|&x| x <= 0.0).collect();
    let win_sum: f64 = wins.iter().sum();
    let loss_sum: f64 = -losses.iter().sum::<f64>();
    let avg_win = (!wins.is_empty()).then(|| win_sum / wins.len() as f64);
    let avg_loss = (!losses.is_empty()).then(|| loss_sum / losses.len() as f64);
    let break_even = match (avg_win, avg_loss) {
        (Some(w), Some(l)) if w + l > 0.0 => Some(100.0 * l / (w + l)),
        _ => None,
    };
    let obs_wr = (n > 0).then(|| 100.0 * wins.len() as f64 / n as f64);
    let edge = match (obs_wr, break_even) {
        (Some(o), Some(b)) => Some(o - b),
        _ => None,
    };
    let pf = (loss_sum > 0.0).then(|| win_sum / loss_sum);
    ObservedProbabilitySummary {
        sample_count: n,
        wins: wins.len(),
        observed_win_rate_pct: obs_wr,
        wilson95_pct: wilson95(wins.len(), n),
        break_even_win_rate_pct: break_even,
        observed_edge_pct_points: edge,
        average_win_usd: avg_win,
        average_loss_usd: avg_loss,
        net_pnl_usd: win_sum - loss_sum,
        profit_factor: pf,
    }
}

pub const FIVE_WHYS_LAYER_FOCUS: [&str; 5] = [
    "GEJALA_DEVIASI_EKSEKUSI",
    "ALASAN_TAKTIKAL_CHART",
    "ASUMSI_DATA_INDIKATOR",
    "PARAMETER_SIZING_BIAS",
    "CELAH_PLAYBOOK_ATURAN",
];

#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct WhyLayer {
    pub layer: u8,
    pub focus: &'static str,
    pub answer: String,
    pub status: &'static str,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct FiveWhysForensicReport {
    pub symbol: String,
    pub strategy_id: String,
    pub execution_mistake: String,
    pub layers: Vec<WhyLayer>,
    pub terminated: &'static str,
}

/// Diagnosis 5-Whys Root Cause Analysis pasca-trade (`fiveWhys.ts:runFiveWhysDiagnosis`).
pub fn generate_five_whys_forensics(
    symbol: &str,
    strategy_id: &str,
    pnl_usd: f64,
    r_multiple: f64,
    mfe_r: f64,
    mae_r: f64,
    mistake: &str,
) -> FiveWhysForensicReport {
    let layers = vec![
        WhyLayer {
            layer: 1,
            focus: FIVE_WHYS_LAYER_FOCUS[0],
            answer: format!(
                "{} ({}) ditutup dengan PnL ${:.4} ({:.2}R), MFE {:.2}R vs MAE {:.2}R; terdeteksi klasifikasi {}.",
                symbol, strategy_id, pnl_usd, r_multiple, mfe_r, mae_r, mistake
            ),
            status: "HYPOTHESIS",
        },
        WhyLayer {
            layer: 2,
            focus: FIVE_WHYS_LAYER_FOCUS[1],
            answer: if mfe_r >= 0.7 && r_multiple < 0.0 {
                "Harga sempat bergerak searah setup >=0.7R namun berbalik sebelum mencapai target 1.7R akibat impuls likuiditas lawan.".into()
            } else {
                "Ekspansi harga pasca-entry langsung menguji batas invalidasi struktural tanpa follow-through agresor.".into()
            },
            status: "HYPOTHESIS",
        },
        WhyLayer {
            layer: 3,
            focus: FIVE_WHYS_LAYER_FOCUS[2],
            answer: "Konfluensi indikator bar tertutup belum mengantisipasi penyerapan pasif (eaten/pulled wall) pada level mikrostruktur.".into(),
            status: "HYPOTHESIS",
        },
        WhyLayer {
            layer: 4,
            focus: FIVE_WHYS_LAYER_FOCUS[3],
            answer: "Rasio SL/ATR pada pita SCALP 2%-4% menjaga risiko tetap $1.00, namun membutuhkan filter pullBiasThreshold / CVD yang ketat.".into(),
            status: "HYPOTHESIS",
        },
        WhyLayer {
            layer: 5,
            focus: FIVE_WHYS_LAYER_FOCUS[4],
            answer: format!(
                "Aturan Playbook If-Then: bila {} berulang >=2x pada {}, aktifkan cooldown 60 menit dan wajibkan konfirmasi CVD + 5m body.",
                mistake, symbol
            ),
            status: "HYPOTHESIS",
        },
    ];
    FiveWhysForensicReport {
        symbol: symbol.to_uppercase(),
        strategy_id: strategy_id.to_string(),
        execution_mistake: mistake.to_string(),
        layers,
        terminated: "COMPLETE",
    }
}

#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct BoundedTunerProposal {
    pub param: &'static str,
    pub from: f64,
    pub to: f64,
    pub bounded_pct: f64,
    pub status: &'static str,
    pub rationale: String,
}

/// Proposal penyetelan parameter bounded `<= 10%` (`strategyTuner.ts:proposeTuning`).
pub fn propose_bounded_tuning(
    strategy_id: &str,
    current_val: f64,
    sample_count: usize,
    win_rate_pct: f64,
) -> BoundedTunerProposal {
    let param = if strategy_id == "S-02" || strategy_id == "S02_SNIPER" {
        "pullBiasThreshold"
    } else {
        "tp1R"
    };
    if sample_count < 30 {
        return BoundedTunerProposal {
            param,
            from: current_val,
            to: current_val,
            bounded_pct: 0.0,
            status: "CANDIDATE",
            rationale: format!(
                "WAITING_FOR_COHORT_SAMPLES: sampel tertutup baru {} (<30); parameter {} dikunci di {:.3}.",
                sample_count, param, current_val
            ),
        };
    }
    let delta_pct = if win_rate_pct < 50.0 { 0.05 } else { 0.0 };
    let to = ((current_val * (1.0 + delta_pct)) * 1000.0).round() / 1000.0;
    BoundedTunerProposal {
        param,
        from: current_val,
        to,
        bounded_pct: delta_pct * 100.0,
        status: if delta_pct > 0.0 { "CANDIDATE" } else { "ACTIVE" },
        rationale: format!(
            "Cohort n={}, WR={:.1}%: perubahan bounded {:.1}% (<=10%) pada {} ({:.3} -> {:.3}); wajib lulus gate replay OOS.",
            sample_count, win_rate_pct, delta_pct * 100.0, param, current_val, to
        ),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn structural_sizing_and_fixed_levels_lock_1usd_risk() {
        let sz = structural_risk_sizing(100.0, 97.5, 0.01, 1.0, 4.8, 25);
        assert!(sz.ok);
        assert!(sz.margin_usd <= 4.8);
        assert!(sz.risk_at_structure_usd <= 1.05);

        let lv = fixed_risk_levels(100.0, sz.qty, Side::Long, 0.01, 1.0, 1.7).unwrap();
        assert!(lv.risk_usd <= 1.0 + 1e-6);
        assert!(lv.tp_price > 100.0);
        assert!(lv.sl_price < 100.0);
    }

    #[test]
    fn sl_tp_menu_and_council_and_rate_guard_work() {
        let menu = build_sl_tp_menu_scalp(100.0, 97.5, Side::Long, 0.008).unwrap();
        assert_eq!(menu.sl[0].id, "A");
        assert_eq!(menu.tp[0].r, 1.7);

        let council = convene_deterministic_council(
            "SOLUSDT",
            Side::Short,
            true,
            "ALT_PLAYGROUND",
            78,
            Some(0.65),
            0.025,
            0.95,
            true,
        );
        assert_eq!(council.final_action, "EXECUTE");
        assert!(council.consensus_level >= 60.0);

        let mut rg = BinanceRateGuard::default();
        assert!(rg.try_acquire("GET /fapi/v1/klines", 10, RatePriority::Bulk, 1000));
        let snap = rg.snapshot(1000);
        assert_eq!(snap.used_weight, 10);
    }

    #[test]
    fn wilson95_and_five_whys_produce_valid_diagnostics() {
        let summary = summarize_observed_probability(&[1.7, 1.7, -1.0, 1.7, -1.0, 1.7, 1.7]);
        assert_eq!(summary.sample_count, 7);
        assert_eq!(summary.wins, 5);
        assert!(summary.wilson95_pct.is_some());

        let fw = generate_five_whys_forensics(
            "HAEDALUSDT",
            "S-02",
            -1.0,
            -1.0,
            0.85,
            1.0,
            "REVERSAL_AFTER_MFE",
        );
        assert_eq!(fw.layers.len(), 5);
        assert_eq!(fw.terminated, "COMPLETE");
    }

    #[test]
    fn council_and_arena_decisions_identical_with_default_tracking() {
        #[allow(clippy::type_complexity)]
        let cases: &[(
            &str,
            Side,
            f64,
            f64,
            Option<f64>,
            bool,
            &str,
            Option<u32>,
            Option<f64>,
            bool,
            Option<f64>,
            bool,
            bool,
        )] = &[
            // Semua input memakai fallback default
            (
                "SOLUSDT",
                Side::Long,
                0.0,
                0.0,
                None,
                true,
                "ALT_PLAYGROUND",
                None,
                None,
                true,
                None,
                true,
                true,
            ),
            // Input lengkap (kecuali arena_trend_score None seperti jalur live saat ini)
            (
                "ETHUSDT",
                Side::Short,
                2500.0,
                2562.5,
                Some(22.0),
                true,
                "BTC_LED",
                Some(82),
                Some(0.68),
                true,
                None,
                true,
                true,
            ),
            // Hard veto makro Vega (NO_GO conf=82 > 70)
            (
                "XRPUSDT",
                Side::Long,
                0.60,
                0.585,
                Some(0.006),
                false,
                "RISK_OFF",
                Some(85),
                Some(0.75),
                true,
                Some(78.0),
                false,
                true,
            ),
            // Hard veto Wardha (SL/ATR < 0.50 -> NO_GO conf=85 > 70)
            (
                "DOGEUSDT",
                Side::Long,
                0.15,
                0.148,
                Some(0.006),
                true,
                "ALT_PLAYGROUND",
                Some(75),
                None,
                true,
                None,
                true,
                false,
            ),
            // Arka HOLD + Wardha HOLD (SL di luar pita [1.2%, 4.5%]) -> REJECT tanpa hard veto
            (
                "ADAUSDT",
                Side::Short,
                0.45,
                0.475,
                Some(0.004),
                true,
                "ALT_PLAYGROUND",
                Some(45),
                Some(0.35),
                false,
                Some(65.0),
                true,
                true,
            ),
            // Wardha memilih varian SL "B" karena sl_atr_ratio < 0.85
            (
                "LINKUSDT",
                Side::Long,
                15.0,
                14.68,
                Some(0.45),
                true,
                "ALT_PLAYGROUND",
                None,
                Some(0.52),
                true,
                None,
                true,
                true,
            ),
        ];

        for &(
            symbol,
            side,
            entry,
            sl,
            atr_14,
            macro_allowed,
            macro_state,
            composite_score,
            knn_prob,
            cvd_aligned,
            arena_trend_score,
            macro_long_ok,
            macro_short_ok,
        ) in cases
        {
            // Rumus lama persis seperti live.rs sebelum WP-4c
            let legacy_atr_pct = match atr_14 {
                Some(a) if entry > 0.0 && a > 0.0 => a / entry,
                _ => 0.008,
            };
            let legacy_sl_pct = if entry > 0.0 {
                (entry - sl).abs() / entry
            } else {
                0.025
            };
            let legacy_ratio = if legacy_atr_pct > 0.0 {
                legacy_sl_pct / legacy_atr_pct
            } else {
                1.0
            };
            let legacy_comp = composite_score.unwrap_or(72);
            let legacy_council = convene_deterministic_council(
                symbol,
                side,
                macro_allowed,
                macro_state,
                legacy_comp,
                knn_prob,
                legacy_sl_pct,
                legacy_ratio,
                cvd_aligned,
            );
            let legacy_battle = run_deterministic_direction_battle(
                symbol,
                arena_trend_score.unwrap_or(72.0),
                side == Side::Long,
                macro_long_ok,
                macro_short_ok,
                if side == Side::Long { 1.0 } else { -1.0 },
            );

            let eval = evaluate_council_and_arena(&CouncilArenaInputs {
                symbol,
                side,
                entry,
                sl,
                atr_14,
                macro_allowed,
                macro_state,
                composite_score,
                knn_prob,
                cvd_aligned,
                arena_trend_score,
                macro_long_ok,
                macro_short_ok,
            });

            // Keputusan wajib 100% identik
            assert!(eval.council.enforced);
            assert!(!eval.battle_arena.enforced);
            assert_eq!(eval.council.final_action, legacy_council.final_action);
            assert_eq!(eval.council.consensus_level, legacy_council.consensus_level);
            assert_eq!(eval.council.votes, legacy_council.votes);
            assert_eq!(eval.council.sl_choice, legacy_council.sl_choice);
            assert_eq!(eval.council.tp_choice, legacy_council.tp_choice);
            assert_eq!(eval.battle_arena, legacy_battle);

            // Pelacakan default jujur
            assert_eq!(
                eval.inputs_defaulted.contains(&"atr_pct"),
                !(entry > 0.0 && atr_14.is_some_and(|a| a > 0.0))
            );
            assert_eq!(eval.inputs_defaulted.contains(&"sl_pct"), !(entry > 0.0));
            assert_eq!(
                eval.inputs_defaulted.contains(&"composite_score"),
                composite_score.is_none()
            );
            assert_eq!(
                eval.inputs_defaulted.contains(&"knn_prob"),
                knn_prob.is_none()
            );
            assert_eq!(
                eval.inputs_defaulted.contains(&"arena_trend_score"),
                arena_trend_score.is_none()
            );
        }
    }
}
