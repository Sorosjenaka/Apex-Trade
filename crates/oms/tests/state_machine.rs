//! Test state machine OMS memakai MockExchange (tidak ada jaringan, tidak ada key).
use apex_core::audit::AuditLog;
use apex_core::config::{EngineConfig, OrphanPolicy, RiskConfig};
use apex_core::risk::ExitKind;
use apex_core::sizing::{size_order, EntryKind, SizingInput};
use apex_core::{Side, SymbolFilters};
use apex_oms::mock::MockExchange;
use apex_oms::store::StateStore;
use apex_oms::*;
use rust_decimal::Decimal;
use rust_decimal_macros::dec;
use std::collections::BTreeMap;

const T0: i64 = 1_767_225_600_000; // 2026-01-01 00:00 UTC

fn filters() -> BTreeMap<String, SymbolFilters> {
    let mut m = BTreeMap::new();
    m.insert(
        "ETHUSDT".into(),
        SymbolFilters::synthetic("ETHUSDT", dec!(0.01), dec!(0.001), dec!(5)),
    );
    m
}

fn tmpdir(name: &str) -> std::path::PathBuf {
    let d = std::env::temp_dir().join(format!("apex-oms-test-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).unwrap();
    d
}

fn mgr(name: &str, ex: MockExchange, policy: OrphanPolicy) -> OrderManager<MockExchange> {
    let d = tmpdir(name);
    let cfg = EngineConfig {
        kill_file: d.join("KILL"),
        state_dir: d.clone(),
        orphan_policy: policy,
        ..Default::default()
    };
    let mut m = OrderManager::new(
        ex,
        ExecMode::Paper,
        cfg,
        RiskConfig::default(),
        StateStore::file(&d.join("oms_state.json")),
        AuditLog::in_memory(),
        filters(),
    )
    .unwrap();
    m.max_backoff_ms = 20;
    m
}

fn intent(side: Side, entry: Decimal, sl: Decimal, kind: EntryKind) -> EntryIntent {
    let f = filters()["ETHUSDT"].clone();
    let rc = RiskConfig::default();
    let sizing = size_order(&SizingInput {
        side,
        entry,
        sl,
        kind,
        filters: &f,
        risk: &rc,
    })
    .unwrap();
    EntryIntent {
        symbol: "ETHUSDT".into(),
        strategy: "S1".into(),
        sizing,
        kind,
        ttl_ms: 3_600_000,
        cancel_beyond: None,
        knn_features: None,
        knn_signal_ms: None,
    }
}

async fn pump(m: &mut OrderManager<MockExchange>, now: i64) {
    for ev in m.ex.drain_events() {
        m.on_event(ev, now).await;
    }
}

#[tokio::test]
async fn market_entry_filled_ack_places_sl_tp_algo() {
    let ex = MockExchange::new(60.0);
    ex.set_price("ETHUSDT", dec!(2500), T0);
    let mut m = mgr("mkt", ex, OrphanPolicy::PlaceSl);
    m.boot(T0).await.unwrap();
    let cid = m
        .submit(
            intent(Side::Long, dec!(2500), dec!(2440), EntryKind::Market),
            T0,
        )
        .await
        .unwrap();
    pump(&mut m, T0).await;
    let t = m.book.trades[&cid].clone();
    assert_eq!(t.state, TradeState::Open);
    assert!(t.sl_algo.is_some() && t.tp_algo.is_some());
    assert!(m.ex.is_one_way());
    assert_eq!(m.ex.leverage_of("ETHUSDT"), Some(25));
    assert_eq!(m.ex.margin_of("ETHUSDT").as_deref(), Some("CROSSED"));
    let algos = m.ex.algos();
    assert_eq!(algos.len(), 2);
    assert!(algos.iter().all(|a| a.close_position
        && a.qty.is_none()
        && a.side == "SELL"
        && a.client_algo_id.len() <= 36));
    assert!(algos
        .iter()
        .any(|a| a.kind == AlgoKind::StopMarket && a.trigger == t.sl));
}

#[tokio::test]
async fn limit_gtx_ack_waits_for_fill_event() {
    let ex = MockExchange::new(60.0);
    ex.set_price("ETHUSDT", dec!(2510), T0);
    let mut m = mgr("lim", ex, OrphanPolicy::PlaceSl);
    m.boot(T0).await.unwrap();
    let cid = m
        .submit(
            intent(Side::Long, dec!(2500), dec!(2440), EntryKind::LimitMaker),
            T0,
        )
        .await
        .unwrap();
    assert_eq!(m.book.trades[&cid].state, TradeState::PendingEntry);
    assert_eq!(m.ex.algo_count(), 0, "tidak boleh ada SL sebelum fill");
    m.ex.set_price("ETHUSDT", dec!(2499.5), T0 + 60_000);
    pump(&mut m, T0 + 60_000).await;
    assert_eq!(m.book.trades[&cid].state, TradeState::Open);
    assert_eq!(m.book.trades[&cid].avg_fill, dec!(2500));
    assert_eq!(m.ex.algo_count(), 2);
}

#[tokio::test]
async fn post_only_reject_is_dropped() {
    let ex = MockExchange::new(60.0);
    ex.set_price("ETHUSDT", dec!(2490), T0);
    let mut m = mgr("gtx", ex, OrphanPolicy::PlaceSl);
    let r = m
        .submit(
            intent(Side::Long, dec!(2500), dec!(2440), EntryKind::LimitMaker),
            T0,
        )
        .await;
    assert!(r.is_err());
    assert_eq!(m.active().count(), 0);
}

#[tokio::test]
async fn market_ack_only_then_event() {
    let ex = MockExchange::new(60.0);
    ex.set_price("ETHUSDT", dec!(2500), T0);
    ex.knobs(|k| k.market_ack_only = true);
    let mut m = mgr("ack", ex, OrphanPolicy::PlaceSl);
    let cid = m
        .submit(
            intent(Side::Short, dec!(2500), dec!(2560), EntryKind::Market),
            T0,
        )
        .await
        .unwrap();
    assert_eq!(
        m.book.trades[&cid].state,
        TradeState::PendingEntry,
        "ACK NEW ≠ FILLED"
    );
    pump(&mut m, T0).await;
    assert_eq!(m.book.trades[&cid].state, TradeState::Open);
    assert!(m.ex.algos().iter().all(|a| a.side == "BUY"));
}

#[tokio::test]
async fn sl_trigger_closes_and_starts_cooldown() {
    let ex = MockExchange::new(60.0);
    ex.set_price("ETHUSDT", dec!(2500), T0);
    let mut m = mgr("sl", ex, OrphanPolicy::PlaceSl);
    let cid = m
        .submit(
            intent(Side::Long, dec!(2500), dec!(2440), EntryKind::Market),
            T0,
        )
        .await
        .unwrap();
    pump(&mut m, T0).await;
    m.ex.set_price("ETHUSDT", dec!(2439), T0 + 900_000);
    pump(&mut m, T0 + 900_000).await;
    assert!(!m.book.trades.contains_key(&cid));
    let c = m.book.closed.last().unwrap();
    assert_eq!(c.exit_kind, Some(ExitKind::StopLoss));
    let pnl = c.pnl_net.unwrap();
    assert!(pnl < -0.8 && pnl > -1.2, "pnl {pnl}");
    assert_eq!(m.book.risk.daily_sl_count, 1);
    assert!(m.book.risk.in_cooldown(T0 + 900_000 + 30 * 60_000));
    assert!(!m.book.risk.in_cooldown(T0 + 900_000 + 61 * 60_000));
    assert_eq!(m.ex.algo_count(), 0, "TP sibling harus dibatalkan");
    assert!(m.ex.position("ETHUSDT").is_zero());
}

#[tokio::test]
async fn tp_trigger_closes_without_cooldown() {
    let ex = MockExchange::new(60.0);
    ex.set_price("ETHUSDT", dec!(2500), T0);
    let mut m = mgr("tp", ex, OrphanPolicy::PlaceSl);
    m.submit(
        intent(Side::Long, dec!(2500), dec!(2440), EntryKind::Market),
        T0,
    )
    .await
    .unwrap();
    pump(&mut m, T0).await;
    m.ex.set_price("ETHUSDT", dec!(2700), T0 + 900_000);
    pump(&mut m, T0 + 900_000).await;
    let c = m.book.closed.last().unwrap();
    assert_eq!(c.exit_kind, Some(ExitKind::TakeProfit));
    assert!(c.pnl_net.unwrap() > 1.5);
    assert_eq!(m.book.risk.daily_sl_count, 0);
    assert!(!m.book.risk.in_cooldown(T0 + 900_000));
    assert_eq!(m.ex.algo_count(), 0);
}

#[tokio::test]
async fn sl_failure_flattens() {
    let ex = MockExchange::new(60.0);
    ex.set_price("ETHUSDT", dec!(2500), T0);
    ex.knobs(|k| k.fail_algo = 10);
    let mut m = mgr("slfail", ex, OrphanPolicy::PlaceSl);
    m.submit(
        intent(Side::Long, dec!(2500), dec!(2440), EntryKind::Market),
        T0,
    )
    .await
    .unwrap();
    pump(&mut m, T0).await;
    assert!(
        m.ex.position("ETHUSDT").is_zero(),
        "posisi tanpa SL harus di-flatten"
    );
    assert_eq!(
        m.book.closed.last().unwrap().exit_kind,
        Some(ExitKind::StopLoss)
    );
    assert_eq!(m.book.risk.daily_sl_count, 1);
}

#[tokio::test]
async fn failed_flatten_keeps_position_and_kills() {
    let ex = MockExchange::new(60.0);
    ex.set_price("ETHUSDT", dec!(2500), T0);
    ex.knobs(|k| {
        k.fail_algo = 3;
        k.fail_reduce_only = 1;
    });
    let mut m = mgr("unprot", ex, OrphanPolicy::PlaceSl);
    let cid = m
        .submit(
            intent(Side::Long, dec!(2500), dec!(2440), EntryKind::Market),
            T0,
        )
        .await
        .unwrap();
    let t = &m.book.trades[&cid];
    assert_eq!(t.state, TradeState::Unprotected);
    assert!(m.kill.is_engaged());
    assert!(!m.ex.position("ETHUSDT").is_zero());
    // entry baru diblokir kill switch
    assert!(m
        .submit(
            intent(Side::Long, dec!(2500), dec!(2440), EntryKind::Market),
            T0
        )
        .await
        .is_err());
    // tick berikut: exchange pulih → SL terpasang, posisi tetap terkelola
    m.tick(T0 + 5_000).await;
    let t = &m.book.trades[&cid];
    assert_eq!(t.state, TradeState::Open);
    assert!(t.sl_algo.is_some());
}

#[tokio::test]
async fn failed_manual_close_does_not_drop_position() {
    let ex = MockExchange::new(60.0);
    ex.set_price("ETHUSDT", dec!(2500), T0);
    let mut m = mgr("close", ex, OrphanPolicy::PlaceSl);
    let cid = m
        .submit(
            intent(Side::Long, dec!(2500), dec!(2440), EntryKind::Market),
            T0,
        )
        .await
        .unwrap();
    pump(&mut m, T0).await;
    m.ex.knobs(|k| k.fail_reduce_only = 1);
    assert!(m
        .close_trade(&cid, ExitKind::Manual, T0 + 1000)
        .await
        .is_err());
    assert!(m.book.trades.contains_key(&cid));
    assert_eq!(m.book.trades[&cid].state, TradeState::Open);
    assert!(m.book.trades[&cid].sl_algo.is_some(), "SL tetap ada");
    m.close_trade(&cid, ExitKind::Manual, T0 + 2000)
        .await
        .unwrap();
    assert!(!m.book.trades.contains_key(&cid));
    assert_eq!(m.ex.algo_count(), 0);
}

#[tokio::test]
async fn reconcile_orphan_places_sl() {
    let ex = MockExchange::new(60.0);
    ex.inject_position("ETHUSDT", dec!(0.01), dec!(2500));
    ex.set_price("ETHUSDT", dec!(2500), T0);
    let mut m = mgr("orph", ex, OrphanPolicy::PlaceSl);
    m.boot(T0).await.unwrap();
    let t = m.active().next().unwrap().clone();
    assert!(t.adopted && t.sl_algo.is_some());
    let sl =
        m.ex.algos()
            .into_iter()
            .find(|a| a.kind == AlgoKind::StopMarket)
            .unwrap();
    assert_eq!(sl.trigger, dec!(2400.00)); // SL 4% (sl_max) dari entry
}

#[tokio::test]
async fn reconcile_orphan_flatten_policy() {
    let ex = MockExchange::new(60.0);
    ex.inject_position("ETHUSDT", dec!(-0.01), dec!(2500));
    ex.set_price("ETHUSDT", dec!(2500), T0);
    let mut m = mgr("orphf", ex, OrphanPolicy::Flatten);
    m.boot(T0).await.unwrap();
    assert!(m.ex.position("ETHUSDT").is_zero());
    assert_eq!(m.active().count(), 0);
}

#[tokio::test]
async fn unknown_outcome_resolved_by_query() {
    let ex = MockExchange::new(60.0);
    ex.set_price("ETHUSDT", dec!(2500), T0);
    ex.knobs(|k| k.unknown_place = 1);
    let mut m = mgr("unk", ex, OrphanPolicy::PlaceSl);
    let cid = m
        .submit(
            intent(Side::Long, dec!(2500), dec!(2440), EntryKind::Market),
            T0,
        )
        .await
        .unwrap();
    assert_eq!(m.book.trades[&cid].state, TradeState::Open);
    assert!(m.book.trades[&cid].sl_algo.is_some());
}

#[tokio::test]
async fn rate_limit_backoff_then_success() {
    let ex = MockExchange::new(60.0);
    ex.set_price("ETHUSDT", dec!(2500), T0);
    ex.knobs(|k| k.rate_limit = 2);
    let mut m = mgr("rl", ex, OrphanPolicy::PlaceSl);
    let cid = m
        .submit(
            intent(Side::Long, dec!(2500), dec!(2440), EntryKind::Market),
            T0,
        )
        .await
        .unwrap();
    assert_eq!(m.book.trades[&cid].state, TradeState::Open);
    assert!(m.audit.count("RATE_LIMIT_BACKOFF") >= 2);
}

#[tokio::test]
async fn pending_ttl_expires_and_kill_blocks() {
    let ex = MockExchange::new(60.0);
    ex.set_price("ETHUSDT", dec!(2510), T0);
    let mut m = mgr("ttl", ex, OrphanPolicy::PlaceSl);
    let cid = m
        .submit(
            intent(Side::Long, dec!(2500), dec!(2440), EntryKind::LimitMaker),
            T0,
        )
        .await
        .unwrap();
    m.tick(T0 + 3_600_001).await;
    assert!(!m.book.trades.contains_key(&cid));
    m.engage_kill("test", false, T0 + 3_700_000).await;
    assert!(matches!(
        m.submit(
            intent(Side::Long, dec!(2500), dec!(2440), EntryKind::LimitMaker),
            T0
        )
        .await,
        Err(apex_oms::manager::OmsError::Killed)
    ));
    m.kill.release().unwrap();
    assert!(m
        .submit(
            intent(Side::Long, dec!(2500), dec!(2440), EntryKind::LimitMaker),
            T0
        )
        .await
        .is_ok());
}

#[tokio::test]
async fn state_persists_across_restart() {
    let d = tmpdir("persist");
    let cfg = EngineConfig {
        kill_file: d.join("KILL"),
        ..Default::default()
    };
    let store = StateStore::file(&d.join("oms_state.json"));
    let ex = MockExchange::new(60.0);
    ex.set_price("ETHUSDT", dec!(2500), T0);
    let mut m = OrderManager::new(
        ex,
        ExecMode::Paper,
        cfg.clone(),
        RiskConfig::default(),
        store.clone(),
        AuditLog::in_memory(),
        filters(),
    )
    .unwrap();
    let cid = m
        .submit(
            intent(Side::Long, dec!(2500), dec!(2440), EntryKind::Market),
            T0,
        )
        .await
        .unwrap();
    let ex2 = m.ex;
    let mut m2 = OrderManager::new(
        ex2,
        ExecMode::Paper,
        cfg,
        RiskConfig::default(),
        store,
        AuditLog::in_memory(),
        filters(),
    )
    .unwrap();
    assert_eq!(m2.book.trades[&cid].state, TradeState::Open);
    m2.reconcile(T0 + 1).await.unwrap();
    assert_eq!(m2.active().count(), 1, "tidak ada duplikasi saat reconcile");
}

#[test]
fn non_paper_requires_arm() {
    let cfg = EngineConfig {
        mode: apex_core::Mode::Testnet,
        arm: false,
        ..Default::default()
    };
    let r = OrderManager::new(
        MockExchange::new(60.0),
        ExecMode::Testnet,
        cfg,
        RiskConfig::default(),
        StateStore::memory(),
        AuditLog::in_memory(),
        filters(),
    );
    assert!(r.is_err());
}

#[tokio::test]
async fn cancel_beyond_uses_closed_bar_and_preserves_fills() {
    let ex = MockExchange::new(60.0);
    ex.set_price("ETHUSDT", dec!(2510), T0);
    let mut m = mgr("cancel-beyond", ex, OrphanPolicy::PlaceSl);
    let mut it = intent(Side::Long, dec!(2500), dec!(2440), EntryKind::LimitMaker);
    it.cancel_beyond = Some(dec!(2490));
    let cid = m.submit(it, T0).await.unwrap();
    m.on_closed_price("ETHUSDT", dec!(2480), T0).await.unwrap();
    assert!(m.book.trades.contains_key(&cid));
    m.on_closed_price("ETHUSDT", dec!(2490), T0 + 1)
        .await
        .unwrap();
    assert!(m.book.trades.contains_key(&cid));
    m.on_closed_price("ETHUSDT", dec!(2480), T0 + 2)
        .await
        .unwrap();
    assert!(!m.book.trades.contains_key(&cid));
    let mut it = intent(Side::Long, dec!(2500), dec!(2440), EntryKind::LimitMaker);
    it.cancel_beyond = Some(dec!(2490));
    let cid = m.submit(it, T0 + 3).await.unwrap();
    m.ex.set_price("ETHUSDT", dec!(2499), T0 + 4);
    m.on_closed_price("ETHUSDT", dec!(2480), T0 + 5)
        .await
        .unwrap();
    assert_eq!(m.book.trades[&cid].state, TradeState::Open);
    assert!(m.book.trades[&cid].sl_algo.is_some());
}

#[tokio::test]
async fn cancel_beyond_short_and_other_symbol() {
    let ex = MockExchange::new(60.0);
    ex.set_price("ETHUSDT", dec!(2490), T0);
    let mut m = mgr("cancel-beyond-short", ex, OrphanPolicy::PlaceSl);
    let mut it = intent(Side::Short, dec!(2500), dec!(2560), EntryKind::LimitMaker);
    it.cancel_beyond = Some(dec!(2510));
    let cid = m.submit(it, T0).await.unwrap();
    m.on_closed_price("BTCUSDT", dec!(2520), T0 + 1)
        .await
        .unwrap();
    m.on_closed_price("ETHUSDT", dec!(2510), T0 + 1)
        .await
        .unwrap();
    assert!(m.book.trades.contains_key(&cid));
    m.on_closed_price("ETHUSDT", dec!(2520), T0 + 2)
        .await
        .unwrap();
    assert!(!m.book.trades.contains_key(&cid));
}

#[tokio::test]
async fn cancel_beyond_cancels_unfilled_remainder_of_partial_entry() {
    let ex = MockExchange::new(60.0);
    ex.set_price("ETHUSDT", dec!(2510), T0);
    let mut m = mgr("cancel-beyond-partial", ex, OrphanPolicy::PlaceSl);
    let mut it = intent(Side::Long, dec!(2500), dec!(2440), EntryKind::LimitMaker);
    it.cancel_beyond = Some(dec!(2490));
    let qty = it.sizing.qty / dec!(2);
    let cid = m.submit(it, T0).await.unwrap();
    m.ex.partial_fill(&cid, qty, T0 + 1);
    pump(&mut m, T0 + 1).await;
    assert_eq!(m.book.trades[&cid].state, TradeState::Open);
    m.on_closed_price("ETHUSDT", dec!(2480), T0 + 2)
        .await
        .unwrap();
    assert_eq!(m.book.trades[&cid].filled_qty, qty);
    assert!(m.book.trades[&cid].entry_final);
    assert!(m.book.trades[&cid].sl_algo.is_some());
    assert!(m.ex.calls().iter().any(|c| c == &format!("cancel {cid}")));
}

#[tokio::test]
async fn knn_features_survive_close_and_state_reload() {
    let ex = MockExchange::new(60.0);
    ex.set_price("ETHUSDT", dec!(2500), T0);
    let mut m = mgr("knn-persist", ex, OrphanPolicy::PlaceSl);
    let mut it = intent(Side::Long, dec!(2500), dec!(2440), EntryKind::Market);
    it.strategy = "S5_RMP".into();
    it.knn_features = Some([10., 20., 30., 40., 50.]);
    it.knn_signal_ms = Some(T0 - 900_000);
    let cid = m.submit(it, T0).await.unwrap();
    m.close_trade(&cid, ExitKind::Manual, T0 + 1_000)
        .await
        .unwrap();
    let saved: apex_oms::manager::Book = StateStore::file(&m.cfg.state_dir.join("oms_state.json"))
        .load()
        .unwrap();
    let trade = saved.closed.last().unwrap();
    assert_eq!(trade.knn_features, Some([10., 20., 30., 40., 50.]));
    assert_eq!(trade.knn_signal_ms, Some(T0 - 900_000));
    assert!(trade.closed_ms.is_some());
}

// ---------------------------------------------------------------- WP-2 durability & persist-before-send

#[tokio::test]
async fn submit_tidak_memanggil_exchange_bila_persist_gagal() {
    let ex = MockExchange::new(60.0);
    ex.set_price("ETHUSDT", dec!(2500), T0);
    let mut m = mgr("persist-fail-submit", ex, OrphanPolicy::PlaceSl);
    m.store.set_fail_saves(1);
    let res = m
        .submit(
            intent(Side::Long, dec!(2500), dec!(2440), EntryKind::Market),
            T0,
        )
        .await;
    assert!(
        matches!(res, Err(apex_oms::manager::OmsError::Store(_))),
        "expected Store error, got {res:?}"
    );
    assert_eq!(
        m.ex.calls().len(),
        0,
        "exchange tidak boleh dipanggil saat persist gagal"
    );
    assert_eq!(
        m.active().count(),
        0,
        "PendingEntry di memori harus dibatalkan"
    );
    assert_eq!(m.audit.count("ENTRY_ABORTED_NO_PERSIST"), 1);
    assert_eq!(m.audit.count("STATE_SAVE_ERROR"), 1);
}

#[tokio::test]
async fn save_atomik_dan_roundtrip() {
    let d = tmpdir("atomic-roundtrip");
    let primary = d.join("oms_state.json");
    let store = StateStore::file(&primary);
    let ex = MockExchange::new(60.0);
    ex.set_price("ETHUSDT", dec!(2500), T0);
    let cfg = EngineConfig {
        kill_file: d.join("KILL"),
        state_dir: d.clone(),
        ..Default::default()
    };
    let mut m = OrderManager::new(
        ex,
        ExecMode::Paper,
        cfg,
        RiskConfig::default(),
        store.clone(),
        AuditLog::in_memory(),
        filters(),
    )
    .unwrap();
    let cid = m
        .submit(
            intent(Side::Long, dec!(2500), dec!(2440), EntryKind::Market),
            T0,
        )
        .await
        .unwrap();
    pump(&mut m, T0).await;
    assert!(primary.exists(), "file utama harus ada");
    assert!(
        !StateStore::tmp_path(&primary).exists(),
        "file tmp harus sudah di-rename"
    );
    assert!(
        StateStore::bak_path(&primary).exists(),
        "file .bak harus ada setelah ≥2 kali save"
    );
    let loaded: apex_oms::manager::Book = store.load().unwrap();
    assert_eq!(loaded.trades[&cid].state, TradeState::Open);
}

#[tokio::test]
async fn pulih_dari_bak_bila_utama_korup() {
    let d = tmpdir("recover-bak");
    let primary = d.join("oms_state.json");
    let store = StateStore::file(&primary);
    let ex = MockExchange::new(60.0);
    ex.set_price("ETHUSDT", dec!(2500), T0);
    let cfg = EngineConfig {
        kill_file: d.join("KILL"),
        state_dir: d.clone(),
        ..Default::default()
    };
    let mut m = OrderManager::new(
        ex,
        ExecMode::Paper,
        cfg.clone(),
        RiskConfig::default(),
        store.clone(),
        AuditLog::in_memory(),
        filters(),
    )
    .unwrap();
    let cid = m
        .submit(
            intent(Side::Long, dec!(2500), dec!(2440), EntryKind::Market),
            T0,
        )
        .await
        .unwrap();
    pump(&mut m, T0).await;
    // Save tambahan via tick agar salinan terakhir dengan state Open juga tersalin ke .json.bak
    m.tick(T0 + 1_000).await;

    // 1. Korupsi file utama -> pulih dari .bak + audit STATE_RECOVERED_FROM_BAK
    std::fs::write(&primary, b"{corrupt json").unwrap();
    let audit1 = AuditLog::in_memory();
    let m_corrupt = OrderManager::new(
        MockExchange::new(60.0),
        ExecMode::Paper,
        cfg.clone(),
        RiskConfig::default(),
        store.clone(),
        audit1,
        filters(),
    )
    .unwrap();
    assert_eq!(m_corrupt.book.trades[&cid].state, TradeState::Open);
    assert_eq!(m_corrupt.audit.count("STATE_RECOVERED_FROM_BAK"), 1);

    // 2. Hapus file utama -> pulih dari .bak + audit STATE_RECOVERED_FROM_BAK
    std::fs::remove_file(&primary).unwrap();
    let audit2 = AuditLog::in_memory();
    let m_missing = OrderManager::new(
        MockExchange::new(60.0),
        ExecMode::Paper,
        cfg,
        RiskConfig::default(),
        store,
        audit2,
        filters(),
    )
    .unwrap();
    assert_eq!(m_missing.book.trades[&cid].state, TradeState::Open);
    assert_eq!(m_missing.audit.count("STATE_RECOVERED_FROM_BAK"), 1);
}

#[test]
fn fixture_state_lama_tetap_terbaca() {
    let fixture = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests")
        .join("fixtures")
        .join("oms_state_v0.json");
    let loaded: apex_oms::manager::Book = StateStore::file(&fixture).load().unwrap();
    assert_eq!(loaded.closed.len(), 1);
    assert_eq!(
        loaded
            .trades
            .values()
            .filter(|t| t.state == TradeState::Open)
            .count(),
        1
    );
    assert_eq!(
        loaded
            .trades
            .values()
            .filter(|t| t.state == TradeState::PendingEntry)
            .count(),
        1
    );
}

#[tokio::test]
async fn tiga_kegagalan_berturut_memicu_kill() {
    let ex = MockExchange::new(60.0);
    ex.set_price("ETHUSDT", dec!(2500), T0);
    let mut m = mgr("three-save-fails", ex, OrphanPolicy::PlaceSl);
    m.store.set_fail_saves(3);
    m.tick(T0 + 1_000).await;
    assert!(!m.kill.is_engaged());
    assert_eq!(m.consecutive_save_failures, 1);
    m.tick(T0 + 2_000).await;
    assert!(!m.kill.is_engaged());
    assert_eq!(m.consecutive_save_failures, 2);
    m.tick(T0 + 3_000).await;
    assert!(
        m.kill.is_engaged(),
        "3 kegagalan simpan berturut-turut wajib mengaktifkan kill switch"
    );
    assert_eq!(m.kill.reason().as_deref(), Some("STATE_SAVE_FAILED"));
    assert_eq!(m.audit.count("KILL_ENGAGED"), 1);
}

// ---------------------------------------------------------------- WP-3 ketahanan OMS (F7-F10)

#[tokio::test]
async fn unknown_submit_disimpan_dan_diselesaikan_lewat_tick_unknown() {
    let ex = MockExchange::new(60.0);
    ex.set_price("ETHUSDT", dec!(2500), T0);
    // 1 panggilan place_order -> Unknown; 2 query_order pertama (1 di submit, 1 di tick_unknown +1s) -> None
    ex.knobs(|k| {
        k.unknown_place = 1;
        k.unknown_invisible_polls = 2;
    });
    let mut m = mgr("unknown-submit-resolve", ex, OrphanPolicy::PlaceSl);
    let cid = m
        .submit(
            intent(Side::Long, dec!(2500), dec!(2440), EntryKind::Market),
            T0,
        )
        .await
        .unwrap();

    // Segera setelah submit: state adalah UnknownSubmit, memblokir simbol & dihitung ke pending_count
    assert_eq!(m.book.trades[&cid].state, TradeState::UnknownSubmit);
    assert!(m.symbol_busy("ETHUSDT"));
    assert_eq!(m.pending_count(), 1);
    assert_eq!(m.audit.count("ENTRY_UNKNOWN_QUEUED"), 1);

    // Sebelum jadwal backoff pertama (1000 ms) tidak ada query tambahan
    let q_before =
        m.ex.calls()
            .iter()
            .filter(|c| c.starts_with("query_order"))
            .count();
    m.tick_unknown(T0 + 500).await;
    let q_mid =
        m.ex.calls()
            .iter()
            .filter(|c| c.starts_with("query_order"))
            .count();
    assert_eq!(q_before, q_mid);

    // Percobaan #1 di +1000 ms masih invisible -> tetap UnknownSubmit
    m.tick_unknown(T0 + 1_000).await;
    assert_eq!(m.book.trades[&cid].state, TradeState::UnknownSubmit);
    assert_eq!(m.book.trades[&cid].unknown_attempts, 1);

    // Percobaan #2 di +3000 ms (1s + 2s) menemukan order terisi -> transisi ke Open + SL dipasang
    m.tick_unknown(T0 + 3_000).await;
    assert_eq!(m.book.trades[&cid].state, TradeState::Open);
    assert!(m.book.trades[&cid].sl_algo.is_some());
    assert_eq!(m.audit.count("UNKNOWN_RESOLVED_BY_QUERY"), 1);
}

#[tokio::test]
async fn unknown_submit_dihapus_hanya_setelah_jadwal_habis_tanpa_jejak() {
    let ex = MockExchange::new(60.0);
    ex.set_price("ETHUSDT", dec!(2500), T0);
    ex.knobs(|k| {
        k.unknown_place = 1;
        k.unknown_drop_before_book = true;
    });
    let mut m = mgr("unknown-submit-drop", ex, OrphanPolicy::PlaceSl);
    let cid = m
        .submit(
            intent(Side::Long, dec!(2500), dec!(2440), EntryKind::Market),
            T0,
        )
        .await
        .unwrap();
    assert_eq!(m.book.trades[&cid].state, TradeState::UnknownSubmit);

    // Selama jadwal backoff [1s, 3s, 7s, 15s] trade TIDAK boleh dihapus dini
    for dt in [1_000_i64, 3_000, 7_000, 15_000] {
        m.tick_unknown(T0 + dt).await;
        assert!(
            m.book.trades.contains_key(&cid),
            "trade tidak boleh dihapus sebelum jadwal habis (dt={dt})"
        );
    }

    // Pada akhir jadwal (+30_000 ms), open_orders & positions kosong -> dihapus dengan ENTRY_NOT_FOUND_FINAL
    m.tick_unknown(T0 + 30_000).await;
    assert!(!m.book.trades.contains_key(&cid));
    assert!(!m.symbol_busy("ETHUSDT"));
    assert_eq!(m.audit.count("ENTRY_NOT_FOUND_FINAL"), 1);
}

#[tokio::test]
async fn unknown_submit_mengadopsi_posisi_yatim_pada_pengecekan_akhir_dan_reconcile() {
    let ex = MockExchange::new(60.0);
    ex.set_price("ETHUSDT", dec!(2500), T0);
    ex.knobs(|k| {
        k.unknown_place = 1;
        k.unknown_drop_before_book = true;
    });
    let mut m = mgr("unknown-submit-adopt", ex, OrphanPolicy::PlaceSl);
    let cid = m
        .submit(
            intent(Side::Long, dec!(2500), dec!(2440), EntryKind::Market),
            T0,
        )
        .await
        .unwrap();

    // Simulasikan posisi sudah muncul di bursa walau query_order by clientOrderId tidak menemukannya
    m.ex.inject_position("ETHUSDT", dec!(0.02), dec!(2500));
    for dt in [1_000_i64, 3_000, 7_000, 15_000, 30_000] {
        m.tick_unknown(T0 + dt).await;
    }
    assert_eq!(m.book.trades[&cid].state, TradeState::Open);
    assert!(m.book.trades[&cid].adopted);
    assert!(m.book.trades[&cid].sl_algo.is_some());
    assert_eq!(m.audit.count("UNKNOWN_ADOPTED_POSITION"), 1);

    // Reconcile saat boot juga memperlakukan UnknownSubmit yang sudah terisi di bursa secara aman
    let ex2 = MockExchange::new(60.0);
    ex2.set_price("ETHUSDT", dec!(2500), T0);
    ex2.knobs(|k| {
        k.unknown_place = 1;
        k.unknown_invisible_polls = 1;
    });
    let mut m2 = mgr("unknown-submit-reconcile", ex2, OrphanPolicy::PlaceSl);
    let cid2 = m2
        .submit(
            intent(Side::Long, dec!(2500), dec!(2440), EntryKind::Market),
            T0,
        )
        .await
        .unwrap();
    assert_eq!(m2.book.trades[&cid2].state, TradeState::UnknownSubmit);
    m2.reconcile(T0 + 500).await.unwrap();
    assert_eq!(m2.book.trades[&cid2].state, TradeState::Open);
    assert!(m2.book.trades[&cid2].sl_algo.is_some());
}

#[tokio::test]
async fn protect_retry_backoff_sukses_sl_missing_dan_flatten_kill_gr8() {
    // 1. place_algo gagal 2 kali lalu sukses pada percobaan ke-3; konfirmasi >2 dtk memicu SL_MISSING
    let ex = MockExchange::new(60.0);
    ex.set_price("ETHUSDT", dec!(2500), T0);
    ex.knobs(|k| {
        k.fail_algo = 2;
    });
    let rec = std::sync::Arc::new(apex_oms::notifier::RecordingNotifier::new());
    let mut m = mgr("protect-retry-ok", ex, OrphanPolicy::PlaceSl).with_notifier(rec.clone());
    let cid = m
        .submit(
            intent(Side::Long, dec!(2500), dec!(2440), EntryKind::Market),
            T0,
        )
        .await
        .unwrap();
    assert_eq!(m.book.trades[&cid].state, TradeState::Open);
    assert!(m.book.trades[&cid].sl_algo.is_some());
    assert!(!m.kill.is_engaged());

    // Simulasikan ulang pemasangan SL yang baru terkonfirmasi pada T0 + 2500 ms (>2000 ms GR-8)
    let old_sl = m.book.trades[&cid].sl_algo.unwrap();
    m.ex.cancel_algo("ETHUSDT", old_sl).await.unwrap();
    m.book.trades.get_mut(&cid).unwrap().sl_algo = None;
    m.protect(&cid, T0 + 2_500).await;
    assert!(m.book.trades[&cid].sl_algo.is_some());
    assert_eq!(m.audit.count("SL_MISSING"), 1);
    assert_eq!(rec.count_event("SL_MISSING"), 1);
    assert!(!m.kill.is_engaged());

    // 2. Gagal terus melewati batas retry -> flatten reduceOnly + engage_kill
    let ex_fail = MockExchange::new(60.0);
    ex_fail.set_price("ETHUSDT", dec!(2500), T0);
    ex_fail.knobs(|k| {
        k.fail_algo = 10;
    });
    let mut m_fail = mgr("protect-retry-fail", ex_fail, OrphanPolicy::PlaceSl);
    m_fail.cfg.protection_retries = 2;
    m_fail.cfg.protection_timeout_ms = 250;
    let cid_fail = m_fail
        .submit(
            intent(Side::Long, dec!(2500), dec!(2440), EntryKind::Market),
            T0,
        )
        .await
        .unwrap();
    assert!(
        !m_fail.book.trades.contains_key(&cid_fail),
        "posisi wajib sudah di-flatten"
    );
    assert_eq!(m_fail.book.closed.len(), 1);
    assert!(m_fail.kill.is_engaged());

    // 3. Jalur would_trigger tetap langsung flatten + kill tanpa menunggu retry sia-sia
    let ex_wt = MockExchange::new(60.0);
    ex_wt.set_price("ETHUSDT", dec!(2500), T0);
    ex_wt.knobs(|k| {
        k.market_ack_only = true;
    });
    let mut m_wt = mgr("protect-would-trigger", ex_wt, OrphanPolicy::PlaceSl);
    let cid_wt = m_wt
        .submit(
            intent(Side::Long, dec!(2500), dec!(2440), EntryKind::Market),
            T0,
        )
        .await
        .unwrap();
    m_wt.ex.knobs(|k| {
        k.market_ack_only = false;
    });
    m_wt.ex.set_price("ETHUSDT", dec!(2430), T0 + 10);
    pump(&mut m_wt, T0 + 10).await;
    assert!(!m_wt.book.trades.contains_key(&cid_wt));
    assert!(m_wt.kill.is_engaged());
    let algo_calls = m_wt
        .ex
        .calls()
        .iter()
        .filter(|c| c.starts_with("algo "))
        .count();
    assert_eq!(
        algo_calls, 1,
        "would_trigger harus langsung keluar tanpa retry sia-sia"
    );
}

#[tokio::test]
async fn notifier_merekam_event_penting_tanpa_http() {
    let ex = MockExchange::new(60.0);
    ex.set_price("ETHUSDT", dec!(2500), T0);
    ex.inject_position("ETHUSDT", dec!(0.02), dec!(2500));
    let rec = std::sync::Arc::new(apex_oms::notifier::RecordingNotifier::new());
    let mut m = mgr("notifier-events", ex, OrphanPolicy::PlaceSl).with_notifier(rec.clone());
    m.risk_cfg.daily_sl_limit = 1;

    m.boot(T0).await.unwrap();
    assert_eq!(rec.count_event("BOOT"), 1);
    assert_eq!(rec.count_event("RECONCILE_ORPHAN"), 1);

    // Tutup posisi orphan sebagai StopLoss -> memicu DAILY_STOP karena max_daily_sl = 1
    let orphan_cid = m.book.trades.keys().next().unwrap().clone();
    m.ex.set_price("ETHUSDT", dec!(2440), T0 + 1_000);
    m.close_trade(&orphan_cid, ExitKind::StopLoss, T0 + 1_000)
        .await
        .unwrap();
    assert_eq!(rec.count_event("DAILY_STOP"), 1);

    m.engage_kill("manual-test", false, T0 + 2_000).await;
    assert_eq!(rec.count_event("KILL_ENGAGED"), 1);

    m.shutdown(T0 + 3_000);
    assert_eq!(rec.count_event("SHUTDOWN"), 1);
}
