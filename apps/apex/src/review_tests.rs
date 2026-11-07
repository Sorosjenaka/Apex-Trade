//! Pengujian unit & snapshot deterministik untuk `apex review`, tuner jujur, dan playbook (`WP-7`).

use std::path::Path;

use apex_core::types::Side;
use apex_oms::{
    progressive_thresholds_for, propose_bounded_tuning, EntryContext, ExecSource, FillSummary,
    MistakeTag, PlaybookEngine, TelemetryExitKind, TradeExcursion, TradeTelemetry, TunerDecision,
    TELEMETRY_SCHEMA_VERSION,
};
use rust_decimal::Decimal;

use crate::review_cmd::{build_review_report_data, render_review_markdown};
use crate::review_stats::{iso_week_block_bootstrap_ci95, iso_year_week};

fn make_synth_trade(idx: usize, base_ms: i64) -> TradeTelemetry {
    // Sebar 40 trade ke dalam 8 minggu ISO berbeda (5 trade per minggu)
    let week_idx = (idx / 5) as i64;
    let within_week = (idx % 5) as i64;
    let created = base_ms + week_idx * 7 * 86_400_000 + within_week * 7_200_000;
    let is_win = idx % 3 == 0;
    let r = if is_win {
        Decimal::new(180, 2)
    } else {
        -Decimal::ONE
    };
    let gave_back = !is_win && idx % 7 == 0;
    let sym = match idx % 3 {
        0 => "ETHUSDT",
        1 => "SOLUSDT",
        _ => "XRPUSDT",
    };
    TradeTelemetry {
        schema: TELEMETRY_SCHEMA_VERSION,
        trade_id: format!("SYN-{idx:03}"),
        symbol: sym.into(),
        strategy: if idx % 2 == 0 {
            "S1".into()
        } else {
            "S01".into()
        },
        validated: idx % 2 == 0,
        side: if idx % 2 == 0 {
            Side::Long
        } else {
            Side::Short
        },
        created_ms: created,
        filled_ms: Some(created + 3_000),
        fill_confirmed_at: Some(created + 3_000),
        sl_placed_ms: Some(created + 3_350),
        unprotected_ms: Some(350),
        closed_ms: created + 1_800_000,
        entry_price: Decimal::from(100),
        exit_price: if is_win {
            Decimal::from(104)
        } else {
            Decimal::from(98)
        },
        planned_risk_usd: Decimal::ONE,
        pnl_net_usd: r,
        r_multiple: r,
        exit_kind: if is_win {
            TelemetryExitKind::Tp1
        } else {
            TelemetryExitKind::SlHit
        },
        entry_ctx: Some(EntryContext {
            spread_bps: Some(Decimal::from(4)),
            planned_slip_bps: Decimal::from(2),
            cvd_aligned: Some(idx % 5 != 1),
            chop: Some(if idx % 6 == 0 { 61.0 } else { 44.0 }),
            adx: Some(26.0),
            atr_pct: Some(0.011),
            regime_label: "TREND_UP".into(),
            macro_bias: "ALT_PLAYGROUND".into(),
            council: None,
            config_hash: "deadbeef01234567".into(),
            signal_ts: created,
            planned_entry: Decimal::from(100),
            planned_sl: Decimal::from(97),
            planned_tp: Decimal::from(106),
        }),
        excursion: Some(TradeExcursion {
            mfe_r: if is_win {
                Decimal::new(190, 2)
            } else if gave_back {
                Decimal::new(125, 2)
            } else {
                Decimal::new(30, 2)
            },
            mae_r: if is_win {
                Decimal::new(20, 2)
            } else {
                Decimal::ONE
            },
            bars_in_trade: 2,
            mfe_ts: Some(created + 900_000),
            mae_ts: Some(created + 900_000),
            resolution: "15m_closed_bar".into(),
        }),
        fills: FillSummary {
            entry_slip_bps: Decimal::ZERO,
            exit_slip_bps: Decimal::from(2),
            model_slip_r: Decimal::new(2, 2),
            realized_slip_r: Decimal::new(3, 2),
            fee_usd: Decimal::new(4, 2),
            funding_usd: Decimal::ZERO,
            maker_entry: true,
            source: ExecSource::Demo,
            paper_slip_zero_by_construction: false,
        },
    }
}

