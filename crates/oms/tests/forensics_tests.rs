//! Pengujian table-driven dan snapshot golden untuk forensik 5-Whys berbasis bukti (`WP-6`).

use apex_core::types::Side;
use apex_oms::{
    build_five_whys, classify_trade_mistake, EntryContext, EvidenceStatus, ExecSource, FillSummary,
    MistakeTag, RcaTermination, TelemetryExitKind, TradeExcursion, TradeTelemetry,
    PROPOSAL_REQUIRES, TELEMETRY_SCHEMA_VERSION,
};
use rust_decimal::Decimal;

fn sample_loser() -> TradeTelemetry {
    TradeTelemetry {
        schema: TELEMETRY_SCHEMA_VERSION,
        trade_id: "T-100".to_string(),
        symbol: "SOLUSDT".to_string(),
        strategy: "S1".to_string(),
        validated: false,
        side: Side::Long,
        created_ms: 1_700_000_000_000,
        filled_ms: Some(1_700_000_005_000),
        fill_confirmed_at: Some(1_700_000_005_050),
        sl_placed_ms: Some(1_700_000_005_300),
        unprotected_ms: Some(250),
        closed_ms: 1_700_000_905_000,
        entry_price: Decimal::from(100),
        exit_price: Decimal::from(97),
        planned_risk_usd: Decimal::ONE,
        pnl_net_usd: -Decimal::ONE,
        r_multiple: -Decimal::ONE,
        exit_kind: TelemetryExitKind::SlHit,
        entry_ctx: Some(EntryContext {
            spread_bps: Some(Decimal::from(4)),
            planned_slip_bps: Decimal::from(5),
            cvd_aligned: Some(true),
            chop: Some(42.0),
            adx: Some(28.0),
            atr_pct: Some(0.012),
            regime_label: "TREND_UP".to_string(),
            macro_bias: "ALT_PLAYGROUND".to_string(),
            council: None,
            config_hash: "0123456789abcdef".to_string(),
            signal_ts: 1_700_000_000_000,
            planned_entry: Decimal::from(100),
            planned_sl: Decimal::from(97),
            planned_tp: Decimal::from(106),
        }),
        excursion: Some(TradeExcursion {
            mfe_r: Decimal::new(25, 2),
            mae_r: Decimal::ONE,
            bars_in_trade: 2,
            mfe_ts: Some(1_700_000_900_000),
            mae_ts: Some(1_700_000_900_000),
            resolution: "15m_closed_bar".to_string(),
        }),
        fills: FillSummary {
            entry_slip_bps: Decimal::ZERO,
            exit_slip_bps: Decimal::from(3),
            model_slip_r: Decimal::new(4, 2),
            realized_slip_r: Decimal::new(4, 2),
            fee_usd: Decimal::new(4, 2),
            funding_usd: Decimal::ZERO,
            maker_entry: true,
            source: ExecSource::Demo,
            paper_slip_zero_by_construction: false,
        },
    }
}

#[test]
fn table_driven_forensics_covers_all_branches_and_none_never_triggers_false_tag() {
    let mut winner = sample_loser();
    winner.pnl_net_usd = Decimal::new(170, 2);
    winner.r_multiple = Decimal::new(170, 2);
    winner.exit_kind = TelemetryExitKind::Tp1;

    let mut gave_back = sample_loser();
    gave_back.trade_id = "T-GAVEBACK".to_string();
    if let Some(ex) = gave_back.excursion.as_mut() {
        ex.mfe_r = Decimal::new(12, 1);
    }

    let mut high_slip = sample_loser();
    high_slip.trade_id = "T-SLIP".to_string();
    high_slip.fills.entry_slip_bps = Decimal::from(12);
    high_slip.fills.model_slip_r = Decimal::new(4, 2);
    high_slip.fills.realized_slip_r = Decimal::new(11, 2);

    let mut high_chop = sample_loser();
    high_chop.trade_id = "T-CHOP".to_string();
    high_chop.closed_ms = high_chop.filled_ms.unwrap() + 4 * 60 * 1000;
    if let Some(ctx) = high_chop.entry_ctx.as_mut() {
        ctx.chop = Some(62.5);
        ctx.adx = Some(16.0);
    }

    let mut missing = sample_loser();
    missing.trade_id = "T-MISSING".to_string();
    missing.entry_ctx = None;
    missing.excursion = None;
    missing.filled_ms = None;

    let mut partial_none = sample_loser();
    partial_none.trade_id = "T-PARTIAL-NONE".to_string();
    partial_none.closed_ms = partial_none.filled_ms.unwrap() + 3 * 60 * 1000;
    if let Some(ctx) = partial_none.entry_ctx.as_mut() {
        ctx.spread_bps = None;
        ctx.cvd_aligned = None;
        ctx.chop = None;
        ctx.adx = None;
    }

    let normal = sample_loser();

    #[allow(clippy::type_complexity)]
    let cases: Vec<(
        &str,
        TradeTelemetry,
        MistakeTag,
        Option<(RcaTermination, usize, bool)>,
    )> = vec![
        ("winner", winner, MistakeTag::None, None),
        (
            "gave_back_1_2r",
            gave_back,
            MistakeTag::NormalVariance,
            Some((RcaTermination::RootCauseSupported, 1, true)),
        ),
        (
            "slip_gt_2x",
            high_slip,
            MistakeTag::ImpulseChase,
            Some((RcaTermination::RootCauseSupported, 2, true)),
        ),
        (
            "high_chop",
            high_chop,
            MistakeTag::ChopWhipsaw,
            Some((RcaTermination::RootCauseSupported, 3, true)),
        ),
        (
            "missing_telemetry",
            missing,
            MistakeTag::InsufficientData,
            Some((RcaTermination::Inconclusive, 0, false)),
        ),
        (
            "partial_none_observables",
            partial_none,
            MistakeTag::InsufficientData,
            Some((RcaTermination::Inconclusive, 0, false)),
        ),
        (
            "normal_variance",
            normal,
            MistakeTag::NormalVariance,
            Some((RcaTermination::NormalVariance, 0, false)),
        ),
    ];

    for (label, tel, exp_tag, exp_rca) in cases {
        assert_eq!(classify_trade_mistake(&tel), exp_tag, "case={label}");
        let rca = build_five_whys(&tel);
        match exp_rca {
            None => assert!(rca.is_none(), "winner wajib tanpa RCA: {label}"),
            Some((exp_term, supported_idx, has_prop)) => {
                let fw = rca.expect("loser wajib menghasilkan RCA");
                assert_eq!(fw.layers.len(), 5, "case={label}");
                assert_eq!(fw.terminated, exp_term, "case={label}");
                assert_eq!(fw.proposal.is_some(), has_prop, "case={label}");
                if let Some(p) = &fw.proposal {
                    assert_eq!(p.requires, PROPOSAL_REQUIRES);
                }
                if supported_idx > 0 {
                    assert_eq!(
                        fw.layers[supported_idx].status,
                        EvidenceStatus::Supported,
                        "case={label}"
                    );
                }
            }
        }
    }
}

