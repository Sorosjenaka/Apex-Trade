//! Implementasi detail pemeriksaan pra-terbang individu C1..C15 untuk WP-9a.

use crate::creds::{KEY_ENV, SECRET_ENV};
use crate::notifier::NOTIFY_URL_ENV;
use crate::preflight::{
    CheckVerdict, PreflightCheck, PreflightInput, PreflightTarget, MIN_FREE_DISK_BYTES,
};
use crate::preflight_http::PreflightBackend;
use crate::safety::{authorize, ExecMode, LIVE_CONFIRM_ENV, LIVE_CONFIRM_VALUE};
use apex_core::num::to_dec;
use apex_core::sizing::{size_order, EntryKind, SizingInput};
use apex_core::types::Side;
use rust_decimal::Decimal;
use std::collections::BTreeMap;

pub fn chk(id: &str, title: &str, verdict: CheckVerdict, detail: String) -> PreflightCheck {
    PreflightCheck {
        id: id.to_string(),
        title: title.to_string(),
        verdict,
        detail,
    }
}

pub fn check_c01_env(inp: &PreflightInput) -> PreflightCheck {
    let mut missing = Vec::new();
    if !inp.env.api_key_set {
        missing.push(KEY_ENV);
    }
    if !inp.env.api_secret_set {
        missing.push(SECRET_ENV);
    }
    if inp.target == PreflightTarget::Live && !inp.env.live_confirm_matches {
        missing.push(LIVE_CONFIRM_ENV);
    }
    let m = |b: bool| if b { "<masked>" } else { "<unset>" };
    let summary = format!(
        "{KEY_ENV}={} {SECRET_ENV}={} {LIVE_CONFIRM_ENV}={} {NOTIFY_URL_ENV}={}",
        m(inp.env.api_key_set), m(inp.env.api_secret_set), m(inp.env.live_confirm_set), m(inp.env.notify_url_set)
    );
    if missing.is_empty() {
        chk("C1", "Variabel environment", CheckVerdict::Pass, summary)
    } else {
        chk(
            "C1",
            "Variabel environment",
            CheckVerdict::Fail,
            format!(
                "variabel wajib hilang/tidak sesuai: [{}] ({summary})",
                missing.join(", ")
            ),
        )
    }
}

pub fn check_c02_mode_arm(inp: &PreflightInput) -> PreflightCheck {
    let lc = inp.env.live_confirm_matches.then_some(LIVE_CONFIRM_VALUE);
    let auth = authorize(&inp.engine, lc);
    let expected = match inp.target {
        PreflightTarget::Demo => ExecMode::Testnet,
        PreflightTarget::Live => ExecMode::Live,
    };
    match auth {
        Ok(m) if m == expected => {
            if inp.target == PreflightTarget::Demo && inp.env.live_confirm_set {
                chk(
                    "C2",
                    "Konsistensi mode/arm/konfirmasi",
                    CheckVerdict::Warn,
                    format!(
                        "mode={:?} arm={} cocok target demo namun {LIVE_CONFIRM_ENV} diset; config_hash={}",
                        inp.engine.mode, inp.engine.arm, inp.config_hash
                    ),
                )
            } else {
                chk(
                    "C2",
                    "Konsistensi mode/arm/konfirmasi",
                    CheckVerdict::Pass,
                    format!(
                        "mode={:?} arm={} terotorisasi ({m:?}); config_hash={}",
                        inp.engine.mode, inp.engine.arm, inp.config_hash
                    ),
                )
            }
        }
        Ok(m) => chk(
            "C2",
            "Konsistensi mode/arm/konfirmasi",
            CheckVerdict::Fail,
            format!(
                "mode {m:?} tidak cocok dengan --target {} (mode={:?}, arm={}); config_hash={}",
                inp.target.as_str(),
                inp.engine.mode,
                inp.engine.arm,
                inp.config_hash
            ),
        ),
        Err(e) => chk(
            "C2",
            "Konsistensi mode/arm/konfirmasi",
            CheckVerdict::Fail,
            format!("{e}; config_hash={}", inp.config_hash),
        ),
    }
}