#[test]
fn iso_week_block_bootstrap_is_deterministic_and_handles_iso_year_boundaries() {
    let d1 = apex_core::time::parse_ymd("2026-01-01").unwrap();
    let d2 = apex_core::time::parse_ymd("2027-01-01").unwrap(); // Jumat pada minggu ISO 2026-W53
    assert_eq!(iso_year_week(d1), (2026, 1));
    assert_eq!(iso_year_week(d2), (2026, 53));

    let trades: Vec<TradeTelemetry> = (0..40).map(|i| make_synth_trade(i, d1)).collect();
    let pts: Vec<(i64, f64)> = trades
        .iter()
        .map(|t| (t.closed_ms, apex_core::num::to_f64(t.r_multiple)))
        .collect();
    let run_a = iso_week_block_bootstrap_ci95(&pts, 10_000).unwrap();
    let run_b = iso_week_block_bootstrap_ci95(&pts, 10_000).unwrap();
    assert_eq!(
        run_a, run_b,
        "seed tetap wajib menghasilkan CI yang 100% identik"
    );
    assert_eq!(run_a.2, 8);
    assert!(run_a.1.is_some());
}

#[test]
fn empty_and_small_samples_are_flagged_and_never_contain_forbidden_phrases() {
    let base = apex_core::time::parse_ymd("2026-02-01").unwrap();
    let forbidden = ["profitable", "edge terbukti"];

    for count in [0usize, 15, 40, 99] {
        let cohort: Vec<TradeTelemetry> = (0..count)
            .map(|i| {
                let mut t = make_synth_trade(i, base);
                // Buat semua menang pun tetap DILARANG mengklaim profitable / edge terbukti saat n < 100
                t.pnl_net_usd = Decimal::TWO;
                t.r_multiple = Decimal::TWO;
                t.exit_kind = TelemetryExitKind::Tp1;
                t
            })
            .collect();
        let data = build_review_report_data("2026-02-01", None, None, &cohort, &[], None).unwrap();
        assert!(
            data.summary.profit_factor.is_none(),
            "PF wajib None bila n < 100 (n={count})"
        );
        assert_eq!(data.conclusion, "INSUFFICIENT");
        if count > 0 && count < 30 {
            assert!(data.summary.sample_status.contains("< 30"));
        }
        let md = render_review_markdown(&data);
        let js = serde_json::to_string(&data).unwrap();
        let lower_md = md.to_ascii_lowercase();
        let lower_js = js.to_ascii_lowercase();
        for phrase in forbidden {
            assert!(
                !lower_md.contains(phrase) && !lower_js.contains(phrase),
                "frasa terlarang '{phrase}' ditemukan pada n={count}"
            );
        }
    }
}