#[test]
fn five_whys_golden_json_snapshot() {
    let mut t = sample_loser();
    if let Some(ex) = t.excursion.as_mut() {
        ex.mfe_r = Decimal::new(12, 1);
    }
    let fw = build_five_whys(&t).unwrap();
    let json = serde_json::to_value(&fw).unwrap();
    let expected = serde_json::json!({
        "trade_id": "T-100",
        "symbol": "SOLUSDT",
        "strategy_id": "S1",
        "mistake_tag": "NormalVariance",
        "terminated": "RootCauseSupported",
        "layers": [
            {
                "layer": 1,
                "focus": "HASIL_TRADE",
                "question": "Mengapa trade ditutup merugi?",
                "answer": "Trade SOLUSDT (S1) ditutup merugi pada SlHit sebesar -1R ($-1).",
                "evidence": [["r_multiple", "-1"], ["pnl_net_usd", "-1"], ["exit_kind", "SlHit"]],
                "status": "Supported"
            },
            {
                "layer": 2,
                "focus": "EXCURSION_PASCA_FILL",
                "question": "Apakah trade sempat mencapai >=0.7R atau >=1.0R sebelum berbalik ke SL?",
                "answer": "Ya, MFE sempat mencapai 1.2R (>=1.0R) sebelum berbalik merugi (gave-back).",
                "evidence": [["mfe_r", "1.2"], ["mae_r", "1"], ["bars_in_trade", "2"], ["resolution", "15m_closed_bar"]],
                "status": "Supported"
            },
            {
                "layer": 3,
                "focus": "KUALITAS_EKSEKUSI",
                "question": "Apakah ada deviasi eksekusi (slippage vs model, spread entry, jeda fill, atau proteksi SL)?",
                "answer": "Slippage realisasi, spread saat entry, dan waktu proteksi SL berada dalam batas model.",
                "evidence": [
                    ["realized_slip_r", "0.04"],
                    ["model_slip_r", "0.04"],
                    ["paper_slip_zero_by_construction", "false"],
                    ["spread_bps", "4"],
                    ["max_spread_bps", "20"],
                    ["fill_latency_ms", "5000"],
                    ["unprotected_ms", "250"]
                ],
                "status": "Unsupported"
            },
            {
                "layer": 4,
                "focus": "KONTEKS_ENTRY",
                "question": "Apakah konteks entry (chop, ADX, rezim, bias makro, CVD) bertentangan dengan rezim target?",
                "answer": "Chop, ADX, rezim, bias makro, dan CVD selaras saat entry.",
                "evidence": [
                    ["chop", "42.00"],
                    ["adx", "28.00"],
                    ["regime_label", "TREND_UP"],
                    ["macro_bias", "ALT_PLAYGROUND"],
                    ["cvd_aligned", "true"]
                ],
                "status": "Unsupported"
            },
            {
                "layer": 5,
                "focus": "HIPOTESIS_PRA_REGISTRASI",
                "question": "Hipotesis apa yang didukung bukti untuk diuji pra-registrasi (tanpa auto-apply)?",
                "answer": "Evaluasi kohort S1 pada fokus EXCURSION_PASCA_FILL (tag=NORMAL_VARIANCE): uji apakah aturan terkait menurunkan kerugian tanpa menggerus frekuensi sampel validasi. Syarat mutlak: uji pra-registrasi lewat pipeline validate.",
                "evidence": [["mfe_r", "1.2"], ["mae_r", "1"], ["bars_in_trade", "2"], ["resolution", "15m_closed_bar"]],
                "status": "Supported"
            }
        ],
        "proposal": {
            "trade_id": "T-100",
            "symbol": "SOLUSDT",
            "strategy": "S1",
            "hypothesis": "Evaluasi kohort S1 pada fokus EXCURSION_PASCA_FILL (tag=NORMAL_VARIANCE): uji apakah aturan terkait menurunkan kerugian tanpa menggerus frekuensi sampel validasi.",
            "evidence": [["mfe_r", "1.2"], ["mae_r", "1"], ["bars_in_trade", "2"], ["resolution", "15m_closed_bar"]],
            "requires": "uji pra-registrasi lewat pipeline validate"
        }
    });
    assert_eq!(json, expected);
}
