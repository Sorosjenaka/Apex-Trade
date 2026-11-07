//! Penangan CLI `apex preflight` dan `apex demo-smoke` (WP-9a & WP-9c).
//!
//! Catatan keamanan (H1/H2/H3):
//! - `preflight` bersifat read-only dan tidak pernah mengirim order.
//! - `demo-smoke` wajib lolos `apex_oms::safety::authorize` serta `check_base` tanpa modifikasi
//!   gerbang keselamatan bawaan, hanya menerima `Mode::Testnet` (`arm = true`) dengan host Demo
//!   resmi Binance (`demo-fapi.binance.com` / `demo-fstream.binance.com`).
#![cfg_attr(
    not(test),
    deny(clippy::unwrap_used, clippy::expect_used, clippy::panic)
)]

use crate::cfg::AllConfig;
use apex_core::time::now_ms;
use apex_oms::binance::BinanceRest;
use apex_oms::creds::Credentials;
use apex_oms::safety::{authorize, check_base, ExecMode, LIVE_CONFIRM_ENV};
use apex_oms::userstream::run_user_stream_with_heartbeat;
use apex_oms::{
    compute_config_hash, detect_git_commit_short, format_preflight_table, run_demo_smoke,
    run_preflight, BinancePreflightBackend, ChannelEventSource, EnvPresence, HttpWebhookNotifier,
    PreflightInput, PreflightTarget, DEFAULT_SPOT_SAPI_BASE,
};
use std::sync::atomic::{AtomicI64, Ordering};
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::mpsc;

fn build_preflight_input(
    cfg: &AllConfig,
    target: PreflightTarget,
    notify_test: bool,
) -> PreflightInput {
    let config_hash = compute_config_hash(&serde_json::json!({
        "risk": &cfg.risk,
        "screener": &cfg.screener,
        "strategies": &cfg.strategies,
    }));
    let enabled_strategies: Vec<(String, String)> = cfg
        .enabled()
        .into_iter()
        .map(|id| {
            let code = id.code().to_string();
            let st = cfg
                .strategies
                .status
                .get(&code)
                .cloned()
                .unwrap_or_else(|| "HYPOTHESIS".into());
            (code, st)
        })
        .collect();
    PreflightInput {
        target,
        notify_test,
        engine: cfg.engine.clone(),
        risk: cfg.risk.clone(),
        universe: cfg.universe(),
        enabled_strategies,
        env: EnvPresence::from_process_env(),
        config_hash,
        binary_version: env!("CARGO_PKG_VERSION").to_string(),
        git_commit: detect_git_commit_short(),
        allow_local_mock_hosts: false,
    }
}

pub async fn run_preflight_cli(
    cfg: &AllConfig,
    target_raw: &str,
    notify_test: bool,
) -> anyhow::Result<bool> {
    let target = PreflightTarget::parse(target_raw).map_err(anyhow::Error::msg)?;
    let inp = build_preflight_input(cfg, target, notify_test);
    let (exec_for_probe, rest_base) = match target {
        PreflightTarget::Demo => (ExecMode::Testnet, cfg.engine.testnet_rest_base.as_str()),
        PreflightTarget::Live => (ExecMode::Live, cfg.engine.live_rest_base.as_str()),
    };
    let rest_opt = match Credentials::from_env() {
        Ok(creds) => {
            BinanceRest::new(exec_for_probe, rest_base, creds, cfg.engine.recv_window_ms).ok()
        }
        Err(_) => None,
    };
    let backend = BinancePreflightBackend {
        rest: rest_opt.as_ref(),
        sapi_base: DEFAULT_SPOT_SAPI_BASE.to_string(),
        notifier: HttpWebhookNotifier::from_env(),
    };
    let rep = run_preflight(&inp, &backend).await;
    print!("{}", format_preflight_table(&rep));
    Ok(!rep.has_failure())
}

pub async fn run_demo_smoke_cli(cfg: &AllConfig) -> anyhow::Result<()> {
    let live_confirm = std::env::var(LIVE_CONFIRM_ENV).ok();
    let mode = authorize(&cfg.engine, live_confirm.as_deref())?;
    if mode != ExecMode::Testnet {
        anyhow::bail!(
            "apex demo-smoke hanya diizinkan saat engine.mode = \"testnet\" dan arm = true (ditemukan {mode:?})"
        );
    }
    check_base(mode, &cfg.engine.testnet_rest_base)?;
    let creds = Credentials::from_env()?;
    let rest = Arc::new(BinanceRest::new(
        mode,
        &cfg.engine.testnet_rest_base,
        creds,
        cfg.engine.recv_window_ms,
    )?);
    let (tx, rx) = mpsc::unbounded_channel();
    let hb = Arc::new(AtomicI64::new(0));
    let ws_task = tokio::spawn(run_user_stream_with_heartbeat(
        rest.clone(),
        cfg.engine.testnet_ws_base.clone(),
        tx,
        Some(hb.clone()),
    ));
    let wait_deadline = tokio::time::Instant::now() + Duration::from_secs(8);
    while hb.load(Ordering::Relaxed) == 0 && tokio::time::Instant::now() < wait_deadline {
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    let inp = build_preflight_input(cfg, PreflightTarget::Demo, false);
    let backend = BinancePreflightBackend {
        rest: Some(rest.as_ref()),
        sapi_base: DEFAULT_SPOT_SAPI_BASE.to_string(),
        notifier: HttpWebhookNotifier::from_env(),
    };
    let mut ev_source = ChannelEventSource { rx, wait_ms: 4_000 };
    let tag = format!("{}", now_ms() % 1_000_000_000);
    let res = run_demo_smoke(&inp, &backend, rest.as_ref(), &mut ev_source, &tag).await;
    ws_task.abort();
    let _ = rest.delete_listen_key().await;
    let rep = res?;
    println!(
        "DEMO SMOKE PASS: symbol={} mark={} limit_gtx={} (jarak {:.2}%) qty={} notional={} sl_trigger={} algo_id={}",
        rep.symbol,
        rep.mark_price,
        rep.limit_price,
        apex_core::num::to_f64(rep.distance_pct) * 100.0,
        rep.qty,
        rep.notional,
        rep.sl_trigger,
        rep.algo_id
    );
    for line in &rep.user_stream_lines {
        println!("  {line}");
    }
    println!(
        "  cleanup: canceled_orders={} canceled_algos={} flattened_positions={}",
        rep.cleanup.canceled_orders, rep.cleanup.canceled_algos, rep.cleanup.flattened_positions
    );
    Ok(())
}
