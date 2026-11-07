//! Port 1:1 sistem Scanner, Asset-Class Guard, Liquidation Map, Multi-Window Delta,
//! dan 5-Panel Microstructure Insights ArtaTrade
//! — khusus mode SCALP (tanpa strategi SWING `RSW`).

use apex_core::types::Bar;
use apex_indicators::{
    adx_last_yahudi, build_knn_features_yahudi, build_volume_profile_48,
    calculate_beta_btc_with_status, calculate_efficiency_ratio, calculate_zero_lag_trend,
    choppiness_last_yahudi, classify_regime_smoothed, compute_oi_change_pct_from_samples,
    detect_usable_zones, distance_from_poc_in_bins, evaluate_squeeze_trigger,
    evaluate_volatility_shield, query_knn_lorentzian, KnnAnalogRecord, TrendDir, VolumeProfile48,
};
use serde::{Deserialize, Serialize};

use crate::yahudi_hub::{TierFlowSummary, WallLifelinesSummary};

// ─────────────────────── 1. Asset-Class & Fan-Token Guard (assetClassGuard.ts) ───────────────────────

pub const COMMODITY_BASES: &[&str] = &[
    "BZ", "BRENT", "CL", "WTI", "OIL", "CRUDE", "NG", "NATGAS", "XAU", "GOLD", "PAXG", "XAG",
    "SILVER", "XPT", "PLATINUM", "XPD", "PALLADIUM", "HG", "COPPER", "IRON", "STEEL", "URANIUM",
    "COAL", "WHEAT", "CORN", "SOY", "SOYBEAN", "COFFEE", "SUGAR", "COCOA", "COTTON", "ALUMINUM",
    "NICKEL", "ZINC", "LEAD", "LUMBER", "RICE", "OATS", "ETHANOL",
];

pub const ETF_BASES: &[&str] = &[
    "SPY", "QQQ", "DIA", "IWM", "EEM", "EWY", "EWJ", "EWT", "EWZ", "FXI", "KORU", "KODEX200",
    "CSOPSKHYNIX2L", "CSOPSAMSUNG2L", "BOT", "SP500", "US500", "NAS100", "US30", "DXY", "VIX",
    "UVXY", "SVXY", "VXX", "TQQQ", "SQQQ", "SOXL", "SOXS", "TZA", "TMF", "TBT", "NVDL", "TSLL",
    "XLE", "XLF", "XLK", "XLI", "XLU", "XLP", "XLY", "XLV", "XLB", "XLC", "XBI", "IBB", "GDX",
    "GDXJ", "SMH", "SOXX", "URNM", "ARKK", "BITO", "HYG", "LQD", "TLT", "AGG", "GLD", "SLV",
    "USO", "UNG", "DBC", "IYR", "VNQ",
];

pub const EQUITY_BASES: &[&str] = &[
    "AAPL", "ABNB", "ADBE", "AGPU", "ALAB", "AMAT", "AMC", "AMD", "AMZN", "ANET", "APLD", "APP",
    "ARM", "ASTS", "AVGO", "AXTI", "BA", "BABA", "BBX", "BE", "BILI", "BMNR", "BRK", "BRKB",
    "BSP", "BX", "CAT", "CBRS", "CIEN", "CLSK", "COHR", "COIN", "COKE", "COST", "CRCL", "CRDO",
    "CRM", "CRWD", "CRWV", "CSCO", "CXMT", "CYPH", "DDOG", "DE", "DELL", "DIS", "DJI", "DJT",
    "DKNG", "DRAM", "EBAY", "F", "FLEX", "FLNC", "FWDI", "GE", "GEV", "GIGADEV", "GLW", "GM",
    "GME", "GOOG", "GOOGL", "GPRO", "GRAB", "GS", "GTLB", "HD", "HIMS", "HPE", "HOOD", "HUT",
    "IBM", "INTC", "INTW", "IONQ", "IREN", "JNJ", "JPM", "KLAC", "KO", "KSTR", "LITE", "LLY",
    "LRCX", "LYTE", "MA", "MARA", "MCD", "MDB", "META", "MINIMAX", "MOONSHOT", "MRK", "MRNA",
    "MRVL", "MS", "MSFT", "MSTR", "MU", "MUU", "MVLL", "NBIS", "NET", "NFLX", "NOK", "NOW",
    "NVDA", "NVO", "ONDS", "OPENAI", "ORCL", "OURA", "PANW", "PATH", "PAYP", "PDD", "PENG",
    "PEP", "PFE", "PLTR", "POPMART", "PYPL", "QCOM", "QNTX", "RAM", "RDDT", "RIOT", "RIVN",
    "RKLB", "SCHW", "SE", "SHAZ", "SHOP", "SKDD", "SKUU", "SMCI", "SNDK", "SNOW", "SNXX",
    "SOFI", "SPCX", "SPX", "SQ", "STRC", "STXX", "TEAM", "TEM", "TER", "TM", "TSLA", "TSM",
    "TTWO", "TXN", "UBER", "UNITREE", "UPST", "USAR", "V", "VRT", "VST", "WDC", "WEN", "WFC",
    "WMT", "ZHIPU", "ZM", "ZS", "BYD", "HANMI", "HK0625", "HK0700", "HK0992", "HK1810",
    "HYUNDAI", "KUAISHOU", "LGELECTRONICS", "MEITUAN", "NAVER", "SAMSUNG", "SAMSUNGEM", "SKHY",
    "SKHYNIX", "SONY", "TENCENT", "ZHONGJI", "ANTHROPIC",
];