pub async fn check_c03_clock<B: PreflightBackend>(
    inp: &PreflightInput,
    backend: &B,
) -> PreflightCheck {
    match backend.clock_offset_ms().await {
        Ok(off) if off.abs() <= inp.engine.max_clock_offset_ms => chk(
            "C3",
            "Selisih jam server (/fapi/v1/time)",
            CheckVerdict::Pass,
            format!(
                "offset={off} ms (batas ±{} ms)",
                inp.engine.max_clock_offset_ms
            ),
        ),
        Ok(off) => chk(
            "C3",
            "Selisih jam server (/fapi/v1/time)",
            CheckVerdict::Fail,
            format!(
                "offset={off} ms melebihi max_clock_offset_ms=±{} ms",
                inp.engine.max_clock_offset_ms
            ),
        ),
        Err(e) => chk(
            "C3",
            "Selisih jam server (/fapi/v1/time)",
            CheckVerdict::Fail,
            format!("gagal membaca waktu server: {e}"),
        ),
    }
}

pub fn validate_target_hosts(inp: &PreflightInput) -> Result<(String, String), String> {
    inp.engine.validate_ws_urls()?;
    let (rest, ws) = match inp.target {
        PreflightTarget::Demo => (
            inp.engine.testnet_rest_base.trim().to_string(),
            inp.engine.testnet_ws_base.trim().to_string(),
        ),
        PreflightTarget::Live => (
            inp.engine.live_rest_base.trim().to_string(),
            inp.engine.live_ws_base.trim().to_string(),
        ),
    };
    if inp.allow_local_mock_hosts
        && (rest.starts_with("http://127.0.0.1") || rest.starts_with("http://localhost"))
    {
        return Ok((rest, ws));
    }
    let rl = rest.to_ascii_lowercase();
    let wl = ws.to_ascii_lowercase();
    match inp.target {
        PreflightTarget::Demo => {
            if !rl.contains("demo-fapi.binance.com") || !wl.contains("demo-fstream.binance.com") {
                return Err(format!(
                    "host demo harus demo-fapi.binance.com & demo-fstream.binance.com, ditemukan rest='{rest}' ws='{ws}'"
                ));
            }
        }
        PreflightTarget::Live => {
            if rl.contains("demo-")
                || wl.contains("demo-")
                || !rl.contains("fapi.binance.com")
                || !wl.contains("fstream.binance.com")
            {
                return Err(format!(
                    "host live harus fapi.binance.com & fstream.binance.com, ditemukan rest='{rest}' ws='{ws}'"
                ));
            }
        }
    }
    Ok((rest, ws))
}

pub async fn check_c04_hosts_and_ws<B: PreflightBackend>(
    inp: &PreflightInput,
    backend: &B,
) -> PreflightCheck {
    let (rest, ws) = match validate_target_hosts(inp) {
        Ok(pair) => pair,
        Err(e) => {
            return chk(
                "C4",
                "Host REST/WS & handshake WS publik",
                CheckVerdict::Fail,
                e,
            )
        }
    };
    match backend.ws_public_handshake(&ws).await {
        Ok(url) => chk(
            "C4",
            "Host REST/WS & handshake WS publik",
            CheckVerdict::Pass,
            format!("rest={rest} ws_handshake_ok={url}"),
        ),
        Err(e) => chk(
            "C4",
            "Host REST/WS & handshake WS publik",
            CheckVerdict::Fail,
            format!("rest={rest} ws={ws}: {e}"),
        ),
    }
}

pub fn check_c05_strategy_go(inp: &PreflightInput) -> PreflightCheck {
    if inp.enabled_strategies.is_empty() {
        return chk(
            "C5",
            "Status GO strategi enabled",
            CheckVerdict::Fail,
            "daftar enabled kosong".into(),
        );
    }
    let non_go: Vec<String> = inp
        .enabled_strategies
        .iter()
        .filter(|(_, st)| !st.eq_ignore_ascii_case("GO"))
        .map(|(id, st)| format!("{id}={st}"))
        .collect();
    if non_go.is_empty() {
        chk(
            "C5",
            "Status GO strategi enabled",
            CheckVerdict::Pass,
            format!(
                "seluruh {} strategi enabled berstatus GO",
                inp.enabled_strategies.len()
            ),
        )
    } else if inp.target == PreflightTarget::Live {
        chk(
            "C5",
            "Status GO strategi enabled",
            CheckVerdict::Fail,
            format!(
                "target live menolak strategi belum GO: [{}]",
                non_go.join(", ")
            ),
        )
    } else {
        chk(
            "C5",
            "Status GO strategi enabled",
            CheckVerdict::Warn,
            format!(
                "mode demo mengizinkan uji forward namun strategi belum GO: [{}]",
                non_go.join(", ")
            ),
        )
    }
}

