//! Uji mock menyeluruh untuk pemeriksaan pra-terbang C1..C15 (WP-9a).
//!
//! Seluruh pengujian memakai implementasi `MockPreflightBackend` in-memory.
//! Tidak ada panggilan jaringan ke Binance (H1/H2).
#![allow(clippy::type_complexity, clippy::field_reassign_with_default)]

use apex_core::config::{EngineConfig, Mode, RiskConfig};
use apex_core::filters::SymbolFilters;
use apex_oms::preflight::{
    format_preflight_table, run_preflight, CheckVerdict, EnvPresence, PreflightCheck,
    PreflightInput, PreflightTarget, MIN_FREE_DISK_BYTES,
};
use apex_oms::preflight_http::{
    AccountModeInfo, ApiRestrictionsInfo, CommissionRateInfo, PreflightBackend, SymbolAccountConfig,
};
use apex_oms::types::ExError;
use rust_decimal::Decimal;
use rust_decimal_macros::dec;
use std::collections::BTreeMap;
use std::path::Path;
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::sync::Arc;

#[derive(Clone)]
struct MockPreflightBackend {
    clock_offset: Result<i64, String>,
    ws_handshake: Result<String, String>,
    restrictions: Result<ApiRestrictionsInfo, ExError>,
    account_mode: Result<AccountModeInfo, String>,
    symbol_configs: Result<Vec<SymbolAccountConfig>, String>,
    balance: Result<f64, String>,
    filters_marks: Result<(BTreeMap<String, SymbolFilters>, BTreeMap<String, Decimal>), String>,
    commission_rate: Result<CommissionRateInfo, String>,
    listen_key: Result<(), String>,
    disk_free: Result<u64, String>,
    notify_status: Result<u16, String>,
    listen_key_probed: Arc<AtomicBool>,
    notify_calls: Arc<AtomicU32>,
}

impl Default for MockPreflightBackend {
    fn default() -> Self {
        let mut filters = BTreeMap::new();
        filters.insert(
            "ETHUSDT".into(),
            SymbolFilters::synthetic("ETHUSDT", dec!(0.01), dec!(0.001), dec!(5)),
        );
        let mut marks = BTreeMap::new();
        marks.insert("ETHUSDT".into(), dec!(2500));

        Self {
            clock_offset: Ok(45),
            ws_handshake: Ok("wss://demo-fstream.binance.com/public/stream?streams=...".into()),
            restrictions: Ok(ApiRestrictionsInfo {
                enable_withdrawals: false,
                enable_futures: true,
                ip_restrict: true,
            }),
            account_mode: Ok(AccountModeInfo {
                dual_side_position: false,
                multi_assets_margin: false,
            }),
            symbol_configs: Ok(vec![SymbolAccountConfig {
                symbol: "ETHUSDT".into(),
                margin_type: "CROSSED".into(),
                leverage: 25,
            }]),
            balance: Ok(120.0),
            filters_marks: Ok((filters, marks)),
            commission_rate: Ok(CommissionRateInfo {
                symbol: "ETHUSDT".into(),
                maker: dec!(0.0002),
                taker: dec!(0.0004),
            }),
            listen_key: Ok(()),
            disk_free: Ok(500 * 1024 * 1024),
            notify_status: Ok(200),
            listen_key_probed: Arc::new(AtomicBool::new(false)),
            notify_calls: Arc::new(AtomicU32::new(0)),
        }
    }
}

impl PreflightBackend for MockPreflightBackend {
    async fn clock_offset_ms(&self) -> Result<i64, String> {
        self.clock_offset.clone()
    }
    async fn ws_public_handshake(&self, _ws_root: &str) -> Result<String, String> {
        self.ws_handshake.clone()
    }
    async fn api_restrictions(&self) -> Result<ApiRestrictionsInfo, ExError> {
        self.restrictions.clone()
    }
    async fn account_mode(&self) -> Result<AccountModeInfo, String> {
        self.account_mode.clone()
    }
    async fn symbol_configs(&self) -> Result<Vec<SymbolAccountConfig>, String> {
        self.symbol_configs.clone()
    }
    async fn usdt_available_balance(&self) -> Result<f64, String> {
        self.balance.clone()
    }
    async fn exchange_filters_and_marks(
        &self,
    ) -> Result<(BTreeMap<String, SymbolFilters>, BTreeMap<String, Decimal>), String> {
        self.filters_marks.clone()
    }
    async fn commission_rate(&self, _symbol: &str) -> Result<CommissionRateInfo, String> {
        self.commission_rate.clone()
    }
    async fn probe_listen_key(&self) -> Result<(), String> {
        self.listen_key_probed.store(true, Ordering::SeqCst);
        self.listen_key.clone()
    }
    async fn probe_state_fsync_and_disk(&self, _state_dir: &Path) -> Result<u64, String> {
        self.disk_free.clone()
    }
    async fn send_notify_test(&self, _ts: i64) -> Result<u16, String> {
        self.notify_calls.fetch_add(1, Ordering::SeqCst);
        self.notify_status.clone()
    }
}

