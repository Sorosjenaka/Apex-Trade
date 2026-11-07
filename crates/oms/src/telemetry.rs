//! Telemetri per-trade yang jujur untuk evaluasi pasca-eksekusi (`WP-5`).
//!
//! Prinsip desain (Fase C):
//! Dengan frekuensi sekitar ±4 trade per bulan, adaptasi parameter secara online hampir pasti
//! melakukan overfit terhadap noise sampel kecil. Oleh sebab itu, "learning loop" di Apex-Trade
//! dirancang murni sebagai **proses riset yang terinstrumentasi**:
//! 1. Rekam data konteks sinyal, fill, biaya, dan excursion secara jujur tanpa default fiktif.
//! 2. Agregasi lintas sampel dengan interval kepercayaan statistik (`apex review`).
//! 3. Hasilkan *proposal bertanda hipotesis* untuk ditinjau manusia.
//! 4. Proposal hanya boleh berlaku bila lulus pipeline validasi OOS/walk-forward yang sudah ada.
//!
//! **Tidak ada perubahan parameter otomatis dan tidak ada ML di jalur pengiriman order.**
//!
//! Keterbatasan resolusi `excursion`:
//! MFE/MAE dihitung dari `high`/`low` kline tertutup 15m **setelah** bar tempat fill terjadi
//! (bar tempat fill dieksekusi sengaja dikecualikan agar extremum sebelum fill pada bar yang sama
//! tidak mencemari perhitungan). Karena resolusinya bar 15m (bukan tick), urutan intrabar antara
//! `high` dan `low` di dalam satu candle 15m tidak diketahui.

use crate::manager::ManagedTrade;
use crate::safety::ExecMode;
use crate::store::StoreError;
use apex_core::config::RiskConfig;
use apex_core::num::to_dec;
use apex_core::risk::ExitKind;
use apex_core::sizing::EntryKind;
use apex_core::types::{Bar, Ms, Side};
use rust_decimal::Decimal;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::io::Write;
use std::path::Path;

pub const TELEMETRY_SCHEMA_VERSION: u8 = 1;
const BAR_15M_MS: Ms = 900_000;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum ExecSource {
    #[default]
    Paper,
    Demo,
    Live,
}

