//! Forensik pasca-trade 5-Whys berbasis bukti terukur (`WP-6`).
//!
//! Prinsip metodologis (wajib diperhatikan pembaca laporan):
//! **RCA (Root Cause Analysis) satu trade pada hasil trading yang didominasi noise stokastik
//! BUKAN merupakan bukti kausal.** Satu kekalahan dengan chop tinggi atau MFE yang berbalik tidak
//! membuktikan bahwa perubahan filter akan meningkatkan ekspektasi out-of-sample. Nilai diagnostik
//! forensik ini baru muncul ketika diagregasi secara statistik lintas sampel pada `apex review`
//! (`WP-7`) dan diuji lewat eksperimen pra-registrasi di pipeline `apex validate`.
//!
//! Dengan sekitar ±4 trade per bulan, adaptasi parameter online hampir pasti overfit. Karena itu,
//! proposal pada Layer 5 tidak pernah diterapkan otomatis ke konfigurasi atau jalur eksekusi.

use rust_decimal::Decimal;
use serde::{Deserialize, Serialize};

use crate::telemetry::TradeTelemetry;
use crate::yahudi_exec::{classify_trade_mistake, MistakeTag};

/// Ambang MFE (0,7R) di mana pembalikan menuju rugi mulai dicatat sebagai indikasi *gave-back*.
pub const GAVE_BACK_WARN_MFE_R: Decimal = Decimal::from_parts(7, 0, 0, false, 1);
/// Ambang MFE kuat (1,0R) sebelum berbalik menyentuh SL.
pub const GAVE_BACK_STRONG_MFE_R: Decimal = Decimal::ONE;
/// Rasio slippage realisasi terhadap model (`> 2x`) yang menandai anomali eksekusi.
pub const SLIP_ANOMALY_RATIO: Decimal = Decimal::TWO;
/// Ambang default gate spread (`config/risk.toml`: `max_spread_bps = 20.0`).
pub const DEFAULT_MAX_SPREAD_BPS: Decimal = Decimal::from_parts(20, 0, 0, false, 0);
/// Batas durasi tanpa SL terkonfirmasi sesuai `Rules.md` GR-8 (`2000 ms`).
pub const UNPROTECTED_ANOMALY_MS: i64 = 2_000;
/// Ambang Choppiness Index (14) yang menandai rezim konsolidasi rawan whipsaw.
pub const HIGH_CHOP_THRESHOLD: f64 = 58.0;
/// Ambang ADX (14) rendah yang menandai ketiadaan kekuatan tren.
pub const LOW_ADX_THRESHOLD: f64 = 20.0;
/// Syarat wajib setiap proposal forensik sebelum boleh menyentuh parameter strategi.
pub const PROPOSAL_REQUIRES: &str = "uji pra-registrasi lewat pipeline validate";

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum EvidenceStatus {
    Supported,
    Unsupported,
    InsufficientData,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum RcaTermination {
    RootCauseSupported,
    NormalVariance,
    Inconclusive,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Proposal {
    pub trade_id: String,
    pub symbol: String,
    pub strategy: String,
    pub hypothesis: String,
    pub evidence: Vec<(String, String)>,
    pub requires: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct FiveWhyLayer {
    pub layer: u8,
    pub focus: String,
    pub question: String,
    pub answer: String,
    pub evidence: Vec<(String, String)>,
    pub status: EvidenceStatus,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct FiveWhys {
    pub trade_id: String,
    pub symbol: String,
    pub strategy_id: String,
    pub mistake_tag: MistakeTag,
    pub layers: Vec<FiveWhyLayer>,
    pub terminated: RcaTermination,
    pub proposal: Option<Proposal>,
}

fn layer(
    num: u8,
    focus: &str,
    question: &str,
    answer: String,
    evidence: Vec<(String, String)>,
    status: EvidenceStatus,
) -> FiveWhyLayer {
    FiveWhyLayer {
        layer: num,
        focus: focus.to_string(),
        question: question.to_string(),
        answer,
        evidence,
        status,
    }
}

fn build_layer_1_outcome(t: &TradeTelemetry) -> FiveWhyLayer {
    let ev = vec![
        ("r_multiple".into(), t.r_multiple.round_dp(4).to_string()),
        ("pnl_net_usd".into(), t.pnl_net_usd.round_dp(4).to_string()),
        ("exit_kind".into(), t.exit_kind.as_str().to_string()),
    ];
    let ans = format!(
        "Trade {} ({}) ditutup merugi pada {} sebesar {}R (${}).",
        t.symbol,
        t.strategy,
        t.exit_kind.as_str(),
        t.r_multiple.round_dp(4),
        t.pnl_net_usd.round_dp(4)
    );
    layer(
        1,
        "HASIL_TRADE",
        "Mengapa trade ditutup merugi?",
        ans,
        ev,
        EvidenceStatus::Supported,
    )
}

fn build_layer_2_excursion(t: &TradeTelemetry) -> FiveWhyLayer {
    let q = "Apakah trade sempat mencapai >=0.7R atau >=1.0R sebelum berbalik ke SL?";
    let Some(ex) = t.excursion.as_ref() else {
        return layer(
            2,
            "EXCURSION_PASCA_FILL",
            q,
            "Data excursion kline 15m pasca-fill tidak tersedia.".into(),
            vec![("excursion".into(), "missing".into())],
            EvidenceStatus::InsufficientData,
        );
    };
    let ev = vec![
        ("mfe_r".into(), ex.mfe_r.round_dp(4).to_string()),
        ("mae_r".into(), ex.mae_r.round_dp(4).to_string()),
        ("bars_in_trade".into(), ex.bars_in_trade.to_string()),
        ("resolution".into(), ex.resolution.clone()),
    ];
    if ex.bars_in_trade == 0 && ex.mfe_r == Decimal::ZERO && ex.mae_r == Decimal::ZERO {
        return layer(
            2,
            "EXCURSION_PASCA_FILL",
            q,
            "Posisi dibuka dan ditutup tanpa bar 15m tertutup pasca-fill; resolusi tidak cukup."
                .into(),
            ev,
            EvidenceStatus::InsufficientData,
        );
    }
    let (ans, st) = if ex.mfe_r >= GAVE_BACK_STRONG_MFE_R {
        (
            format!(
                "Ya, MFE sempat mencapai {}R (>=1.0R) sebelum berbalik merugi (gave-back).",
                ex.mfe_r.round_dp(4)
            ),
            EvidenceStatus::Supported,
        )
    } else if ex.mfe_r >= GAVE_BACK_WARN_MFE_R {
        (
            format!(
                "Ya, MFE sempat mencapai {}R (>=0.7R) namun gagal lanjut ke target.",
                ex.mfe_r.round_dp(4)
            ),
            EvidenceStatus::Supported,
        )
    } else {
        (
            format!(
                "Tidak, MFE maksimum hanya {}R (<0.7R) dengan MAE {}R.",
                ex.mfe_r.round_dp(4),
                ex.mae_r.round_dp(4)
            ),
            EvidenceStatus::Unsupported,
        )
    };
    layer(2, "EXCURSION_PASCA_FILL", q, ans, ev, st)
}

fn build_layer_3_execution(t: &TradeTelemetry, max_spread_bps: Decimal) -> FiveWhyLayer {
    let q = "Apakah ada deviasi eksekusi (slippage vs model, spread entry, jeda fill, atau proteksi SL)?";
    let spread_opt = t.entry_ctx.as_ref().and_then(|c| c.spread_bps);
    let fill_wait_ms = t.filled_ms.map(|f| f.saturating_sub(t.created_ms));
    let slip_anom = !t.fills.paper_slip_zero_by_construction
        && t.fills.model_slip_r > Decimal::ZERO
        && t.fills.realized_slip_r > SLIP_ANOMALY_RATIO * t.fills.model_slip_r;
    let spread_anom = spread_opt.is_some_and(|s| s > max_spread_bps);
    let unprot_anom = t.unprotected_ms.is_some_and(|u| u > UNPROTECTED_ANOMALY_MS);
    let ev = vec![
        (
            "realized_slip_r".into(),
            t.fills.realized_slip_r.round_dp(4).to_string(),
        ),
        (
            "model_slip_r".into(),
            t.fills.model_slip_r.round_dp(4).to_string(),
        ),
        (
            "paper_slip_zero_by_construction".into(),
            t.fills.paper_slip_zero_by_construction.to_string(),
        ),
        (
            "spread_bps".into(),
            spread_opt
                .map(|s| s.round_dp(4).to_string())
                .unwrap_or_else(|| "missing".into()),
        ),
        (
            "max_spread_bps".into(),
            max_spread_bps.round_dp(4).to_string(),
        ),
        (
            "fill_latency_ms".into(),
            fill_wait_ms
                .map(|v| v.to_string())
                .unwrap_or_else(|| "missing".into()),
        ),
        (
            "unprotected_ms".into(),
            t.unprotected_ms
                .map(|v| v.to_string())
                .unwrap_or_else(|| "missing".into()),
        ),
    ];
    let (ans, st) = if slip_anom || spread_anom || unprot_anom {
        (
            format!("Anomali eksekusi terukur: slip_anom={slip_anom}, spread_anom={spread_anom}, unprotected_anom={unprot_anom}."),
            EvidenceStatus::Supported,
        )
    } else if (t.entry_ctx.is_none() && t.filled_ms.is_none())
        || (t.fills.paper_slip_zero_by_construction && spread_opt.is_none())
    {
        (
            "Data spread/slippage riil belum cukup untuk memvalidasi kualitas eksekusi.".into(),
            EvidenceStatus::InsufficientData,
        )
    } else {
        (
            "Slippage realisasi, spread saat entry, dan waktu proteksi SL berada dalam batas model.".into(),
            EvidenceStatus::Unsupported,
        )
    };
    layer(3, "KUALITAS_EKSEKUSI", q, ans, ev, st)
}

fn build_layer_4_context(t: &TradeTelemetry) -> FiveWhyLayer {
    let q = "Apakah konteks entry (chop, ADX, rezim, bias makro, CVD) bertentangan dengan rezim target?";
    let Some(ctx) = t.entry_ctx.as_ref() else {
        return layer(
            4,
            "KONTEKS_ENTRY",
            q,
            "Konteks entry tidak terekam pada trade ini.".into(),
            vec![("entry_ctx".into(), "missing".into())],
            EvidenceStatus::InsufficientData,
        );
    };
    let ev = vec![
        (
            "chop".into(),
            ctx.chop
                .map(|v| format!("{v:.2}"))
                .unwrap_or_else(|| "missing".into()),
        ),
        (
            "adx".into(),
            ctx.adx
                .map(|v| format!("{v:.2}"))
                .unwrap_or_else(|| "missing".into()),
        ),
        (
            "regime_label".into(),
            if ctx.regime_label.is_empty() {
                "missing".into()
            } else {
                ctx.regime_label.clone()
            },
        ),
        (
            "macro_bias".into(),
            if ctx.macro_bias.is_empty() {
                "missing".into()
            } else {
                ctx.macro_bias.clone()
            },
        ),
        (
            "cvd_aligned".into(),
            ctx.cvd_aligned
                .map(|v| v.to_string())
                .unwrap_or_else(|| "missing".into()),
        ),
    ];
    let high_chop = ctx.chop.is_some_and(|c| c > HIGH_CHOP_THRESHOLD);
    let low_adx = ctx.adx.is_some_and(|a| a < LOW_ADX_THRESHOLD);
    let counter_cvd = ctx.cvd_aligned == Some(false);
    let hostile_regime = ctx.regime_label.eq_ignore_ascii_case("CHOP")
        || ctx.regime_label.eq_ignore_ascii_case("CHOP_NO_TRADE")
        || ctx.macro_bias.eq_ignore_ascii_case("RISK_OFF");
    let (ans, st) = if high_chop
        || (low_adx && ctx.chop.is_some_and(|c| c > 52.0))
        || counter_cvd
        || hostile_regime
    {
        (
            format!("Konteks kurang kondusif terukur (high_chop={high_chop}, low_adx={low_adx}, counter_cvd={counter_cvd}, hostile_regime={hostile_regime})."),
            EvidenceStatus::Supported,
        )
    } else if ctx.chop.is_none() || ctx.adx.is_none() || ctx.cvd_aligned.is_none() {
        (
            "Sebagian indikator konteks (chop/ADX/CVD) tidak tersedia untuk strategi ini.".into(),
            EvidenceStatus::InsufficientData,
        )
    } else {
        (
            "Chop, ADX, rezim, bias makro, dan CVD selaras saat entry.".into(),
            EvidenceStatus::Unsupported,
        )
    };
    layer(4, "KONTEKS_ENTRY", q, ans, ev, st)
}

fn build_layer_5_proposal(
    t: &TradeTelemetry,
    prior: &[FiveWhyLayer],
    mistake_tag: MistakeTag,
) -> (FiveWhyLayer, Option<Proposal>, RcaTermination) {
    let q = "Hipotesis apa yang didukung bukti untuk diuji pra-registrasi (tanpa auto-apply)?";
    let supported: Vec<&FiveWhyLayer> = prior
        .iter()
        .filter(|l| l.layer >= 2 && l.status == EvidenceStatus::Supported)
        .collect();
    if let Some(primary) = supported.first() {
        let ev: Vec<(String, String)> = supported
            .iter()
            .flat_map(|l| l.evidence.iter().cloned())
            .collect();
        let hypothesis = format!(
            "Evaluasi kohort {} pada fokus {} (tag={}): uji apakah aturan terkait menurunkan kerugian tanpa menggerus frekuensi sampel validasi.",
            t.strategy,
            primary.focus,
            mistake_tag.as_str()
        );
        let prop = Proposal {
            trade_id: t.trade_id.clone(),
            symbol: t.symbol.clone(),
            strategy: t.strategy.clone(),
            hypothesis: hypothesis.clone(),
            evidence: ev.clone(),
            requires: PROPOSAL_REQUIRES.to_string(),
        };
        let l5 = layer(
            5,
            "HIPOTESIS_PRA_REGISTRASI",
            q,
            format!("{hypothesis} Syarat mutlak: {PROPOSAL_REQUIRES}."),
            ev,
            EvidenceStatus::Supported,
        );
        return (l5, Some(prop), RcaTermination::RootCauseSupported);
    }
    let any_insuff = prior
        .iter()
        .any(|l| l.layer >= 2 && l.status == EvidenceStatus::InsufficientData)
        || mistake_tag == MistakeTag::InsufficientData;
    if any_insuff {
        (
            layer(
                5,
                "HIPOTESIS_PRA_REGISTRASI",
                q,
                "Data telemetri belum lengkap; tidak ada proposal yang diajukan.".into(),
                vec![("mistake_tag".into(), mistake_tag.as_str().into())],
                EvidenceStatus::InsufficientData,
            ),
            None,
            RcaTermination::Inconclusive,
        )
    } else {
        (
            layer(
                5,
                "HIPOTESIS_PRA_REGISTRASI",
                q,
                "Tidak ada anomali eksekusi, excursion, atau konteks; diklasifikasikan variansi normal.".into(),
                vec![("mistake_tag".into(), mistake_tag.as_str().into())],
                EvidenceStatus::Unsupported,
            ),
            None,
            RcaTermination::NormalVariance,
        )
    }
}

/// Bangun RCA 5-Whys berbasis bukti dengan ambang spread kustom.
/// Trade menang/impas (`pnl_net_usd >= 0` dan `r_multiple >= 0`) tidak diberi RCA (`None`).
pub fn build_five_whys_with_gate(t: &TradeTelemetry, max_spread_bps: Decimal) -> Option<FiveWhys> {
    if t.pnl_net_usd >= Decimal::ZERO && t.r_multiple >= Decimal::ZERO {
        return None;
    }
    let mistake_tag = classify_trade_mistake(t);
    let mut layers = vec![
        build_layer_1_outcome(t),
        build_layer_2_excursion(t),
        build_layer_3_execution(t, max_spread_bps),
        build_layer_4_context(t),
    ];
    let (l5, proposal, terminated) = build_layer_5_proposal(t, &layers, mistake_tag);
    layers.push(l5);
    Some(FiveWhys {
        trade_id: t.trade_id.clone(),
        symbol: t.symbol.clone(),
        strategy_id: t.strategy.clone(),
        mistake_tag,
        layers,
        terminated,
        proposal,
    })
}

/// Bangun RCA 5-Whys berbasis bukti memakai `DEFAULT_MAX_SPREAD_BPS`.
pub fn build_five_whys(t: &TradeTelemetry) -> Option<FiveWhys> {
    build_five_whys_with_gate(t, DEFAULT_MAX_SPREAD_BPS)
}