pub async fn check_c06_api_restrictions<B: PreflightBackend>(
    inp: &PreflightInput,
    backend: &B,
) -> PreflightCheck {
    match backend.api_restrictions().await {
        Ok(info) => {
            if info.enable_withdrawals || !info.enable_futures {
                return chk(
                    "C6",
                    "Izin API key (/sapi/v1/account/apiRestrictions)",
                    CheckVerdict::Fail,
                    format!(
                        "enableWithdrawals={} (wajib false), enableFutures={} (wajib true), ipRestrict={}",
                        info.enable_withdrawals, info.enable_futures, info.ip_restrict
                    ),
                );
            }
            if !info.ip_restrict {
                let v = if inp.target == PreflightTarget::Live {
                    CheckVerdict::Fail
                } else {
                    CheckVerdict::Warn
                };
                return chk(
                    "C6",
                    "Izin API key (/sapi/v1/account/apiRestrictions)",
                    v,
                    "ipRestrict=false (wajib true pada live)".into(),
                );
            }
            chk(
                "C6",
                "Izin API key (/sapi/v1/account/apiRestrictions)",
                CheckVerdict::Pass,
                "enableWithdrawals=false enableFutures=true ipRestrict=true".into(),
            )
        }
        Err(e) if inp.target == PreflightTarget::Demo => chk(
            "C6",
            "Izin API key (/sapi/v1/account/apiRestrictions)",
            CheckVerdict::Warn,
            format!(
                "endpoint Spot SAPI tidak tersedia pada key demo ({e}); verifikasi manual di UI Demo"
            ),
        ),
        Err(e) => chk(
            "C6",
            "Izin API key (/sapi/v1/account/apiRestrictions)",
            CheckVerdict::Fail,
            format!("gagal memverifikasi pembatasan API key: {e}"),
        ),
    }
}

pub async fn check_c07_account_mode<B: PreflightBackend>(backend: &B) -> PreflightCheck {
    let t = "Mode akun (one-way & single-asset)";
    match backend.account_mode().await {
        Ok(m) if !m.dual_side_position && !m.multi_assets_margin => chk(
            "C7",
            t,
            CheckVerdict::Pass,
            "dualSidePosition=false (one-way) & multiAssetsMargin=false (single-asset)".into(),
        ),
        Ok(m) => chk(
            "C7",
            t,
            CheckVerdict::Fail,
            format!(
                "mode akun salah: dualSidePosition={} (wajib false), multiAssetsMargin={} (wajib false)",
                m.dual_side_position, m.multi_assets_margin
            ),
        ),
        Err(e) => chk("C7", t, CheckVerdict::Fail, format!("gagal membaca mode akun: {e}")),
    }
}