impl ExecSource {
    pub fn from_mode(mode: ExecMode) -> Self {
        match mode {
            ExecMode::Paper => ExecSource::Paper,
            ExecMode::Testnet => ExecSource::Demo,
            ExecMode::Live => ExecSource::Live,
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            ExecSource::Paper => "paper",
            ExecSource::Demo => "demo",
            ExecSource::Live => "live",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, Default)]
pub enum TelemetryExitKind {
    SlHit,
    Tp1,
    Tp2,
    Trail,
    TimeStop,
    TrendFlip,
    FlattenProtect,
    FlattenKill,
    Manual,
    #[default]
    Unknown,
}

impl TelemetryExitKind {
    pub fn from_oms_exit(kind: Option<ExitKind>) -> Self {
        match kind {
            Some(ExitKind::StopLoss) => TelemetryExitKind::SlHit,
            Some(ExitKind::TakeProfit) => TelemetryExitKind::Tp1,
            Some(ExitKind::TimeStop) => TelemetryExitKind::TimeStop,
            Some(ExitKind::Manual) => TelemetryExitKind::Manual,
            Some(ExitKind::Liquidation) | Some(ExitKind::EndOfData) | None => {
                TelemetryExitKind::Unknown
            }
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            TelemetryExitKind::SlHit => "SlHit",
            TelemetryExitKind::Tp1 => "Tp1",
            TelemetryExitKind::Tp2 => "Tp2",
            TelemetryExitKind::Trail => "Trail",
            TelemetryExitKind::TimeStop => "TimeStop",
            TelemetryExitKind::TrendFlip => "TrendFlip",
            TelemetryExitKind::FlattenProtect => "FlattenProtect",
            TelemetryExitKind::FlattenKill => "FlattenKill",
            TelemetryExitKind::Manual => "Manual",
            TelemetryExitKind::Unknown => "Unknown",
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(default)]
pub struct CouncilSummary {
    pub action: String,
    pub enforced: bool,
    pub inputs_defaulted: Vec<String>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, Default)]
#[serde(default)]
pub struct EntryContext {
    pub spread_bps: Option<Decimal>,
    pub planned_slip_bps: Decimal,
    pub cvd_aligned: Option<bool>,
    pub chop: Option<f64>,
    pub adx: Option<f64>,
    pub atr_pct: Option<f64>,
    pub regime_label: String,
    pub macro_bias: String,
    pub council: Option<CouncilSummary>,
    pub config_hash: String,
    pub signal_ts: Ms,
    pub planned_entry: Decimal,
    pub planned_sl: Decimal,
    pub planned_tp: Decimal,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct TradeExcursion {
    pub mfe_r: Decimal,
    pub mae_r: Decimal,
    pub bars_in_trade: u32,
    pub mfe_ts: Option<Ms>,
    pub mae_ts: Option<Ms>,
    pub resolution: String,
}

impl Default for TradeExcursion {
    fn default() -> Self {
        Self {
            mfe_r: Decimal::ZERO,
            mae_r: Decimal::ZERO,
            bars_in_trade: 0,
            mfe_ts: None,
            mae_ts: None,
            resolution: "15m_closed_bar".to_string(),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(default)]
pub struct FillSummary {
    pub entry_slip_bps: Decimal,
    pub exit_slip_bps: Decimal,
    pub model_slip_r: Decimal,
    pub realized_slip_r: Decimal,
    pub fee_usd: Decimal,
    pub funding_usd: Decimal,
    pub maker_entry: bool,
    pub source: ExecSource,
    pub paper_slip_zero_by_construction: bool,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct TradeTelemetry {
    pub schema: u8,
    pub trade_id: String,
    pub symbol: String,
    pub strategy: String,
    #[serde(default)]
    pub validated: bool,
    pub side: Side,
    pub created_ms: Ms,
    #[serde(default)]
    pub filled_ms: Option<Ms>,
    #[serde(default)]
    pub fill_confirmed_at: Option<Ms>,
    #[serde(default)]
    pub sl_placed_ms: Option<Ms>,
    #[serde(default)]
    pub unprotected_ms: Option<i64>,
    pub closed_ms: Ms,
    pub entry_price: Decimal,
    pub exit_price: Decimal,
    pub planned_risk_usd: Decimal,
    pub pnl_net_usd: Decimal,
    pub r_multiple: Decimal,
    pub exit_kind: TelemetryExitKind,
    #[serde(default)]
    pub entry_ctx: Option<EntryContext>,
    #[serde(default)]
    pub excursion: Option<TradeExcursion>,
    #[serde(default)]
    pub fills: FillSummary,
}

/// Hash deterministik SHA-256 (16 karakter hex awal) atas representasi JSON konfigurasi aktif.
pub fn compute_config_hash(v: &serde_json::Value) -> String {
    let hex_full = hex::encode(Sha256::digest(v.to_string().as_bytes()));
    hex_full.chars().take(16).collect()
}

/// Hitung MFE/MAE dalam satuan R (`|planned_entry - planned_sl|`) dari bar tertutup 15m
/// yang terbuka **setelah** bar tempat fill terjadi (`open_time > fill_bar_open && open_time < closed_ms`).
pub fn compute_excursion_15m(
    side: Side,
    planned_entry: Decimal,
    planned_sl: Decimal,
    filled_ms: Ms,
    closed_ms: Ms,
    bars_15m: &[Bar],
) -> Option<TradeExcursion> {
    let r_dist = (planned_entry - planned_sl).abs();
    if planned_entry <= Decimal::ZERO || r_dist <= Decimal::ZERO || closed_ms <= filled_ms {
        return None;
    }
    let fill_bar_open = filled_ms - filled_ms.rem_euclid(BAR_15M_MS);
    let mut mfe_r = Decimal::ZERO;
    let mut mae_r = Decimal::ZERO;
    let mut mfe_ts: Option<Ms> = None;
    let mut mae_ts: Option<Ms> = None;
    let mut count: u32 = 0;

    for b in bars_15m
        .iter()
        .filter(|b| b.open_time > fill_bar_open && b.open_time < closed_ms)
    {
        count = count.saturating_add(1);
        let high = to_dec(b.high);
        let low = to_dec(b.low);
        let bar_close_ts = b.open_time + BAR_15M_MS;
        let (fav, adv) = match side {
            Side::Long => (
                ((high - planned_entry) / r_dist).max(Decimal::ZERO),
                ((planned_entry - low) / r_dist).max(Decimal::ZERO),
            ),
            Side::Short => (
                ((planned_entry - low) / r_dist).max(Decimal::ZERO),
                ((high - planned_entry) / r_dist).max(Decimal::ZERO),
            ),
        };
        if mfe_ts.is_none() || fav > mfe_r {
            mfe_r = fav.round_dp(4);
            mfe_ts = Some(bar_close_ts);
        }
        if mae_ts.is_none() || adv > mae_r {
            mae_r = adv.round_dp(4);
            mae_ts = Some(bar_close_ts);
        }
    }
    if count == 0 {
        return None;
    }
    Some(TradeExcursion {
        mfe_r,
        mae_r,
        bars_in_trade: count,
        mfe_ts,
        mae_ts,
        resolution: "15m_closed_bar".to_string(),
    })
}

fn side_sign_dec(side: Side) -> Decimal {
    match side {
        Side::Long => Decimal::ONE,
        Side::Short => -Decimal::ONE,
    }
}

fn compute_slip_r(
    trade: &ManagedTrade,
    exit_kind: TelemetryExitKind,
    fill_in: Decimal,
    exit_px: Decimal,
    ref_exit: Option<Decimal>,
    risk_cfg: &RiskConfig,
    paper_zero: bool,
) -> (Decimal, Decimal) {
    let sgn = side_sign_dec(trade.side);
    let denom_r = if trade.planned_risk > Decimal::ZERO {
        trade.planned_risk
    } else {
        (trade.filled_qty * (trade.entry_price - trade.sl).abs()).max(Decimal::ONE)
    };
    let model_slip_usd = if exit_kind == TelemetryExitKind::SlHit {
        trade.filled_qty * trade.sl * to_dec(risk_cfg.slip_sl)
    } else {
        Decimal::ZERO
    };
    let model_r = (model_slip_usd / denom_r).round_dp(4);
    let realized_r = if paper_zero {
        model_r
    } else {
        let in_usd = sgn * (fill_in - trade.entry_price) * trade.filled_qty;
        let out_usd = ref_exit
            .map(|r_px| sgn * (r_px - exit_px) * trade.filled_qty)
            .unwrap_or(Decimal::ZERO);
        ((in_usd + out_usd) / denom_r).round_dp(4)
    };
    (model_r, realized_r)
}

/// Hitung ringkasan kualitas fill dan perbandingan slippage realisasi terhadap model `RiskConfig`.
pub fn compute_fill_summary(
    trade: &ManagedTrade,
    exit_kind: TelemetryExitKind,
    risk_cfg: &RiskConfig,
    mode: ExecMode,
) -> FillSummary {
    let source = ExecSource::from_mode(mode);
    let sgn = side_sign_dec(trade.side);
    let ten_k = Decimal::from(10_000);
    let fill_in = if trade.avg_fill > Decimal::ZERO {
        trade.avg_fill
    } else {
        trade.entry_price
    };
    let entry_slip_bps = if trade.entry_price > Decimal::ZERO {
        (sgn * (fill_in - trade.entry_price) / trade.entry_price * ten_k).round_dp(4)
    } else {
        Decimal::ZERO
    };
    let exit_px = trade.exit_price.unwrap_or(trade.sl);
    let ref_exit = match exit_kind {
        TelemetryExitKind::SlHit => Some(trade.sl),
        TelemetryExitKind::Tp1 | TelemetryExitKind::Tp2 => Some(trade.tp),
        _ => None,
    };
    let exit_slip_bps = match ref_exit {
        Some(r_px) if r_px > Decimal::ZERO => (sgn * (r_px - exit_px) / r_px * ten_k).round_dp(4),
        _ => Decimal::ZERO,
    };
    let paper_zero = source == ExecSource::Paper;
    let (model_slip_r, realized_slip_r) = compute_slip_r(
        trade, exit_kind, fill_in, exit_px, ref_exit, risk_cfg, paper_zero,
    );
    FillSummary {
        entry_slip_bps,
        exit_slip_bps,
        model_slip_r,
        realized_slip_r,
        fee_usd: trade.fees.round_dp(6),
        funding_usd: trade.funding_usd.round_dp(6),
        maker_entry: trade.entry_kind == EntryKind::LimitMaker,
        source,
        paper_slip_zero_by_construction: paper_zero,
    }
}

/// Bangun satu record `TradeTelemetry` (`schema: 1`) dari `ManagedTrade` tertutup tanpa menduplikasi state.
pub fn build_trade_telemetry(
    trade: &ManagedTrade,
    risk_cfg: &RiskConfig,
    mode: ExecMode,
    bars_15m: &[Bar],
) -> TradeTelemetry {
    let exit_kind = trade
        .telemetry_exit
        .unwrap_or_else(|| TelemetryExitKind::from_oms_exit(trade.exit_kind));
    let closed_ms = trade.closed_ms.unwrap_or(trade.created_ms);
    let entry_px = if trade.avg_fill > Decimal::ZERO {
        trade.avg_fill
    } else {
        trade.entry_price
    };
    let pnl_dec = to_dec(trade.pnl_net.unwrap_or(0.0)).round_dp(6);
    let denom_r = if trade.planned_risk > Decimal::ZERO {
        trade.planned_risk
    } else {
        to_dec(risk_cfg.risk_usd).max(Decimal::ONE)
    };
    let excursion = trade.excursion.clone().or_else(|| {
        compute_excursion_15m(
            trade.side,
            trade.entry_price,
            trade.sl,
            trade.filled_ms?,
            closed_ms,
            bars_15m,
        )
    });
    let fills = trade
        .fills
        .clone()
        .unwrap_or_else(|| compute_fill_summary(trade, exit_kind, risk_cfg, mode));
    let unprotected_ms = match (
        trade.fill_confirmed_at.or(trade.filled_ms),
        trade.sl_placed_ms,
    ) {
        (Some(f), Some(s)) => Some((s - f).max(0)),
        _ => None,
    };
    TradeTelemetry {
        schema: TELEMETRY_SCHEMA_VERSION,
        trade_id: trade.client_id.clone(),
        symbol: trade.symbol.clone(),
        strategy: trade.strategy.clone(),
        validated: trade.validated,
        side: trade.side,
        created_ms: trade.created_ms,
        filled_ms: trade.filled_ms,
        fill_confirmed_at: trade.fill_confirmed_at,
        sl_placed_ms: trade.sl_placed_ms,
        unprotected_ms,
        closed_ms,
        entry_price: entry_px,
        exit_price: trade.exit_price.unwrap_or(trade.sl),
        planned_risk_usd: trade.planned_risk,
        pnl_net_usd: pnl_dec,
        r_multiple: (pnl_dec / denom_r).round_dp(4),
        exit_kind,
        entry_ctx: trade.entry_ctx.clone(),
        excursion,
        fills,
    }
}

/// Tambahkan satu baris JSON ke file JSONL secara durable (`sync_data`).
pub fn append_jsonl_record<T: Serialize>(path: &Path, rec: &T) -> Result<(), StoreError> {
    if let Some(parent) = path.parent() {
        if !parent.as_os_str().is_empty() {
            std::fs::create_dir_all(parent).map_err(|source| StoreError::Io {
                op: "create_jsonl_dir",
                source,
            })?;
        }
    }
    let mut line = serde_json::to_vec(rec).map_err(StoreError::Serialize)?;
    line.push(b'\n');
    let mut fh = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)
        .map_err(|source| StoreError::Io {
            op: "open_jsonl",
            source,
        })?;
    fh.write_all(&line).map_err(|source| StoreError::Io {
        op: "write_jsonl",
        source,
    })?;
    fh.sync_data().map_err(|source| StoreError::Io {
        op: "sync_jsonl",
        source,
    })?;
    Ok(())
}

/// Baca seluruh baris `TradeTelemetry` dari file JSONL (mengembalikan `Vec` kosong bila file belum ada).
pub fn read_telemetry_jsonl(path: &Path) -> Result<Vec<TradeTelemetry>, StoreError> {
    if !path.exists() {
        return Ok(Vec::new());
    }
    let raw = std::fs::read_to_string(path).map_err(|source| StoreError::Io {
        op: "read_telemetry_jsonl",
        source,
    })?;
    let mut out = Vec::new();
    for (idx, line) in raw.lines().enumerate() {
        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }
        let item: TradeTelemetry =
            serde_json::from_str(trimmed).map_err(|source| StoreError::Deserialize {
                path: format!("{}:{}", path.display(), idx + 1),
                source,
            })?;
        out.push(item);
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use rust_decimal_macros::dec;

    fn mk_bar(open_time: Ms, open: f64, high: f64, low: f64, close: f64) -> Bar {
        Bar {
            open_time,
            open,
            high,
            low,
            close,
            volume: 100.0,
            quote_volume: 10_000.0,
            trades: 50,
            taker_buy_base: 50.0,
        }
    }

    #[test]
    fn excursion_excludes_fill_bar_for_long_and_short_and_sl_is_one_r() {
        // Long: entry 100, SL 98 (1R = 2). Bar fill [900_000, 1_800_000) punya spike 120/80 → wajib dikecualikan.
        let bars_l = vec![
            mk_bar(900_000, 100.0, 120.0, 80.0, 100.5),
            mk_bar(1_800_000, 100.5, 102.4, 99.6, 101.0),
            mk_bar(2_700_000, 101.0, 101.2, 98.0, 98.0),
        ];
        let ex_l = compute_excursion_15m(
            Side::Long,
            dec!(100),
            dec!(98),
            1_100_000,
            3_600_000,
            &bars_l,
        )
        .unwrap();
        assert_eq!(
            (ex_l.bars_in_trade, ex_l.mfe_r, ex_l.mae_r),
            (2, dec!(1.2), dec!(1))
        );

        // Short: entry 200, SL 206 (1R = 6). Bar fill [0, 900_000) dikecualikan.
        let bars_s = vec![
            mk_bar(0, 200.0, 250.0, 150.0, 200.0),
            mk_bar(900_000, 200.0, 201.5, 194.0, 195.0),
            mk_bar(1_800_000, 195.0, 206.0, 195.0, 206.0),
        ];
        let ex_s = compute_excursion_15m(
            Side::Short,
            dec!(200),
            dec!(206),
            300_000,
            2_700_000,
            &bars_s,
        )
        .unwrap();
        assert_eq!(
            (ex_s.bars_in_trade, ex_s.mfe_r, ex_s.mae_r),
            (2, dec!(1), dec!(1))
        );
    }
}
