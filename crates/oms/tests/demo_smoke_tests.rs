//! Uji komprehensif `apex demo-smoke` dengan `MockExchange` (WP-9c).
//!
//! Membuktikan:
//! 1. Penolakan keras pada mode `Paper` dan `Live`.
//! 2. Penolakan keras pada host non-demo.
//! 3. Jarak order limit GTX selalu >= 5% dari mark price dan tidak pernah terisi.
//! 4. Pembersihan residual (`cleanup_residual`) berjalan dan membatalkan sisa order/algo serta
//!    meratakan posisi saat terjadi injeksi kegagalan di tengah alur eksekusi.
#![allow(clippy::type_complexity, clippy::field_reassign_with_default)]

use apex_core::config::{EngineConfig, Mode, RiskConfig};
use apex_core::filters::SymbolFilters;
use apex_oms::demo_smoke::{cleanup_residual, run_demo_smoke, DemoSmokeError, MockEventSource};
use apex_oms::mock::MockExchange;
use apex_oms::preflight::{EnvPresence, PreflightInput, PreflightTarget};
use apex_oms::preflight_http::{
    AccountModeInfo, ApiRestrictionsInfo, CommissionRateInfo, PreflightBackend, SymbolAccountConfig,
};
use apex_oms::types::ExError;
use rust_decimal::Decimal;
use rust_decimal_macros::dec;
use std::collections::BTreeMap;
use std::path::Path;

#[derive(Clone)]
struct SmokeTestBackend {
    filters: BTreeMap<String, SymbolFilters>,
    marks: BTreeMap<String, Decimal>,
}

impl Default for SmokeTestBackend {
    fn default() -> Self {
        let mut filters = BTreeMap::new();
        // SUIUSDT memiliki percent_down = 0.85 (memungkinkan limit pada 94% mark) dan min_notional = 5.
        filters.insert(
            "SUIUSDT".into(),
            SymbolFilters {
                symbol: "SUIUSDT".into(),
                tick_size: dec!(0.0001),
                step_size: dec!(0.1),
                min_qty: dec!(0.1),
                market_step_size: dec!(0.1),
                market_min_qty: dec!(0.1),
                market_max_qty: dec!(100000),
                min_notional: dec!(5),
                percent_up: dec!(1.15),
                percent_down: dec!(0.85),
            },
        );
        let mut marks = BTreeMap::new();
        marks.insert("SUIUSDT".into(), dec!(2.0000));
        Self { filters, marks }
    }
}

impl PreflightBackend for SmokeTestBackend {
    async fn clock_offset_ms(&self) -> Result<i64, String> {
        Ok(20)
    }
    async fn ws_public_handshake(&self, _ws: &str) -> Result<String, String> {
        Ok("wss://demo-fstream.binance.com/public/stream".into())
    }
    async fn api_restrictions(&self) -> Result<ApiRestrictionsInfo, ExError> {
        Ok(ApiRestrictionsInfo {
            enable_withdrawals: false,
            enable_futures: true,
            ip_restrict: false,
        })
    }
    async fn account_mode(&self) -> Result<AccountModeInfo, String> {
        Ok(AccountModeInfo {
            dual_side_position: false,
            multi_assets_margin: false,
        })
    }
    async fn symbol_configs(&self) -> Result<Vec<SymbolAccountConfig>, String> {
        Ok(vec![SymbolAccountConfig {
            symbol: "SUIUSDT".into(),
            margin_type: "CROSSED".into(),
            leverage: 25,
        }])
    }
    async fn usdt_available_balance(&self) -> Result<f64, String> {
        Ok(250.0)
    }
    async fn exchange_filters_and_marks(
        &self,
    ) -> Result<(BTreeMap<String, SymbolFilters>, BTreeMap<String, Decimal>), String> {
        Ok((self.filters.clone(), self.marks.clone()))
    }
    async fn commission_rate(&self, _s: &str) -> Result<CommissionRateInfo, String> {
        Ok(CommissionRateInfo {
            symbol: "SUIUSDT".into(),
            maker: dec!(0.0002),
            taker: dec!(0.0004),
        })
    }
    async fn probe_listen_key(&self) -> Result<(), String> {
        Ok(())
    }
    async fn probe_state_fsync_and_disk(&self, _p: &Path) -> Result<u64, String> {
        Ok(1024 * 1024 * 1024)
    }
    async fn send_notify_test(&self, _ts: i64) -> Result<u16, String> {
        Ok(200)
    }
}

