//! Pengujian invarian `R1`..`R8` dan evaluator `apex forward-gate` (`WP-8`).

use std::collections::BTreeSet;
use std::path::PathBuf;

use apex_core::config::RiskConfig;
use apex_core::sizing::EntryKind;
use apex_core::types::Side;
use apex_oms::manager::Book;
use apex_oms::{
    append_jsonl_record, check_rule_invariants, evaluate_forward_gate_data,
    evaluate_forward_gate_files, AuditForwardStats, ExchangeInvariantState, ExecMode, ExecSource,
    FillSummary, ForwardGateFile, ForwardStage, GateVerdict, ManagedTrade, PositionInfo,
    TelemetryExitKind, TradeState, TradeTelemetry, TELEMETRY_SCHEMA_VERSION,
};
use rust_decimal::Decimal;
use rust_decimal_macros::dec;

fn base_trade(cid: &str, sym: &str, now: i64) -> ManagedTrade {
    ManagedTrade {
        client_id: cid.into(),
        symbol: sym.into(),
        strategy: "S1".into(),
        side: Side::Long,
        entry_kind: EntryKind::LimitMaker,
        planned_qty: dec!(0.4),
        filled_qty: dec!(0.4),
        entry_price: dec!(100),
        avg_fill: dec!(100),
        sl: dec!(97),
        tp: dec!(106),
        planned_risk: dec!(0.99),
        state: TradeState::Open,
        sl_algo: Some(11),
        tp_algo: Some(12),
        created_ms: now,
        expires_ms: now + 60_000,
        cancel_beyond: None,
        entry_final: true,
        knn_features: None,
        knn_signal_ms: None,
        unknown_since_ms: None,
        unknown_attempts: 0,
        fill_confirmed_at: Some(now),
        sl_placed_ms: Some(now + 200),
        validated: true,
        filled_ms: Some(now),
        closed_ms: None,
        exit_price: None,
        exit_kind: None,
        exit_hint: None,
        telemetry_exit: None,
        entry_ctx: None,
        excursion: None,
        fills: None,
        fees: Decimal::ZERO,
        funding_usd: Decimal::ZERO,
        pnl_net: None,
        close_attempts: 0,
        protect_attempts: 0,
        flat_seen_ms: None,
        last_error: None,
        adopted: false,
    }
}

#[test]
fn check_rule_invariants_detects_r1_through_r8_and_deduplicates() {
    let risk = RiskConfig::default();
    let t0 = 1_700_000_000_000i64;

    // R1 (planned_risk > 1.005) & R2 (sl 6% > 4%) & R8 (strategi non-GO di Testnet)
    let mut book = Book::default();
    let mut bad = base_trade("T1", "SOLUSDT", t0);
    bad.planned_risk = dec!(1.25);
    bad.sl = dec!(94); // 6% > 4%
    book.trades.insert("T1".into(), bad);

    // Tambah T2 & T3 untuk memicu R3 (> max_positions = 2) dan R4 (entry saat cooldown SL aktif)
    book.risk.last_sl_fill_ms = Some(t0 + 1_000);
    book.risk.cooldown_until = t0 + 3_600_000;
    let t2 = base_trade("T2", "ETHUSDT", t0 + 2_000); // dibuat saat cooldown aktif -> R4
    let t3 = base_trade("T3", "XRPUSDT", t0);
    book.trades.insert("T2".into(), t2);
    book.trades.insert("T3".into(), t3);

    let mut ex = ExchangeInvariantState::new(ExecMode::Testnet);
    ex.now_ms = t0 + 20_000;
    ex.reconcile_grace_ms = 15_000;
    ex.orphan_first_seen.insert("DOGEUSDT".into(), t0);
    ex.positions = vec![
        // SOLUSDT tanpa algo STOP_MARKET selama 20 dtk (>2000ms -> R5) dan margin isolated / lev 10x (-> R7)
        PositionInfo {
            symbol: "SOLUSDT".into(),
            amt: dec!(0.4),
            entry_price: dec!(100),
            mark_price: dec!(99),
            margin_type: "isolated".into(),
            leverage: 10,
        },
        // DOGEUSDT yatim melewati masa tenggang 15 dtk (-> R6)
        PositionInfo {
            symbol: "DOGEUSDT".into(),
            amt: dec!(10),
            entry_price: dec!(0.15),
            mark_price: dec!(0.15),
            margin_type: "CROSSED".into(),
            leverage: 25,
        },
    ];

    let v1 = check_rule_invariants(&book, &risk, &mut ex);
    let rules: BTreeSet<&str> = v1.iter().map(|v| v.rule).collect();
    for expected in ["R1", "R2", "R3", "R4", "R5", "R6", "R7", "R8"] {
        assert!(
            rules.contains(expected),
            "aturan {expected} wajib terdeteksi, dapat={rules:?}"
        );
    }

    // Panggilan kedua pada state yang sama wajib terdeduplikasi (kosong)
    let v2 = check_rule_invariants(&book, &risk, &mut ex);
    assert!(
        v2.is_empty(),
        "pelanggaran identik tidak boleh duplikat: {v2:?}"
    );
}