pub const FAN_TOKEN_BASES: &[&str] = &[
    "ACM", "AFC", "ALPINE", "ASR", "ATM", "BAR", "CITY", "GAL", "JUV", "LAZ", "LAZIO", "LEG",
    "MENGO", "NAP", "OG", "PSG", "POR", "PORTO", "SANTOS", "SPFC", "TRA", "ESPORTS", "SPORTFUN",
];

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct AssetClassCheck {
    pub symbol: String,
    pub blocked: bool,
    pub reason: Option<String>,
    pub base_asset: String,
    pub asset_class: &'static str,
}

/// Pemeriksaan kelas aset crypto-only (`assetClassGuard.ts:checkAssetClass`).
pub fn check_asset_class(symbol: &str) -> AssetClassCheck {
    let s = symbol.trim().to_uppercase();
    if s.len() < 5
        || !s.ends_with("USDT")
        || !s.chars().all(|c| c.is_ascii_uppercase() || c.is_ascii_digit())
    {
        return AssetClassCheck {
            symbol: s.clone(),
            blocked: true,
            reason: Some("SYMBOL_FORMAT_INVALID".into()),
            base_asset: s,
            asset_class: "INVALID",
        };
    }
    if s == "BTCDOMUSDT" || s == "DEFIUSDT" {
        let base_asset = s.trim_end_matches("USDT").to_string();
        return AssetClassCheck {
            symbol: s,
            blocked: true,
            reason: Some("INDEX_DATA_ONLY".into()),
            base_asset,
            asset_class: "INDEX",
        };
    }
    let raw_base = s.trim_end_matches("USDT");
    let clean_base = raw_base
        .strip_prefix("1000000")
        .or_else(|| raw_base.strip_prefix("1000"))
        .unwrap_or(raw_base)
        .to_string();

    if FAN_TOKEN_BASES.contains(&clean_base.as_str()) {
        return AssetClassCheck {
            symbol: s,
            blocked: true,
            reason: Some("fan-token:operator".into()),
            base_asset: clean_base,
            asset_class: "OTHER",
        };
    }
    if COMMODITY_BASES.contains(&clean_base.as_str()) {
        return AssetClassCheck {
            symbol: s,
            blocked: true,
            reason: Some("NON_CRYPTO_COMMODITY_BLOCKED".into()),
            base_asset: clean_base,
            asset_class: "COMMODITY",
        };
    }
    if ETF_BASES.contains(&clean_base.as_str()) {
        return AssetClassCheck {
            symbol: s,
            blocked: true,
            reason: Some("NON_CRYPTO_ETF_BLOCKED".into()),
            base_asset: clean_base,
            asset_class: "INDEX",
        };
    }
    if EQUITY_BASES.contains(&clean_base.as_str()) {
        return AssetClassCheck {
            symbol: s,
            blocked: true,
            reason: Some("NON_CRYPTO_EQUITY_BLOCKED".into()),
            base_asset: clean_base,
            asset_class: "EQUITY",
        };
    }
    AssetClassCheck {
        symbol: s,
        blocked: false,
        reason: None,
        base_asset: clean_base,
        asset_class: "CRYPTO",
    }
}

#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct UniverseAuditRow {
    pub symbol: String,
    pub base_asset: String,
    pub quote_asset: String,
    pub asset_class: &'static str,
    pub is_blacklisted: bool,
    pub blacklist_reason: Option<String>,
    pub volume_24h_usd: f64,
    pub eligible: bool,
    pub eligible_reason: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct UniverseAuditSummary {
    pub total_symbols: usize,
    pub crypto_perps_count: usize,
    pub eligible_count: usize,
    pub non_crypto_filtered: usize,
    pub manual_blacklist_count: usize,
    pub volume_min_usd: f64,
    pub symbols: Vec<UniverseAuditRow>,
}

