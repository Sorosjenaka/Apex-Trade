# Architecture

This document describes each subsystem in the **Apex Trade** Rust workspace: its inputs, outputs, and failure semantics.

## 1. Market Data & Rate-Limit Layer (`apex-data`, `apex-oms`)

- **Public REST & WebSocket (`rustls`):** Fetches exchange metadata, 24h tickers, funding rates, and closed klines across `5m`, `15m`, `1h`, and `1d` intervals without OpenSSL dependencies.
- **Binance IP Rate-Limit Guard (`BinanceRateGuard`):** Tracks per-minute request weight (`X-MBX-USED-WEIGHT-1M`), automatically throttling non-essential scans above high-water thresholds and enforcing exponential cooldowns on HTTP `429` or `418` responses.

## 2. Universe Curation & 7-Gate Scanner (`apex-screener`, `apex-market-reader`)

- **Asset Class Guard:** Applies a deny-by-default filter to exclude tokenized equities, commodities, FX pairs, indices, and fan tokens so only pure crypto USDT-M perpetuals enter evaluation.
- **6-State Macro Matrix (`BTC.D × BTC`):** Classifies macro dominance shifts alongside global volatility regimes (`CHOP_NO_TRADE`, `SHOCK_HALT`) to gate directional altcoin exposure.
- **7-Gate Scalping Scanner:** Scores each symbol across volume, regime, ZLEMA trend, composite setup quality, plan availability, Choppiness Index, and ATR volatility bounds.
- **Microstructure Synthesis:** Builds 48-bin Volume Profiles (`POC`/`VAH`/`VAL`), structural liquidation magnet maps, multi-window taker buy/sell delta (`5m`/`15m`/`1h`), and panel insights.

## 3. Closed-Bar Scalping Signal Engine (`apex-strategies`, `apex-indicators`)

- **`S1_TFP` (Trend-Following Pullback):** Captures trend continuation pullbacks with strict structural stop placement and 2.0R targets.
- **`S01_SWEEP` (Liquidity Sweep Reclaim):** Detects sweeps of prior range/session liquidity highs or lows that reclaim inside structure with aligned CVD orderflow.
- **`S02_SNIPER` (Liquidity Sniper):** Targets high-density opposing liquidity pools when `5m` momentum, ZLEMA bias, and `15m` volatility shield align.
- **Causal `k-NN` Analog Engine:** Queries historical closed setups strictly prior to the signal timestamp, failing closed unless sufficient labeled neighbors exist.

## 4. Pre-Entry Decision Council & Risk Engine (`apex-core`, `apex-oms`)

- **Deterministic `$1` Risk Sizing:** Computes position quantity from a fixed `$1.00` risk budget and exchange `stepSize`/`tickSize` constraints via `rust_decimal`, rejecting trades whose rounded risk falls outside the permitted tolerance or whose stop distance lies outside the `2%–4%` band.
- **3-Agent Pre-Entry Council (`convene_deterministic_council`):**
  - **Vega (`macro-scout`):** Enforces macro regime compatibility.
  - **Arka (`setup-judge`):** Audits composite score, `k-NN` probability, and CVD alignment.
  - **Wardha (`risk-officer`):** Validates `SL / ATR` resilience and selects the optimal structural SL/TP menu variant.
  - Any single `NO_GO` vote with confidence `> 70` on the deterministic council triggers an immediate veto (`enforced = true`); execution requires at least 2 `GO` votes and average confidence `>= 60`. Fallback defaults are recorded in `inputs_defaulted`.
- **Directional Battle Arena (`enforced = false`):** Computes `LONG` vs `SHORT` thesis scores as **advisory telemetry only** (logged to audit and dashboard, does not veto orders).

## 5. Order Management System & Supervision (`apex-oms`)

- **Idempotent Execution:** Persists `ManagedTrade` state before dispatching `LIMIT GTX` (post-only) or `MARKET` entry orders.
- **Protective Orders & Anti-Orphan Reconciler:** Attaches exchange-side conditional `STOP_MARKET` / `TAKE_PROFIT_MARKET` (`MARK_PRICE`, `closePosition=true`) orders and periodically reconciles local state against exchange positions.
- **Trend-Flip Confirmation Guard:** Monitors consecutive opposite trend bars to exit deteriorating scalps before full stop-loss excursion.

## 6. Quantitative Validation & Learning Loop (`apex-backtest`, `apex-oms`)

- **5-Whys Post-Mortem Forensics:** Generates structured root-cause chains on closed losses and updates **Wilson 95% Lower Bound** win-rate estimates and bounded (`<= 10%`) parameter tuning proposals.
- **Combinatorial Purged Cross-Validation (CPCV):** Evaluates strategy robustness across combinatorial train/test splits with purge and embargo windows to prevent leakage.