fn synth_demo_trade(idx: usize, t0: i64, slip_diff_r: Decimal) -> TradeTelemetry {
    let created = t0 + (idx as i64) * 40_000_000;
    TradeTelemetry {
        schema: TELEMETRY_SCHEMA_VERSION,
        trade_id: format!("D-{idx}"),
        symbol: "ETHUSDT".into(),
        strategy: "S1".into(),
        validated: true,
        side: Side::Long,
        created_ms: created,
        filled_ms: Some(created + 1_000),
        fill_confirmed_at: Some(created + 1_000),
        sl_placed_ms: Some(created + 1_400),
        unprotected_ms: Some(400),
        closed_ms: created + 900_000,
        entry_price: dec!(2500),
        exit_price: dec!(2550),
        planned_risk_usd: Decimal::ONE,
        pnl_net_usd: Decimal::ONE,
        r_multiple: Decimal::ONE,
        exit_kind: TelemetryExitKind::Tp1,
        entry_ctx: None,
        excursion: None,
        fills: FillSummary {
            entry_slip_bps: Decimal::ZERO,
            exit_slip_bps: Decimal::from(2),
            model_slip_r: dec!(0.02),
            realized_slip_r: dec!(0.02) + slip_diff_r,
            fee_usd: dec!(0.03),
            funding_usd: Decimal::ZERO,
            maker_entry: true,
            source: ExecSource::Demo,
            paper_slip_zero_by_construction: false,
        },
    }
}

fn gate_cfg_path() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap()
        .parent()
        .unwrap()
        .join("config")
        .join("forward_gate.toml")
}