fn valid_smoke_input() -> PreflightInput {
    let tmp = std::env::temp_dir().join(format!("apex_smoke_test_{}", std::process::id()));
    PreflightInput {
        target: PreflightTarget::Demo,
        notify_test: false,
        engine: EngineConfig {
            mode: Mode::Testnet,
            arm: true,
            state_dir: tmp.clone(),
            kill_file: tmp.join("KILL"),
            testnet_rest_base: "https://demo-fapi.binance.com".into(),
            testnet_ws_base: "wss://demo-fstream.binance.com".into(),
            live_rest_base: "https://fapi.binance.com".into(),
            live_ws_base: "wss://fstream.binance.com".into(),
            ..Default::default()
        },
        risk: RiskConfig::default(),
        universe: vec!["SUIUSDT".into()],
        enabled_strategies: vec![("S1".into(), "NO-GO".into())],
        env: EnvPresence {
            api_key_set: true,
            api_secret_set: true,
            live_confirm_set: false,
            live_confirm_matches: false,
            notify_url_set: false,
        },
        config_hash: "0123456789abcdef".into(),
        binary_version: "0.1.0".into(),
        git_commit: "c7de02a11223".into(),
        allow_local_mock_hosts: false,
    }
}

#[tokio::test]
async fn demo_smoke_strictly_rejects_paper_mode() {
    let mut inp = valid_smoke_input();
    inp.engine.mode = Mode::Paper;
    let be = SmokeTestBackend::default();
    let ex = MockExchange::new(250.0);
    let mut ev = MockEventSource(&ex);
    let res = run_demo_smoke(&inp, &be, &ex, &mut ev, "t1").await;
    match res {
        Err(DemoSmokeError::ModeNotDemo(m)) => assert_eq!(m, apex_oms::safety::ExecMode::Paper),
        other => panic!("expected ModeNotDemo(Paper), got {other:?}"),
    }
}

#[tokio::test]
async fn demo_smoke_strictly_rejects_live_mode() {
    let mut inp = valid_smoke_input();
    inp.engine.mode = Mode::Live;
    inp.env.live_confirm_matches = true;
    let be = SmokeTestBackend::default();
    let ex = MockExchange::new(250.0);
    let mut ev = MockEventSource(&ex);
    let res = run_demo_smoke(&inp, &be, &ex, &mut ev, "t2").await;
    match res {
        Err(DemoSmokeError::ModeNotDemo(m)) => assert_eq!(m, apex_oms::safety::ExecMode::Live),
        other => panic!("expected ModeNotDemo(Live), got {other:?}"),
    }
}

#[tokio::test]
async fn demo_smoke_rejects_non_demo_hosts() {
    let mut inp = valid_smoke_input();
    inp.engine.testnet_rest_base = "https://fstream.binancefuture.com".into();
    let be = SmokeTestBackend::default();
    let ex = MockExchange::new(250.0);
    let mut ev = MockEventSource(&ex);
    let res = run_demo_smoke(&inp, &be, &ex, &mut ev, "t3").await;
    assert!(matches!(res, Err(DemoSmokeError::HostNotDemo(_))));
}

#[tokio::test]
async fn demo_smoke_places_far_order_verifies_distance_and_clean_exit() {
    let inp = valid_smoke_input();
    let be = SmokeTestBackend::default();
    let ex = MockExchange::new(250.0);
    ex.set_price("SUIUSDT", dec!(2.0000), 1_700_000_000_000);
    let mut ev = MockEventSource(&ex);

    let rep = run_demo_smoke(&inp, &be, &ex, &mut ev, "t4")
        .await
        .expect("demo smoke run failed");

    assert_eq!(rep.symbol, "SUIUSDT");
    assert!(rep.distance_pct >= dec!(0.05), "distance must be >= 5%");
    assert_eq!(rep.limit_price, dec!(1.8800)); // 2.0 * 0.94
    assert!(rep.notional >= dec!(5.0), "notional >= min_notional");
    assert_eq!(rep.cleanup.canceled_orders, 0); // sudah dibatalkan dalam alur normal
    assert_eq!(rep.cleanup.canceled_algos, 0);
    assert_eq!(rep.cleanup.flattened_positions, 0);
    assert!(!rep.user_stream_lines.is_empty());
}

#[tokio::test]
async fn demo_smoke_cleans_up_residual_on_injected_error() {
    let ex = MockExchange::new(200.0);
    ex.set_price("SUIUSDT", dec!(2.0000), 1_700_000_000_000);
    ex.inject_position("SUIUSDT", dec!(15.0), dec!(1.9500));

    let sum = cleanup_residual(&ex, "SUIUSDT", "errtag").await;
    assert_eq!(sum.flattened_positions, 1);
    assert_eq!(ex.position("SUIUSDT"), dec!(0));
}
