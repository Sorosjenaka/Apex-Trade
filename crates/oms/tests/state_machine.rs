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
    m.insert("ETHUSDT".into(), SymbolFilters::synthetic("ETHUSDT", dec!(0.01), dec!(0.001), dec!(5)));
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
    let cfg = EngineConfig { kill_file: d.join("KILL"), state_dir: d.clone(), orphan_policy: policy, ..Default::default() };
    let mut m = OrderManager::new(ex, ExecMode::Paper, cfg, RiskConfig::default(), StateStore::file(&d.join("oms_state.json")), AuditLog::in_memory(), filters()).unwrap();
    m.max_backoff_ms = 20;
    m
}

fn intent(side: Side, entry: Decimal, sl: Decimal, kind: EntryKind) -> EntryIntent {
    let f = filters()["ETHUSDT"].clone();
    let rc = RiskConfig::default();
    let sizing = size_order(&SizingInput { side, entry, sl, kind, filters: &f, risk: &rc }).unwrap();
    EntryIntent { symbol: "ETHUSDT".into(), strategy: "S1".into(), sizing, kind, ttl_ms: 3_600_000 }
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
    let cid = m.submit(intent(Side::Long, dec!(2500), dec!(2440), EntryKind::Market), T0).await.unwrap();
    pump(&mut m, T0).await;
    let t = m.book.trades[&cid].clone();
    assert_eq!(t.state, TradeState::Open);
    assert!(t.sl_algo.is_some() && t.tp_algo.is_some());
    assert!(m.ex.is_one_way());
    assert_eq!(m.ex.leverage_of("ETHUSDT"), Some(25));
    assert_eq!(m.ex.margin_of("ETHUSDT").as_deref(), Some("CROSSED"));
    let algos = m.ex.algos();
    assert_eq!(algos.len(), 2);
    assert!(algos.iter().all(|a| a.close_position && a.qty.is_none() && a.side == "SELL" && a.client_algo_id.len() <= 36));
    assert!(algos.iter().any(|a| a.kind == AlgoKind::StopMarket && a.trigger == t.sl));
}

#[tokio::test]
async fn limit_gtx_ack_waits_for_fill_event() {
    let ex = MockExchange::new(60.0);
    ex.set_price("ETHUSDT", dec!(2510), T0);
    let mut m = mgr("lim", ex, OrphanPolicy::PlaceSl);
    m.boot(T0).await.unwrap();
    let cid = m.submit(intent(Side::Long, dec!(2500), dec!(2440), EntryKind::LimitMaker), T0).await.unwrap();
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
    let r = m.submit(intent(Side::Long, dec!(2500), dec!(2440), EntryKind::LimitMaker), T0).await;
    assert!(r.is_err());
    assert_eq!(m.active().count(), 0);
}

#[tokio::test]
async fn market_ack_only_then_event() {
    let ex = MockExchange::new(60.0);
    ex.set_price("ETHUSDT", dec!(2500), T0);
    ex.knobs(|k| k.market_ack_only = true);
    let mut m = mgr("ack", ex, OrphanPolicy::PlaceSl);
    let cid = m.submit(intent(Side::Short, dec!(2500), dec!(2560), EntryKind::Market), T0).await.unwrap();
    assert_eq!(m.book.trades[&cid].state, TradeState::PendingEntry, "ACK NEW ≠ FILLED");
    pump(&mut m, T0).await;
    assert_eq!(m.book.trades[&cid].state, TradeState::Open);
    assert!(m.ex.algos().iter().all(|a| a.side == "BUY"));
}