fn sample_input(target: PreflightTarget) -> PreflightInput {
    static SEQ: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
    let seq = SEQ.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
    let temp_dir = std::env::temp_dir().join(format!("apex_pfl_test_{}_{seq}", std::process::id()));
    let (mode, arm) = match target {
        PreflightTarget::Demo => (Mode::Testnet, true),
        PreflightTarget::Live => (Mode::Live, true),
    };
    let mut engine = EngineConfig {
        mode,
        arm,
        state_dir: temp_dir.clone(),
        kill_file: temp_dir.join("KILL"),
        testnet_rest_base: "https://demo-fapi.binance.com".into(),
        testnet_ws_base: "wss://demo-fstream.binance.com".into(),
        live_rest_base: "https://fapi.binance.com".into(),
        live_ws_base: "wss://fstream.binance.com".into(),
        max_clock_offset_ms: 500,
        ..Default::default()
    };
    engine.universe = vec!["ETHUSDT".into()];
    PreflightInput {
        target,
        notify_test: false,
        engine,
        risk: RiskConfig::default(),
        universe: vec!["ETHUSDT".into()],
        enabled_strategies: vec![("S1".into(), "GO".into())],
        env: EnvPresence {
            api_key_set: true,
            api_secret_set: true,
            live_confirm_set: target == PreflightTarget::Live,
            live_confirm_matches: target == PreflightTarget::Live,
            notify_url_set: true,
        },
        config_hash: "fedcba9876543210".into(),
        binary_version: "0.1.0".into(),
        git_commit: "c7de02a11223".into(),
        allow_local_mock_hosts: false,
    }
}

fn find_chk<'a>(checks: &'a [PreflightCheck], id: &str) -> &'a PreflightCheck {
    checks.iter().find(|c| c.id == id).expect("check not found")
}

#[tokio::test]
async fn c01_env_presence_pass_and_fail() {
    let mut inp = sample_input(PreflightTarget::Live);
    let be = MockPreflightBackend::default();
    let rep = run_preflight(&inp, &be).await;
    assert_eq!(find_chk(&rep.checks, "C1").verdict, CheckVerdict::Pass);
    assert!(!rep.has_failure());

    inp.env.live_confirm_matches = false;
    let rep_fail = run_preflight(&inp, &be).await;
    let c1 = find_chk(&rep_fail.checks, "C1");
    assert_eq!(c1.verdict, CheckVerdict::Fail);
    assert!(c1.detail.contains("APEX_LIVE_CONFIRM"));
    assert!(!c1.detail.contains("I_UNDERSTAND_LIVE_ORDERS"));
}

#[tokio::test]
async fn c02_mode_arm_consistency_pass_and_fail() {
    let mut inp = sample_input(PreflightTarget::Demo);
    let be = MockPreflightBackend::default();
    let rep = run_preflight(&inp, &be).await;
    assert_eq!(find_chk(&rep.checks, "C2").verdict, CheckVerdict::Pass);

    inp.engine.arm = false;
    let rep2 = run_preflight(&inp, &be).await;
    assert_eq!(find_chk(&rep2.checks, "C2").verdict, CheckVerdict::Fail);
}

#[tokio::test]
async fn c03_clock_offset_pass_and_fail() {
    let inp = sample_input(PreflightTarget::Demo);
    let mut be = MockPreflightBackend::default();
    be.clock_offset = Ok(150);
    let rep_pass = run_preflight(&inp, &be).await;
    assert_eq!(find_chk(&rep_pass.checks, "C3").verdict, CheckVerdict::Pass);

    be.clock_offset = Ok(650);
    let rep_fail = run_preflight(&inp, &be).await;
    let c3 = find_chk(&rep_fail.checks, "C3");
    assert_eq!(c3.verdict, CheckVerdict::Fail);
    assert!(c3.detail.contains("650 ms melebihi max_clock_offset_ms"));
}