#[test]
fn review_markdown_snapshot_on_40_synthetic_trades() {
    let base = apex_core::time::parse_ymd("2026-02-02").unwrap();
    let cohort: Vec<TradeTelemetry> = (0..40).map(|i| make_synth_trade(i, base)).collect();
    let aud_dir = std::env::temp_dir().join(format!("apex-rev-snap-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&aud_dir);
    std::fs::create_dir_all(&aud_dir).unwrap();
    let aud_file = aud_dir.join("audit.jsonl");
    std::fs::write(
        &aud_file,
        format!(
            "{{\"ts\":{},\"event\":\"SKIP\",\"data\":{{\"reason\":\"REGIME\"}}}}\n{{\"ts\":{},\"event\":\"SKIP\",\"data\":{{\"reason\":\"REGIME\"}}}}\n{{\"ts\":{},\"event\":\"SKIP\",\"data\":{{\"reason\":\"COOLDOWN\"}}}}\n",
            base + 1000,
            base + 2000,
            base + 3000
        ),
    )
    .unwrap();

    let rep1 = build_review_report_data(
        "2026-02-02",
        Some("2026-04-30"),
        None,
        &cohort,
        &[aud_file.as_path()],
        None,
    )
    .unwrap();
    let md1 = render_review_markdown(&rep1);
    let rep2 = build_review_report_data(
        "2026-02-02",
        Some("2026-04-30"),
        None,
        &cohort,
        &[aud_file.as_path()],
        None,
    )
    .unwrap();
    let md2 = render_review_markdown(&rep2);

    // Determinisme snapshot penuh
    assert_eq!(md1, md2);
    assert!(md1.contains("# Laporan Review Berkala Apex-Trade (`2026-02-02` s/d `2026-04-30`)"));
    assert!(md1.contains("Status sampel: **INSUFFICIENT_FOR_PF_AND_CONCLUSION (n=40 < 100)** (`n = 40`, minggu ISO = `8`)"));
    assert!(md1.contains("| SKIP_REGIME | 2 |"));
    assert!(md1.contains("| SKIP_COOLDOWN | 1 |"));
    assert!(md1.contains("INSUFFICIENT (n<10)"));
    assert!(md1.contains("Profit Factor: `INSUFFICIENT (PF hanya dihitung bila n >= 100)`"));
    assert!(md1.contains("Tingkat *gave-back* (`MFE >= 1.0R` lalu `SlHit`): `4` trade (`10.00%`)"));
    assert!(md1.contains(
        "**INSUFFICIENT** — TIDAK ADA KESIMPULAN: sampel n=40 (<100 minimum inferensi statistik)."
    ));

    let _ = std::fs::remove_dir_all(&aud_dir);
}

#[test]
fn honest_tuner_progressive_advisory_and_playbook_counts_persistence() {
    // 7b: Tuner menolak <100 sampel dan menolak CI yang mencakup 0, serta tidak pernah mengubah angka
    let p_small = propose_bounded_tuning("S-01", 1.7, 45, 38.0, Some(-0.1), Some((-0.3, 0.1)));
    assert_eq!(p_small.status, "NO_PROPOSAL");
    assert_eq!(p_small.from, p_small.to);
    assert_eq!(p_small.bounded_pct, 0.0);
    assert!(matches!(p_small.decision, TunerDecision::NoProposal { .. }));

    let p_zero_ci = propose_bounded_tuning("S-01", 1.7, 120, 48.0, Some(0.04), Some((-0.05, 0.14)));
    assert_eq!(p_zero_ci.status, "NO_PROPOSAL");
    assert_eq!(p_zero_ci.from, 1.7);
    assert_eq!(p_zero_ci.to, 1.7);

    let p_sig = propose_bounded_tuning("S-01", 1.7, 120, 56.0, Some(0.18), Some((0.04, 0.31)));
    assert_eq!(p_sig.status, "REVIEW_RECOMMENDED");
    assert_eq!(p_sig.from, 1.7);
    assert_eq!(p_sig.to, 1.7);
    assert_eq!(p_sig.bounded_pct, 0.0);

    // 7c: Progressive policy wajib bertanda advisory = true
    for lvl in 0..=5 {
        let pt = progressive_thresholds_for(lvl);
        assert!(pt.advisory);
        let js = serde_json::to_value(&pt).unwrap();
        assert_eq!(js["advisory"], serde_json::json!(true));
    }

    // 7d: Playbook menyimpan mistake_counts ke playbook_counts.json tanpa mengubah PB-01..03
    let dir = std::env::temp_dir().join(format!("apex-pb-counts-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let pb_path = dir.join("playbook_counts.json");

    let mut pb = PlaybookEngine::default();
    let orig_rules = pb.rules.clone();
    for _ in 0..3 {
        pb.record_mistake(MistakeTag::ChopWhipsaw);
    }
    pb.save_counts(Path::new(&pb_path), 1_700_000_000_000)
        .unwrap();
    assert_eq!(
        pb.rules, orig_rules,
        "aturan PB-01..03 tidak boleh berubah otomatis"
    );

    let mut pb2 = PlaybookEngine::default();
    pb2.load_counts(Path::new(&pb_path)).unwrap();
    assert_eq!(pb2.mistake_counts.get("CHOP_WHIPSAW"), Some(&3));
    let snap = pb2.build_counts_snapshot(1_700_000_000_000);
    assert_eq!(snap.proposals.len(), 1);

    let _ = std::fs::remove_dir_all(&dir);
}