#[tokio::test]
async fn sl_trigger_closes_and_starts_cooldown() {
    let ex = MockExchange::new(60.0);
    ex.set_price("ETHUSDT", dec!(2500), T0);
    let mut m = mgr("sl", ex, OrphanPolicy::PlaceSl);
    let cid = m.submit(intent(Side::Long, dec!(2500), dec!(2440), EntryKind::Market), T0).await.unwrap();
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
    m.submit(intent(Side::Long, dec!(2500), dec!(2440), EntryKind::Market), T0).await.unwrap();
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
    m.submit(intent(Side::Long, dec!(2500), dec!(2440), EntryKind::Market), T0).await.unwrap();
    pump(&mut m, T0).await;
    assert!(m.ex.position("ETHUSDT").is_zero(), "posisi tanpa SL harus di-flatten");
    assert_eq!(m.book.closed.last().unwrap().exit_kind, Some(ExitKind::StopLoss));
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
    let cid = m.submit(intent(Side::Long, dec!(2500), dec!(2440), EntryKind::Market), T0).await.unwrap();
    let t = &m.book.trades[&cid];
    assert_eq!(t.state, TradeState::Unprotected);
    assert!(m.kill.is_engaged());
    assert!(!m.ex.position("ETHUSDT").is_zero());
    // entry baru diblokir kill switch
    assert!(m.submit(intent(Side::Long, dec!(2500), dec!(2440), EntryKind::Market), T0).await.is_err());
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
    let cid = m.submit(intent(Side::Long, dec!(2500), dec!(2440), EntryKind::Market), T0).await.unwrap();
    pump(&mut m, T0).await;
    m.ex.knobs(|k| k.fail_reduce_only = 1);
    assert!(m.close_trade(&cid, ExitKind::Manual, T0 + 1000).await.is_err());
    assert!(m.book.trades.contains_key(&cid));
    assert_eq!(m.book.trades[&cid].state, TradeState::Open);
    assert!(m.book.trades[&cid].sl_algo.is_some(), "SL tetap ada");
    m.close_trade(&cid, ExitKind::Manual, T0 + 2000).await.unwrap();
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
    let sl = m.ex.algos().into_iter().find(|a| a.kind == AlgoKind::StopMarket).unwrap();
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
    let cid = m.submit(intent(Side::Long, dec!(2500), dec!(2440), EntryKind::Market), T0).await.unwrap();
    assert_eq!(m.book.trades[&cid].state, TradeState::Open);
    assert!(m.book.trades[&cid].sl_algo.is_some());
}

#[tokio::test]
async fn rate_limit_backoff_then_success() {
    let ex = MockExchange::new(60.0);
    ex.set_price("ETHUSDT", dec!(2500), T0);
    ex.knobs(|k| k.rate_limit = 2);
    let mut m = mgr("rl", ex, OrphanPolicy::PlaceSl);
    let cid = m.submit(intent(Side::Long, dec!(2500), dec!(2440), EntryKind::Market), T0).await.unwrap();
    assert_eq!(m.book.trades[&cid].state, TradeState::Open);
    assert!(m.audit.count("RATE_LIMIT_BACKOFF") >= 2);
}

#[tokio::test]
async fn pending_ttl_expires_and_kill_blocks() {
    let ex = MockExchange::new(60.0);
    ex.set_price("ETHUSDT", dec!(2510), T0);
    let mut m = mgr("ttl", ex, OrphanPolicy::PlaceSl);
    let cid = m.submit(intent(Side::Long, dec!(2500), dec!(2440), EntryKind::LimitMaker), T0).await.unwrap();
    m.tick(T0 + 3_600_001).await;
    assert!(!m.book.trades.contains_key(&cid));
    m.engage_kill("test", false, T0 + 3_700_000).await;
    assert!(matches!(m.submit(intent(Side::Long, dec!(2500), dec!(2440), EntryKind::LimitMaker), T0).await, Err(apex_oms::manager::OmsError::Killed)));
    m.kill.release().unwrap();
    assert!(m.submit(intent(Side::Long, dec!(2500), dec!(2440), EntryKind::LimitMaker), T0).await.is_ok());
}

#[tokio::test]
async fn state_persists_across_restart() {
    let d = tmpdir("persist");
    let cfg = EngineConfig { kill_file: d.join("KILL"), ..Default::default() };
    let store = StateStore::file(&d.join("oms_state.json"));
    let ex = MockExchange::new(60.0);
    ex.set_price("ETHUSDT", dec!(2500), T0);
    let mut m = OrderManager::new(ex, ExecMode::Paper, cfg.clone(), RiskConfig::default(), store.clone(), AuditLog::in_memory(), filters()).unwrap();
    let cid = m.submit(intent(Side::Long, dec!(2500), dec!(2440), EntryKind::Market), T0).await.unwrap();
    let ex2 = m.ex;
    let mut m2 = OrderManager::new(ex2, ExecMode::Paper, cfg, RiskConfig::default(), store, AuditLog::in_memory(), filters()).unwrap();
    assert_eq!(m2.book.trades[&cid].state, TradeState::Open);
    m2.reconcile(T0 + 1).await.unwrap();
    assert_eq!(m2.active().count(), 1, "tidak ada duplikasi saat reconcile");
}

#[test]
fn non_paper_requires_arm() {
    let cfg = EngineConfig { mode: apex_core::Mode::Testnet, arm: false, ..Default::default() };
    let r = OrderManager::new(MockExchange::new(60.0), ExecMode::Testnet, cfg, RiskConfig::default(), StateStore::memory(), AuditLog::in_memory(), filters());
    assert!(r.is_err());
}