#[tokio::test]
async fn c04_hosts_and_public_ws_pass_and_fail() {
    let mut inp = sample_input(PreflightTarget::Demo);
    let be = MockPreflightBackend::default();
    let rep = run_preflight(&inp, &be).await;
    assert_eq!(find_chk(&rep.checks, "C4").verdict, CheckVerdict::Pass);

    inp.engine.testnet_rest_base = "https://fapi.binance.com".into();
    let rep_fail = run_preflight(&inp, &be).await;
    assert_eq!(find_chk(&rep_fail.checks, "C4").verdict, CheckVerdict::Fail);
}

#[tokio::test]
async fn c05_strategy_go_pass_demo_warn_and_live_fail() {
    let mut inp = sample_input(PreflightTarget::Demo);
    inp.enabled_strategies = vec![("S1".into(), "NO-GO".into())];
    let be = MockPreflightBackend::default();
    let rep_demo = run_preflight(&inp, &be).await;
    assert_eq!(find_chk(&rep_demo.checks, "C5").verdict, CheckVerdict::Warn);
    assert!(!rep_demo.has_failure());

    inp.target = PreflightTarget::Live;
    inp.engine.mode = Mode::Live;
    inp.env.live_confirm_matches = true;
    let rep_live = run_preflight(&inp, &be).await;
    assert_eq!(find_chk(&rep_live.checks, "C5").verdict, CheckVerdict::Fail);
    assert!(rep_live.has_failure());
}

#[tokio::test]
async fn c06_api_restrictions_withdrawals_fail_and_ip_restrict_policy() {
    let inp_live = sample_input(PreflightTarget::Live);
    let mut be = MockPreflightBackend::default();
    be.restrictions = Ok(ApiRestrictionsInfo {
        enable_withdrawals: true,
        enable_futures: true,
        ip_restrict: true,
    });
    let rep1 = run_preflight(&inp_live, &be).await;
    assert_eq!(find_chk(&rep1.checks, "C6").verdict, CheckVerdict::Fail);

    be.restrictions = Ok(ApiRestrictionsInfo {
        enable_withdrawals: false,
        enable_futures: true,
        ip_restrict: false,
    });
    let rep_live_no_ip = run_preflight(&inp_live, &be).await;
    assert_eq!(
        find_chk(&rep_live_no_ip.checks, "C6").verdict,
        CheckVerdict::Fail
    );

    let inp_demo = sample_input(PreflightTarget::Demo);
    let rep_demo_no_ip = run_preflight(&inp_demo, &be).await;
    assert_eq!(
        find_chk(&rep_demo_no_ip.checks, "C6").verdict,
        CheckVerdict::Warn
    );
}

#[tokio::test]
async fn c07_account_mode_one_way_and_single_asset() {
    let inp = sample_input(PreflightTarget::Demo);
    let mut be = MockPreflightBackend::default();
    let rep = run_preflight(&inp, &be).await;
    assert_eq!(find_chk(&rep.checks, "C7").verdict, CheckVerdict::Pass);

    be.account_mode = Ok(AccountModeInfo {
        dual_side_position: true,
        multi_assets_margin: false,
    });
    let rep_fail = run_preflight(&inp, &be).await;
    assert_eq!(find_chk(&rep_fail.checks, "C7").verdict, CheckVerdict::Fail);
}

#[tokio::test]
async fn c08_symbol_configs_margin_and_leverage() {
    let inp = sample_input(PreflightTarget::Demo);
    let mut be = MockPreflightBackend::default();
    let rep = run_preflight(&inp, &be).await;
    assert_eq!(find_chk(&rep.checks, "C8").verdict, CheckVerdict::Pass);

    be.symbol_configs = Ok(vec![SymbolAccountConfig {
        symbol: "ETHUSDT".into(),
        margin_type: "ISOLATED".into(),
        leverage: 25,
    }]);
    let rep_fail = run_preflight(&inp, &be).await;
    assert_eq!(find_chk(&rep_fail.checks, "C8").verdict, CheckVerdict::Fail);
}

#[tokio::test]
async fn c09_balance_above_floor_and_insufficient_fail() {
    let inp = sample_input(PreflightTarget::Demo);
    let mut be = MockPreflightBackend::default();
    be.balance = Ok(80.0);
    let rep = run_preflight(&inp, &be).await;
    assert_eq!(find_chk(&rep.checks, "C9").verdict, CheckVerdict::Pass);

    be.balance = Ok(35.0);
    let rep_fail = run_preflight(&inp, &be).await;
    assert_eq!(find_chk(&rep_fail.checks, "C9").verdict, CheckVerdict::Fail);
}