pub fn audit_universe_symbols(
    tickers: &[(String, f64)],
    manual_blacklist: &[String],
    volume_min_usd: f64,
) -> UniverseAuditSummary {
    let mut rows = Vec::with_capacity(tickers.len());
    let mut crypto_perps_count = 0usize;
    let mut eligible_count = 0usize;
    let mut non_crypto_filtered = 0usize;
    let mut manual_blacklist_count = 0usize;

    for (sym, vol) in tickers {
        let guard = check_asset_class(sym);
        let manual_bl = manual_blacklist.iter().any(|b| b.eq_ignore_ascii_case(sym));
        let is_blacklisted = guard.blocked || manual_bl;
        let blacklist_reason = if guard.blocked {
            non_crypto_filtered += 1;
            guard.reason.clone()
        } else if manual_bl {
            manual_blacklist_count += 1;
            Some("MANUAL_BLACKLIST".into())
        } else {
            crypto_perps_count += 1;
            None
        };

        let (eligible, eligible_reason) = if is_blacklisted {
            (false, blacklist_reason.clone())
        } else if !vol.is_finite() || *vol <= volume_min_usd {
            (false, Some("VOLUME_BELOW_MIN".into()))
        } else {
            eligible_count += 1;
            (true, None)
        };

        rows.push(UniverseAuditRow {
            symbol: guard.symbol,
            base_asset: guard.base_asset,
            quote_asset: "USDT".into(),
            asset_class: guard.asset_class,
            is_blacklisted,
            blacklist_reason,
            volume_24h_usd: *vol,
            eligible,
            eligible_reason,
        });
    }

    UniverseAuditSummary {
        total_symbols: tickers.len(),
        crypto_perps_count,
        eligible_count,
        non_crypto_filtered,
        manual_blacklist_count,
        volume_min_usd,
        symbols: rows,
    }
}