#[test]
fn forward_gate_evaluates_pass_insufficient_and_every_fail_path() {
    let gf = ForwardGateFile::load(&gate_cfg_path()).expect("forward_gate.toml wajib valid");
    let th = gf.thresholds_for(ForwardStage::Demo);
    let t0 = 1_760_000_000_000i64;

    // 1. INSUFFICIENT ketika data kosong
    let empty_rep = evaluate_forward_gate_data(
        ForwardStage::Demo,
        th,
        t0,
        &[],
        AuditForwardStats::default(),
    );
    assert!(!empty_rep.overall_pass);
    assert!(empty_rep
        .rows
        .iter()
        .all(|r| r.verdict == GateVerdict::Insufficient));

    // 2. PASS penuh untuk 32 trade selama >14 hari, slip diff +0.01R <= 0.05R, 0 pelanggaran, uptime 99.5%
    let trades_ok: Vec<TradeTelemetry> = (0..32)
        .map(|i| synth_demo_trade(i, t0, dec!(0.01)))
        .collect();
    let span_15d = 15 * 86_400_000i64;
    let audit_ok = AuditForwardStats {
        event_count: 1000,
        first_ts: Some(t0),
        last_ts: Some(t0 + span_15d),
        sl_missing_count: 0,
        rule_violation_count: 0,
        orphan_reconcile_count: 0,
        max_audit_unprotected_ms: 450,
        uptime_pct: Some(99.5),
        ws_reconnect_count: 2,
        ws_backfill_bars: 4,
        ws_unfilled_gaps: 0,
    };
    let pass_rep =
        evaluate_forward_gate_data(ForwardStage::Demo, th, t0, &trades_ok, audit_ok.clone());
    assert!(
        pass_rep.overall_pass,
        "semua gerbang wajib PASS: {:?}",
        pass_rep.rows
    );

    // 3. Uji masing-masing jalur FAIL secara independen
    let trades_bad_slip: Vec<TradeTelemetry> = (0..32)
        .map(|i| synth_demo_trade(i, t0, dec!(0.09)))
        .collect();
    let rep_slip = evaluate_forward_gate_data(
        ForwardStage::Demo,
        th,
        t0,
        &trades_bad_slip,
        audit_ok.clone(),
    );
    assert!(!rep_slip.overall_pass);
    assert_eq!(rep_slip.rows[2].verdict, GateVerdict::Fail);

    let mut aud_sl = audit_ok.clone();
    aud_sl.sl_missing_count = 1;
    let rep_sl = evaluate_forward_gate_data(ForwardStage::Demo, th, t0, &trades_ok, aud_sl);
    assert_eq!(rep_sl.rows[3].verdict, GateVerdict::Fail);

    let mut aud_rv = audit_ok.clone();
    aud_rv.rule_violation_count = 1;
    let rep_rv = evaluate_forward_gate_data(ForwardStage::Demo, th, t0, &trades_ok, aud_rv);
    assert_eq!(rep_rv.rows[4].verdict, GateVerdict::Fail);

    let mut aud_unp = audit_ok.clone();
    aud_unp.max_audit_unprotected_ms = 2_500;
    let rep_unp = evaluate_forward_gate_data(ForwardStage::Demo, th, t0, &trades_ok, aud_unp);
    assert_eq!(rep_unp.rows[5].verdict, GateVerdict::Fail);

    let mut aud_orph = audit_ok.clone();
    aud_orph.orphan_reconcile_count = 1;
    let rep_orph = evaluate_forward_gate_data(ForwardStage::Demo, th, t0, &trades_ok, aud_orph);
    assert_eq!(rep_orph.rows[6].verdict, GateVerdict::Fail);

    let mut aud_up = audit_ok.clone();
    aud_up.uptime_pct = Some(82.0);
    let rep_up = evaluate_forward_gate_data(ForwardStage::Demo, th, t0, &trades_ok, aud_up);
    assert_eq!(rep_up.rows[7].verdict, GateVerdict::Fail);

    let mut aud_ws = audit_ok;
    aud_ws.ws_unfilled_gaps = 1;
    let rep_ws = evaluate_forward_gate_data(ForwardStage::Demo, th, t0, &trades_ok, aud_ws);
    assert_eq!(rep_ws.rows[8].verdict, GateVerdict::Fail);
}

#[test]
fn forward_gate_reads_synthetic_audit_and_telemetry_files_for_shadow_pass() {
    let dir = std::env::temp_dir().join(format!("apex-fg-shadow-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let tel_path = dir.join("trades_telemetry.jsonl");
    let aud_path = dir.join("audit.jsonl");

    let t0 = 1_760_000_000_000i64;
    let mut tr = synth_demo_trade(0, t0, Decimal::ZERO);
    tr.fills.source = ExecSource::Paper;
    tr.fills.paper_slip_zero_by_construction = true;
    append_jsonl_record(&tel_path, &tr).unwrap();

    // Tulis heartbeat audit tiap 15 menit selama 8 hari (> 7 hari shadow)
    let steps = 8 * 24 * 4;
    let mut lines = String::new();
    for i in 0..=steps {
        let ts = t0 + (i as i64) * 900_000;
        lines.push_str(&format!(r#"{{"ts":{ts},"event":"HEARTBEAT","data":{{}}}}"#));
        lines.push('\n');
    }
    std::fs::write(&aud_path, lines).unwrap();

    let rep = evaluate_forward_gate_files(
        ForwardStage::Shadow,
        &gate_cfg_path(),
        &tel_path,
        &aud_path,
        t0,
    )
    .unwrap();
    assert!(
        rep.overall_pass,
        "shadow 8 hari bersih wajib PASS: {:?}",
        rep.rows
    );
    let _ = std::fs::remove_dir_all(&dir);
}