pub async fn check_c08_symbol_configs<B: PreflightBackend>(
    inp: &PreflightInput,
    backend: &B,
) -> PreflightCheck {
    let rows = match backend.symbol_configs().await {
        Ok(r) => r,
        Err(e) => {
            return chk(
                "C8",
                "Konfigurasi margin & leverage universe (/fapi/v1/symbolConfig)",
                CheckVerdict::Fail,
                format!("gagal membaca symbolConfig: {e}"),
            );
        }
    };
    let map: BTreeMap<&str, _> = rows.iter().map(|r| (r.symbol.as_str(), r)).collect();
    let mut bad = Vec::new();
    let mut higher = Vec::new();
    for sym in &inp.universe {
        match map.get(sym.as_str()) {
            None => bad.push(format!("{sym}:tidak_ditemukan")),
            Some(cfg) => {
                let mt = cfg.margin_type.to_ascii_uppercase();
                let cross_ok = mt == "CROSSED" || mt == "CROSS";
                if !cross_ok || cfg.leverage < inp.risk.leverage {
                    bad.push(format!(
                        "{sym}:margin={},lev={}x",
                        cfg.margin_type, cfg.leverage
                    ));
                } else if cfg.leverage > inp.risk.leverage {
                    higher.push(format!("{sym}:{}x", cfg.leverage));
                }
            }
        }
    }
    if !bad.is_empty() {
        chk(
            "C8",
            "Konfigurasi margin & leverage universe (/fapi/v1/symbolConfig)",
            CheckVerdict::Fail,
            format!(
                "simbol gagal syarat {} >= {}x: [{}]",
                inp.risk.margin_type,
                inp.risk.leverage,
                bad.join(", ")
            ),
        )
    } else if !higher.is_empty() {
        chk(
            "C8",
            "Konfigurasi margin & leverage universe (/fapi/v1/symbolConfig)",
            CheckVerdict::Warn,
            format!(
                "CROSSED terpenuhi namun leverage akun > {}x pada [{}] (engine menyesuaikan saat boot)",
                inp.risk.leverage, higher.join(", ")
            ),
        )
    } else {
        chk(
            "C8",
            "Konfigurasi margin & leverage universe (/fapi/v1/symbolConfig)",
            CheckVerdict::Pass,
            format!(
                "{} simbol universe CROSSED {}x",
                inp.universe.len(),
                inp.risk.leverage
            ),
        )
    }
}

pub async fn check_c09_balance<B: PreflightBackend>(
    inp: &PreflightInput,
    backend: &B,
) -> PreflightCheck {
    let avail = match backend.usdt_available_balance().await {
        Ok(b) => b,
        Err(e) => {
            return chk(
                "C9",
                "Saldo tersedia (/fapi/v3/balance)",
                CheckVerdict::Fail,
                format!("gagal membaca saldo USDT: {e}"),
            );
        }
    };
    let max_notional_per_pos = inp.risk.risk_usd / inp.risk.sl_min_pct.max(1e-6);
    let margin_needed =
        (max_notional_per_pos / inp.risk.leverage.max(1) as f64) * inp.risk.max_positions as f64;
    if avail < inp.risk.equity_floor_usd || avail < margin_needed {
        chk(
            "C9",
            "Saldo tersedia (/fapi/v3/balance)",
            CheckVerdict::Fail,
            format!(
                "availableBalance=${avail:.2} di bawah lantai ${:.2} atau margin ${margin_needed:.2}",
                inp.risk.equity_floor_usd
            ),
        )
    } else if avail < inp.risk.equity_floor_usd + margin_needed {
        chk(
            "C9",
            "Saldo tersedia (/fapi/v3/balance)",
            CheckVerdict::Warn,
            format!(
                "availableBalance=${avail:.2} di atas lantai ${:.2} namun margin tipis (${margin_needed:.2})",
                inp.risk.equity_floor_usd
            ),
        )
    } else {
        chk(
            "C9",
            "Saldo tersedia (/fapi/v3/balance)",
            CheckVerdict::Pass,
            format!(
                "availableBalance=${avail:.2} >= lantai ${:.2} + margin maks ${margin_needed:.2}",
                inp.risk.equity_floor_usd
            ),
        )
    }
}