// ─────────────────────── 2. 7-Gate Candidate Scanner (/api/markets/scan) ───────────────────────

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct GatesPass {
    pub volume_gate: bool,
    pub regime_gate: bool,
    pub trend_gate: bool,
    pub score_gate: bool,
    pub plan_gate: bool,
    pub chop_gate: bool,
    pub volatility_gate: bool,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct PlanCandidateRow {
    pub strategy_id: String,
    pub matched: bool,
    pub quality: u32,
    pub reasons: Vec<String>,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct ScanCandidateRow {
    pub symbol: String,
    pub score: u32,
    pub mode: &'static str,
    pub regime: &'static str,
    pub zone_score: u32,
    pub trigger_score: u32,
    pub trend_score: u32,
    pub knn_prob: Option<f64>,
    pub knn_n: usize,
    pub knn_label: &'static str,
    pub gates_pass: GatesPass,
    pub status: &'static str,
    pub reject_reason: Option<String>,
    pub chop: Option<f64>,
    pub chop_adx: Option<f64>,
    pub structure_atr_pct: Option<f64>,
    pub er: Option<f64>,
    pub er_status: &'static str,
    pub poc_distance_bins: Option<f64>,
    pub oi_change_pct: Option<f64>,
    pub oi_status: &'static str,
    pub oi_reason: String,
    pub beta_btc: Option<f64>,
    pub beta_status: &'static str,
    pub beta_reason: String,
    pub bias_chop: Option<f64>,
    pub bias_adx: Option<f64>,
    pub bias_trend: &'static str,
    pub elig_reasons: Vec<String>,
    pub strategy_id: Option<String>,
    pub strategy_name: Option<String>,
    pub entry_type: Option<String>,
    pub plan_candidates: Vec<PlanCandidateRow>,
    pub zone_type: Option<&'static str>,
    pub zone_top: Option<f64>,
    pub zone_bottom: Option<f64>,
    pub tf_summary: &'static str,
    pub mtf_analyzed: bool,
    pub mtf_analyzed_at: i64,
}

/// Evaluasi lengkap satu kandidat Scalping untuk tabel `/api/markets/scan` (`marketsService.ts`).
/// Hanya mengevaluasi `S-01` (Liquidity Sweep Reclaim) & `S-02` (Liquidity Sniper) — tanpa `RSW` Swing.
pub fn evaluate_scan_candidate(
    symbol: &str,
    bars_15m: &[Bar],
    bars_5m: &[Bar],
    btc_bars_5m: Option<&[Bar]>,
    volume_24h_usd: f64,
    volume_min_usd: f64,
    knn_history: &[KnnAnalogRecord],
    s01_matched: Option<(u32, Vec<String>)>,
    s02_matched: Option<(u32, Vec<String>)>,
    now_ms: i64,
) -> ScanCandidateRow {
    let guard = check_asset_class(symbol);
    let volume_gate = !guard.blocked && volume_24h_usd > volume_min_usd;

    let reg = classify_regime_smoothed(bars_15m, 0.0, 5);
    let zlema = calculate_zero_lag_trend(bars_15m, 21, 1.0, TrendDir::Neutral);
    let shield = evaluate_volatility_shield(bars_15m, 0.0);
    let chop = choppiness_last_yahudi(bars_15m, 14);
    let adx_res = adx_last_yahudi(bars_15m, 14);
    let adx = adx_res.map(|a| a.adx);
    let (zone_score_f, best_zone, _) = detect_usable_zones(bars_15m, zlema.direction);
    let sq = evaluate_squeeze_trigger(bars_15m);

    let confirm_trend = if bars_5m.len() >= 30 {
        Some(calculate_zero_lag_trend(bars_5m, 21, 1.0, TrendDir::Neutral))
    } else {
        None
    };

    let trend_score = zlema.trend_score.round().clamp(5.0, 98.0) as u32;
    let zone_score = zone_score_f.round().clamp(0.0, 98.0) as u32;
    let trigger_score = sq.trigger_score.round().clamp(20.0, 98.0) as u32;
    let confirm_score = confirm_trend
        .as_ref()
        .map(|c| c.trend_score.round().clamp(20.0, 98.0) as u32);

    let composite_score = match confirm_score {
        Some(cs) => {
            (trend_score as f64 * 0.30
                + zone_score as f64 * 0.30
                + trigger_score as f64 * 0.25
                + cs as f64 * 0.15)
                .round() as u32
        }
        None => {
            (trend_score as f64 * 0.35 + zone_score as f64 * 0.35 + trigger_score as f64 * 0.30)
                .round() as u32
        }
    };

    let knn_feat = build_knn_features_yahudi(
        zone_score as f64,
        trigger_score as f64,
        trend_score as f64,
        adx,
        Some(volume_24h_usd),
    );
    let knn = query_knn_lorentzian(&knn_feat, knn_history, 5);

    let er = calculate_efficiency_ratio(bars_15m, 30);
    let er_status = if er.is_some() {
        "OK"
    } else {
        "INSUFFICIENT_SAMPLES"
    };

    let vp = build_volume_profile_48(bars_15m, 48);
    let last_price = bars_15m.last().map(|b| b.close).unwrap_or(0.0);
    let poc_dist = distance_from_poc_in_bins(&vp, last_price);

    let beta_res = calculate_beta_btc_with_status(
        if bars_5m.len() >= 51 {
            bars_5m
        } else {
            bars_15m
        },
        btc_bars_5m,
    );

    // Estimasi dinamika Open Interest / Partisipasi Volume 30m dari 7 bar 5m terakhir
    let oi_samples: Vec<f64> = if bars_5m.len() >= 7 {
        bars_5m[bars_5m.len() - 7..]
            .iter()
            .map(|b| b.quote_volume.max(b.volume * b.close))
            .collect()
    } else {
        Vec::new()
    };
    let oi_res = compute_oi_change_pct_from_samples(&oi_samples);

    let chop_gate = chop.map(|c| c < 58.3).unwrap_or(false) && adx.map(|a| a >= 20.0).unwrap_or(false);
    let regime_gate = reg.regime == apex_indicators::MarketRegime::Trending;
    let trend_gate = zlema.direction != TrendDir::Neutral;
    let score_gate = composite_score >= 60;
    let volatility_gate = !shield.block_new_signals;

    let (s01_ok, s01_q, s01_reasons) = match s01_matched {
        Some((q, r)) => (q > 0, q, r),
        None => (false, 0, vec!["NO_LIQUIDITY_SWEEP_RECLAIM".into()]),
    };
    let (s02_ok, s02_q, s02_reasons) = match s02_matched {
        Some((q, r)) => (q > 0, q, r),
        None => (false, 0, vec!["LIQUIDITY_TREND_MISMATCH".into()]),
    };
    let plan_gate = s01_ok || s02_ok;

    let mut elig_reasons = Vec::new();
    if guard.blocked {
        elig_reasons.push(guard.reason.unwrap_or_else(|| "BLACKLISTED".into()));
    }
    if !volume_gate {
        elig_reasons.push("VOLUME_BELOW_MIN".into());
    }
    if let Some(c) = chop {
        if c >= 58.3 {
            elig_reasons.push("CHOP_HIGH".into());
        }
    }
    if let Some(a) = adx {
        if a < 20.0 {
            elig_reasons.push("ADX_LOW".into());
        }
    }

    let (strategy_id, strategy_name, entry_type) = if s02_ok && s02_q >= s01_q {
        (
            Some("S-02".into()),
            Some("Liquidity Sniper".into()),
            Some("MARKET_PULLBACK".into()),
        )
    } else if s01_ok {
        (
            Some("S-01".into()),
            Some("Liquidity Sweep Reclaim".into()),
            Some("LIMIT_RETEST".into()),
        )
    } else {
        (None, None, None)
    };

    let all_gates = volume_gate
        && regime_gate
        && trend_gate
        && score_gate
        && plan_gate
        && chop_gate
        && volatility_gate;

    let status = if all_gates {
        "READY"
    } else if volume_gate && volatility_gate && composite_score >= 48 {
        "BORDERLINE_REVIEW"
    } else {
        "WAIT"
    };

    let reject_reason = if all_gates {
        None
    } else if !volume_gate {
        Some("VOLUME_BELOW_MIN".into())
    } else if !volatility_gate {
        Some("VOLATILITY_SHIELD_ACTIVE".into())
    } else if !trend_gate {
        Some("BIAS_SIDEWAYS".into())
    } else if !regime_gate || !chop_gate {
        Some("CHOP_OR_REGIME_RANGING".into())
    } else if !plan_gate {
        Some("NO_SCALP_STRATEGY_MATCH".into())
    } else {
        Some("SCORE_BELOW_GATE".into())
    };

    let structure_atr_pct = if last_price > 0.0 && shield.current_atr > 0.0 {
        Some(((shield.current_atr / last_price) * 10_000.0 * 100.0).round() / 10_000.0)
    } else {
        None
    };

    ScanCandidateRow {
        symbol: symbol.to_uppercase(),
        score: composite_score,
        mode: "SCALP",
        regime: reg.regime.as_str(),
        zone_score,
        trigger_score,
        trend_score,
        knn_prob: knn.knn_prob,
        knn_n: knn.knn_n,
        knn_label: knn.label,
        gates_pass: GatesPass {
            volume_gate,
            regime_gate,
            trend_gate,
            score_gate,
            plan_gate,
            chop_gate,
            volatility_gate,
        },
        status,
        reject_reason,
        chop,
        chop_adx: adx,
        structure_atr_pct,
        er,
        er_status,
        poc_distance_bins: poc_dist,
        oi_change_pct: oi_res.value,
        oi_status: oi_res.status,
        oi_reason: oi_res.reason,
        beta_btc: beta_res.value,
        beta_status: beta_res.status,
        beta_reason: beta_res.reason,
        bias_chop: chop,
        bias_adx: adx,
        bias_trend: zlema.direction.as_str(),
        elig_reasons,
        strategy_id,
        strategy_name,
        entry_type,
        plan_candidates: vec![
            PlanCandidateRow {
                strategy_id: "S-01".into(),
                matched: s01_ok,
                quality: s01_q,
                reasons: s01_reasons,
            },
            PlanCandidateRow {
                strategy_id: "S-02".into(),
                matched: s02_ok,
                quality: s02_q,
                reasons: s02_reasons,
            },
        ],
        zone_type: best_zone.as_ref().map(|z| z.zone_type.as_str()),
        zone_top: best_zone.as_ref().map(|z| z.top),
        zone_bottom: best_zone.as_ref().map(|z| z.bottom),
        tf_summary: "SCALP: 15m (trigger/konfirmasi/struktur/bias, bar tutup) + makro 1h",
        mtf_analyzed: true,
        mtf_analyzed_at: now_ms,
    }
}

// ─────────────────────── 3. Liquidation Map Hub ±15% (liquidationMapHub.ts) ───────────────────────

pub const LIQ_RANGE_PCT: f64 = 15.0;
pub const LIQ_BINS: usize = 30;

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct LiquidationBin {
    pub price: f64,
    pub long_usd: f64,
    pub short_usd: f64,
    pub dist_pct: f64,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct LiquidationMapSnapshot {
    pub symbol: String,
    pub source: &'static str,
    pub mark_price: f64,
    pub updated_at: i64,
    pub stale: bool,
    pub top_long_magnet_price: Option<f64>,
    pub top_long_magnet_usd: f64,
    pub top_short_magnet_price: Option<f64>,
    pub top_short_magnet_usd: f64,
    pub bins: Vec<LiquidationBin>,
}

/// Resample bin likuidasi ke grid seragam 30-bin `±15%` di sekitar `mark_price` (`liquidationMapHub.ts:resampleBins`).
pub fn resample_liquidation_bins(raw: &[LiquidationBin], mark_price: f64) -> Vec<LiquidationBin> {
    if !(mark_price > 0.0) || !mark_price.is_finite() {
        return Vec::new();
    }
    let lo = mark_price * (1.0 - LIQ_RANGE_PCT / 100.0);
    let hi = mark_price * (1.0 + LIQ_RANGE_PCT / 100.0);
    let bin_size = (hi - lo) / LIQ_BINS as f64;
    let mut acc = vec![(0.0_f64, 0.0_f64); LIQ_BINS];
    for b in raw {
        if b.price >= lo && b.price <= hi {
            let idx = (((b.price - lo) / bin_size).floor() as usize).min(LIQ_BINS - 1);
            acc[idx].0 += b.long_usd.max(0.0);
            acc[idx].1 += b.short_usd.max(0.0);
        }
    }
    (0..LIQ_BINS)
        .map(|i| {
            let price = ((lo + bin_size * (i as f64 + 0.5)) * 1e6).round() / 1e6;
            let dist_pct = (((price - mark_price) / mark_price) * 100.0 * 100.0).round() / 100.0;
            LiquidationBin {
                price,
                long_usd: (acc[i].0 * 100.0).round() / 100.0,
                short_usd: (acc[i].1 * 100.0).round() / 100.0,
                dist_pct,
            }
        })
        .collect()
}

/// Bangun peta likuidasi `±15%` (30 bin) berbasis distribusi posisi leverage (`10x`, `20x`, `25x`, `50x`, `100x`)
/// dari volume bar historis di sekitar harga mark saat ini.
pub fn build_liquidation_map_snapshot(
    symbol: &str,
    bars: &[Bar],
    mark_price: f64,
    now_ms: i64,
) -> LiquidationMapSnapshot {
    let mut raw = Vec::new();
    let leverages = [
        (10.0, 0.095, 0.30),
        (20.0, 0.047, 0.30),
        (25.0, 0.038, 0.20),
        (50.0, 0.019, 0.12),
        (100.0, 0.009, 0.08),
    ];
    for b in bars.iter().rev().take(60) {
        let notional = if b.quote_volume > 0.0 {
            b.quote_volume
        } else {
            b.volume * b.close
        };
        if !(notional > 0.0) {
            continue;
        }
        let buy_share = if b.volume > 0.0 && b.taker_buy_base > 0.0 {
            (b.taker_buy_base / b.volume).clamp(0.1, 0.9)
        } else if b.close >= b.open {
            0.56
        } else {
            0.44
        };
        let sell_share = 1.0 - buy_share;
        for &(_lev, liq_offset, weight) in &leverages {
            let long_liq_px = b.close * (1.0 - liq_offset);
            let short_liq_px = b.close * (1.0 + liq_offset);
            if long_liq_px < mark_price {
                raw.push(LiquidationBin {
                    price: long_liq_px,
                    long_usd: notional * buy_share * weight * 0.08,
                    short_usd: 0.0,
                    dist_pct: ((long_liq_px - mark_price) / mark_price) * 100.0,
                });
            }
            if short_liq_px > mark_price {
                raw.push(LiquidationBin {
                    price: short_liq_px,
                    long_usd: 0.0,
                    short_usd: notional * sell_share * weight * 0.08,
                    dist_pct: ((short_liq_px - mark_price) / mark_price) * 100.0,
                });
            }
        }
    }
    let bins = resample_liquidation_bins(&raw, mark_price);
    let mut top_long_px = None;
    let mut top_long_usd = 0.0;
    let mut top_short_px = None;
    let mut top_short_usd = 0.0;
    for b in &bins {
        if b.long_usd > top_long_usd {
            top_long_usd = b.long_usd;
            top_long_px = Some(b.price);
        }
        if b.short_usd > top_short_usd {
            top_short_usd = b.short_usd;
            top_short_px = Some(b.price);
        }
    }
    LiquidationMapSnapshot {
        symbol: symbol.to_uppercase(),
        source: "perp_leverage_cluster_model",
        mark_price,
        updated_at: now_ms,
        stale: bins.is_empty(),
        top_long_magnet_price: top_long_px,
        top_long_magnet_usd: top_long_usd,
        top_short_magnet_price: top_short_px,
        top_short_magnet_usd: top_short_usd,
        bins,
    }
}

// ─────────────────────── 4. Multi-Window Orderflow Delta Hub (deltaEngine.ts) ───────────────────────

#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct DeltaWindowStats {
    pub window: &'static str,
    pub buy_notional: f64,
    pub sell_notional: f64,
    pub buy_ratio: f64,
    pub total_notional: f64,
    pub trade_count: u64,
    pub delta_notional: f64,
    pub cvd_session: f64,
    pub rolling_delta: f64,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct MultiWindowDeltaSnapshot {
    pub symbol: String,
    pub w5m: DeltaWindowStats,
    pub w15m: DeltaWindowStats,
    pub w1h: DeltaWindowStats,
    pub w4h: DeltaWindowStats,
}

fn calc_window_delta(window: &'static str, bars: &[Bar], cvd_session: f64) -> DeltaWindowStats {
    let mut buy_notional = 0.0;
    let mut sell_notional = 0.0;
    let mut trade_count = 0u64;
    for b in bars {
        let quote = if b.quote_volume > 0.0 {
            b.quote_volume
        } else {
            b.volume * b.close
        };
        let buy_ratio = if b.volume > 0.0 && b.taker_buy_base > 0.0 {
            (b.taker_buy_base / b.volume).clamp(0.0, 1.0)
        } else if b.close >= b.open {
            0.55
        } else {
            0.45
        };
        buy_notional += quote * buy_ratio;
        sell_notional += quote * (1.0 - buy_ratio);
        trade_count += b.trades;
    }
    let total = buy_notional + sell_notional;
    let ratio = if total > 0.0 {
        buy_notional / total
    } else {
        0.5
    };
    let delta = buy_notional - sell_notional;
    DeltaWindowStats {
        window,
        buy_notional: (buy_notional * 100.0).round() / 100.0,
        sell_notional: (sell_notional * 100.0).round() / 100.0,
        buy_ratio: (ratio * 10_000.0).round() / 10_000.0,
        total_notional: (total * 100.0).round() / 100.0,
        trade_count,
        delta_notional: (delta * 100.0).round() / 100.0,
        cvd_session: (cvd_session * 100.0).round() / 100.0,
        rolling_delta: (delta * 100.0).round() / 100.0,
    }
}

/// Hitung delta multi-window (`5m`, `15m`, `1h`, `4h`) dari bar `5m` (`deltaEngine.ts`).
pub fn compute_multi_window_delta(symbol: &str, bars_5m: &[Bar]) -> MultiWindowDeltaSnapshot {
    let session = calc_window_delta("session", bars_5m, 0.0).delta_notional;
    let slice_last = |n: usize| -> &[Bar] {
        if bars_5m.len() > n {
            &bars_5m[bars_5m.len() - n..]
        } else {
            bars_5m
        }
    };
    MultiWindowDeltaSnapshot {
        symbol: symbol.to_uppercase(),
        w5m: calc_window_delta("5m", slice_last(1), session),
        w15m: calc_window_delta("15m", slice_last(3), session),
        w1h: calc_window_delta("1h", slice_last(12), session),
        w4h: calc_window_delta("4h", slice_last(48), session),
    }
}

// ─────────────────────── 5. 5-Panel Microstructure AI Briefs (panelInsights.ts) ───────────────────────

#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct PanelAiBrief {
    pub panel: &'static str,
    pub status: &'static str,
    pub text: String,
    pub evidence_freshness: &'static str,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct PanelInsightsSnapshot {
    pub symbol: String,
    pub generated_at_ms: i64,
    pub fresh: bool,
    pub evidence_freshness: &'static str,
    pub orderflow: PanelAiBrief,
    pub walls: PanelAiBrief,
    pub liqmap: PanelAiBrief,
    pub largetrades_vp: PanelAiBrief,
    pub tierflow: PanelAiBrief,
    pub combined: PanelAiBrief,
}

/// Bangun analisis terstruktur 5-panel + `combined` sesuai kontrak `panelInsights.ts` & `panelBrief.ts`.
pub fn build_panel_insights_snapshot(
    symbol: &str,
    delta: &MultiWindowDeltaSnapshot,
    walls: &WallLifelinesSummary,
    liqmap: &LiquidationMapSnapshot,
    vp: &VolumeProfile48,
    tiers: &TierFlowSummary,
    now_ms: i64,
) -> PanelInsightsSnapshot {
    let of_text = format!(
        "Ringkasan deterministik Orderflow {}: Delta 5m ${:.0} (buy ratio {:.1}%), 15m ${:.0}, 1h ${:.0}, CVD sesi ${:.0}. {}",
        symbol,
        delta.w5m.delta_notional,
        delta.w5m.buy_ratio * 100.0,
        delta.w15m.delta_notional,
        delta.w1h.delta_notional,
        delta.w1h.cvd_session,
        if delta.w15m.delta_notional > 0.0 && delta.w1h.delta_notional > 0.0 {
            "Arus agresor 15m dan 1h selaras positif (akumulasi)."
        } else if delta.w15m.delta_notional < 0.0 && delta.w1h.delta_notional < 0.0 {
            "Arus agresor 15m dan 1h selaras negatif (distribusi)."
        } else {
            "Divergensi horizon pendek (15m) vs menengah (1h)."
        }
    );

    let walls_text = format!(
        "Wall Lifelines {}: bid aktif {}, ask aktif {}, pulled {}x, eaten {}x (spoof score {:.1}%).",
        symbol,
        walls.active_bid_walls,
        walls.active_ask_walls,
        walls.pulled_count,
        walls.eaten_count,
        walls.spoof_score * 100.0,
    );

    let liq_text = format!(
        "Liquidation Map ±15% {} (mark {:.4}): Magnet short atas {:?} (${:.0}) vs magnet long bawah {:?} (${:.0}).",
        symbol,
        liqmap.mark_price,
        liqmap.top_short_magnet_price,
        liqmap.top_short_magnet_usd,
        liqmap.top_long_magnet_price,
        liqmap.top_long_magnet_usd,
    );

    let vp_text = format!(
        "Volume Profile 48-bin {}: POC {:?} (buy share {:?}%), VAH {:?}, VAL {:?}, posisi harga terhadap Value Area = {}.",
        symbol,
        vp.poc,
        vp.poc_buy_share_pct,
        vp.vah,
        vp.val,
        vp.price_vs_value_area,
    );

    let tf_text = format!(
        "Tier Flow {}: Net Institusional (MEGA+WHALE+LARGE) ${:.0} vs Retail (MEDIUM+SMALL) ${:.0} (bias = {}, alert = {:?}).",
        symbol,
        tiers.whale_net_usd,
        tiers.retail_net_usd,
        tiers.whale_retail_bias,
        tiers.absorption_alert
    );

    let combined_text = format!(
        "Sintesis 5 Panel {}: [1] Orderflow 15m ${:.0} / 1h ${:.0}; [2] Walls bid={} ask={} (pulled {}x vs eaten {}x); [3] LiqMap atas ${:.0} vs bawah ${:.0}; [4] VP {} (POC {:?}); [5] TierFlow Whale net ${:.0} ({}).",
        symbol,
        delta.w15m.delta_notional,
        delta.w1h.delta_notional,
        walls.active_bid_walls,
        walls.active_ask_walls,
        walls.pulled_count,
        walls.eaten_count,
        liqmap.top_short_magnet_usd,
        liqmap.top_long_magnet_usd,
        vp.price_vs_value_area,
        vp.poc,
        tiers.whale_net_usd,
        tiers.whale_retail_bias
    );

    PanelInsightsSnapshot {
        symbol: symbol.to_uppercase(),
        generated_at_ms: now_ms,
        fresh: true,
        evidence_freshness: "fresh",
        orderflow: PanelAiBrief {
            panel: "orderflow",
            status: "FALLBACK",
            text: of_text,
            evidence_freshness: "fresh",
        },
        walls: PanelAiBrief {
            panel: "walls",
            status: "FALLBACK",
            text: walls_text,
            evidence_freshness: "fresh",
        },
        liqmap: PanelAiBrief {
            panel: "liqmap",
            status: "FALLBACK",
            text: liq_text,
            evidence_freshness: "fresh",
        },
        largetrades_vp: PanelAiBrief {
            panel: "largetrades_vp",
            status: "FALLBACK",
            text: vp_text,
            evidence_freshness: "fresh",
        },
        tierflow: PanelAiBrief {
            panel: "tierflow",
            status: "FALLBACK",
            text: tf_text,
            evidence_freshness: "fresh",
        },
        combined: PanelAiBrief {
            panel: "combined",
            status: "FALLBACK",
            text: combined_text,
            evidence_freshness: "fresh",
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_bars(n: usize) -> Vec<Bar> {
        (0..n)
            .map(|i| {
                let p = 100.0 + (i as f64) * 0.4 + ((i % 5) as f64) * 0.1;
                Bar {
                    open_time: (i as i64) * 300_000,
                    open: p - 0.1,
                    high: p + 0.5,
                    low: p - 0.4,
                    close: p + 0.1,
                    volume: 1000.0 + (i as f64) * 10.0,
                    quote_volume: (1000.0 + (i as f64) * 10.0) * p,
                    trades: 120,
                    taker_buy_base: (1000.0 + (i as f64) * 10.0) * 0.58,
                }
            })
            .collect()
    }

    #[test]
    fn asset_class_guard_blocks_tradfi_and_fan_tokens() {
        assert!(check_asset_class("BZUSDT").blocked);
        assert_eq!(check_asset_class("BZUSDT").asset_class, "COMMODITY");
        assert!(check_asset_class("NVDAUSDT").blocked);
        assert_eq!(check_asset_class("NVDAUSDT").asset_class, "EQUITY");
        assert!(check_asset_class("SPYUSDT").blocked);
        assert_eq!(check_asset_class("SPYUSDT").asset_class, "INDEX");
        assert!(check_asset_class("PSGUSDT").blocked);
        assert_eq!(
            check_asset_class("PSGUSDT").reason.as_deref(),
            Some("fan-token:operator")
        );
        assert!(!check_asset_class("BTCUSDT").blocked);
        assert!(!check_asset_class("1000PEPEUSDT").blocked);
    }

    #[test]
    fn scan_candidate_and_liqmap_and_delta_work() {
        let b15 = sample_bars(90);
        let b5 = sample_bars(90);
        let row = evaluate_scan_candidate(
            "SOLUSDT",
            &b15,
            &b5,
            Some(&b5),
            85_000_000.0,
            10_000_000.0,
            &[],
            Some((82, vec!["SWEEP_RECLAIM_OK".into()])),
            None,
            1_700_000_000_000,
        );
        assert_eq!(row.symbol, "SOLUSDT");
        assert_eq!(row.mode, "SCALP");
        assert_eq!(row.plan_candidates.len(), 2);
        assert!(row.gates_pass.volume_gate);
        assert!(row.gates_pass.plan_gate);

        let liq = build_liquidation_map_snapshot("SOLUSDT", &b5, 135.0, 1_700_000_000_000);
        assert_eq!(liq.bins.len(), 30);
        assert!(liq.top_long_magnet_usd > 0.0);
        assert!(liq.top_short_magnet_usd > 0.0);

        let delta = compute_multi_window_delta("SOLUSDT", &b5);
        assert!(delta.w15m.total_notional > 0.0);
        assert!(delta.w1h.buy_ratio > 0.5);
    }
}