#[tokio::test]
async fn c10_exchange_info_sizing_pass_and_min_notional_fail() {
    let inp = sample_input(PreflightTarget::Demo);
    let mut be = MockPreflightBackend::default();
    let rep = run_preflight(&inp, &be).await;
    assert_eq!(find_chk(&rep.checks, "C10").verdict, CheckVerdict::Pass);

    let mut bad_filters = BTreeMap::new();
    bad_filters.insert(
        "ETHUSDT".into(),
        SymbolFilters::synthetic("ETHUSDT", dec!(0.01), dec!(0.001), dec!(100)),
    );
    be.filters_marks = Ok((
        bad_filters,
        BTreeMap::from([("ETHUSDT".into(), dec!(2500))]),
    ));
    let rep_fail = run_preflight(&inp, &be).await;
    assert_eq!(
        find_chk(&rep_fail.checks, "C10").verdict,
        CheckVerdict::Fail
    );
}

#[tokio::test]
async fn c11_commission_rates_pass_and_exceeding_model_fail() {
    let inp = sample_input(PreflightTarget::Demo);
    let mut be = MockPreflightBackend::default();
    let rep = run_preflight(&inp, &be).await;
    assert_eq!(find_chk(&rep.checks, "C11").verdict, CheckVerdict::Pass);

    be.commission_rate = Ok(CommissionRateInfo {
        symbol: "ETHUSDT".into(),
        maker: dec!(0.0003),
        taker: dec!(0.0007),
    });
    let rep_fail = run_preflight(&inp, &be).await;
    assert_eq!(
        find_chk(&rep_fail.checks, "C11").verdict,
        CheckVerdict::Fail
    );
}

#[tokio::test]
async fn c12_listen_key_probe_invoked_and_handles_error() {
    let inp = sample_input(PreflightTarget::Demo);
    let be = MockPreflightBackend::default();
    let rep = run_preflight(&inp, &be).await;
    assert!(be.listen_key_probed.load(Ordering::SeqCst));
    assert_eq!(find_chk(&rep.checks, "C12").verdict, CheckVerdict::Pass);

    let mut be_err = MockPreflightBackend::default();
    be_err.listen_key = Err("binance -1125 listenKey unauthorized".into());
    let rep_fail = run_preflight(&inp, &be_err).await;
    assert_eq!(
        find_chk(&rep_fail.checks, "C12").verdict,
        CheckVerdict::Fail
    );
}

#[tokio::test]
async fn c13_state_fsync_and_disk_space_thresholds() {
    let inp = sample_input(PreflightTarget::Demo);
    let mut be = MockPreflightBackend::default();
    be.disk_free = Ok(MIN_FREE_DISK_BYTES + 1024);
    let rep = run_preflight(&inp, &be).await;
    assert_eq!(find_chk(&rep.checks, "C13").verdict, CheckVerdict::Pass);

    be.disk_free = Ok(MIN_FREE_DISK_BYTES - 1024);
    let rep_fail = run_preflight(&inp, &be).await;
    assert_eq!(
        find_chk(&rep_fail.checks, "C13").verdict,
        CheckVerdict::Fail
    );
}

#[tokio::test]
async fn c14_kill_switch_active_flags_failure() {
    let inp = sample_input(PreflightTarget::Demo);
    let ks = apex_oms::killswitch::KillSwitch::new(&inp.engine.kill_file);
    let _ = ks.release();
    let be = MockPreflightBackend::default();
    let rep = run_preflight(&inp, &be).await;
    assert_eq!(find_chk(&rep.checks, "C14").verdict, CheckVerdict::Pass);

    ks.engage("uji operator kill").unwrap();
    let rep_fail = run_preflight(&inp, &be).await;
    let c14 = find_chk(&rep_fail.checks, "C14");
    assert_eq!(c14.verdict, CheckVerdict::Fail);
    assert!(c14.detail.contains("uji operator kill"));
    let _ = ks.release();
}

#[tokio::test]
async fn c15_notifier_skipped_without_flag_and_invoked_with_flag() {
    let mut inp = sample_input(PreflightTarget::Demo);
    let be = MockPreflightBackend::default();
    inp.notify_test = false;
    let rep_no_flag = run_preflight(&inp, &be).await;
    assert_eq!(
        find_chk(&rep_no_flag.checks, "C15").verdict,
        CheckVerdict::Pass
    );
    assert_eq!(be.notify_calls.load(Ordering::SeqCst), 0);

    inp.notify_test = true;
    let rep_with_flag = run_preflight(&inp, &be).await;
    assert_eq!(
        find_chk(&rep_with_flag.checks, "C15").verdict,
        CheckVerdict::Pass
    );
    assert_eq!(be.notify_calls.load(Ordering::SeqCst), 1);

    let table = format_preflight_table(&rep_with_flag);
    assert!(table.contains("OVERALL: PASS"));
    assert!(table.contains("C15"));
}