pub async fn check_c10_exchange_info_sizing<B: PreflightBackend>(
    inp: &PreflightInput,
    backend: &B,
) -> PreflightCheck {
    let (filters, marks) = match backend.exchange_filters_and_marks().await {
        Ok(pair) => pair,
        Err(e) => {
            return chk(
                "C10",
                "Kecocokan minNotional exchangeInfo vs sizing $1-risk",
                CheckVerdict::Fail,
                format!("gagal mengambil exchangeInfo/premiumIndex: {e}"),
            );
        }
    };
    let mut failed = Vec::new();
    for sym in &inp.universe {
        let Some(flt) = filters.get(sym) else {
            failed.push(format!("{sym}:missing_filter"));
            continue;
        };
        let Some(&mark) = marks.get(sym) else {
            failed.push(format!("{sym}:missing_mark"));
            continue;
        };
        for sl_frac in [Decimal::new(25, 3), Decimal::new(38, 3)] {
            let sl = mark * (Decimal::ONE - sl_frac);
            let res = size_order(&SizingInput {
                side: Side::Long,
                entry: mark,
                sl,
                kind: EntryKind::LimitMaker,
                filters: flt,
                risk: &inp.risk,
            });
            if let Err(reason) = res {
                failed.push(format!(
                    "{sym}(sl={}%,minNotional={},err={reason:?})",
                    sl_frac * Decimal::from(100),
                    flt.min_notional
                ));
                break;
            }
        }
    }
    if failed.is_empty() {
        chk(
            "C10",
            "Kecocokan minNotional exchangeInfo vs sizing $1-risk",
            CheckVerdict::Pass,
            format!(
                "seluruh {} simbol universe lolos sizing $1-risk",
                inp.universe.len()
            ),
        )
    } else {
        chk(
            "C10",
            "Kecocokan minNotional exchangeInfo vs sizing $1-risk",
            CheckVerdict::Fail,
            format!("simbol gagal minNotional/step: [{}]", failed.join(", ")),
        )
    }
}

pub async fn check_c11_commission_rates<B: PreflightBackend>(
    inp: &PreflightInput,
    backend: &B,
) -> PreflightCheck {
    let model_maker = to_dec(inp.risk.fee_maker);
    let model_taker = to_dec(inp.risk.fee_taker);
    let mut bad = Vec::new();
    for sym in &inp.universe {
        match backend.commission_rate(sym).await {
            Ok(rate) => {
                if rate.maker > model_maker || rate.taker > model_taker {
                    bad.push(format!(
                        "{sym}(maker={}>{model_maker} atau taker={}>{model_taker})",
                        rate.maker, rate.taker
                    ));
                }
            }
            Err(e) => bad.push(format!("{sym}:err({e})")),
        }
    }
    if bad.is_empty() {
        chk(
            "C11",
            "Tarif komisi akun vs model (/fapi/v1/commissionRate)",
            CheckVerdict::Pass,
            format!(
                "tarif nyata <= model (maker<={model_maker}, taker<={model_taker}) untuk {} simbol",
                inp.universe.len()
            ),
        )
    } else {
        chk(
            "C11",
            "Tarif komisi akun vs model (/fapi/v1/commissionRate)",
            CheckVerdict::Fail,
            format!("tarif nyata melebihi model: [{}]", bad.join(", ")),
        )
    }
}

pub async fn check_c12_listen_key<B: PreflightBackend>(backend: &B) -> PreflightCheck {
    let t = "Siklus hidup listenKey (POST + DELETE)";
    match backend.probe_listen_key().await {
        Ok(()) => chk(
            "C12",
            t,
            CheckVerdict::Pass,
            "listenKey berhasil dibuat (nilai dimasker) dan langsung dihapus".into(),
        ),
        Err(e) => chk("C12", t, CheckVerdict::Fail, format!("gagal membuat/menghapus listenKey: {e}")),
    }
}

pub async fn check_c13_state_fsync_disk<B: PreflightBackend>(
    inp: &PreflightInput,
    backend: &B,
) -> PreflightCheck {
    match backend
        .probe_state_fsync_and_disk(&inp.engine.state_dir)
        .await
    {
        Ok(free) if free >= MIN_FREE_DISK_BYTES => chk(
            "C13",
            "Direktori state writable, fsync, & ruang disk",
            CheckVerdict::Pass,
            format!(
                "state_dir={} fsync_ok ruang_kosong={} MiB",
                inp.engine.state_dir.display(),
                free / (1024 * 1024)
            ),
        ),
        Ok(free) => chk(
            "C13",
            "Direktori state writable, fsync, & ruang disk",
            CheckVerdict::Fail,
            format!(
                "ruang disk {} B di bawah minimum {} MiB",
                free,
                MIN_FREE_DISK_BYTES / (1024 * 1024)
            ),
        ),
        Err(e) => chk(
            "C13",
            "Direktori state writable, fsync, & ruang disk",
            CheckVerdict::Fail,
            e,
        ),
    }
}
