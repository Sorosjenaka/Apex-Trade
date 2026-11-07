# Workflow

This document outlines the operational cycle of **Apex Trade** from market ingestion through post-trade forensic learning.

## 1. Closed-Bar Scan Cycle

1. **Clock & Rate-Limit Check:** At each cycle tick, `BinanceRateGuard` verifies current minute weight and ensures no temporary HTTP `429`/`418` ban is active.
2. **Market Snapshot:** Closed klines (`5m`, `15m`, `1h`, `1d`) and 24h tickers are refreshed for BTC, ETH, dominance proxies, and tradeable USDT-M perpetuals.
3. **Global Regime & Macro Gate:** `MarketReader` and the 6-state `BTC.D × BTC` matrix evaluate whether `LONG`, `SHORT`, or neither (`CHOP_NO_TRADE` / `SHOCK_HALT`) is permitted.
4. **Symbol Screening & Microstructure:** Eligible crypto contracts are evaluated through the 7-gate scanner, producing 48-bin Volume Profiles, liquidation maps, and CVD orderflow windows.

## 2. Candidate Evaluation & Execution

1. **Signal Generation:** Closed-bar scalping strategies (`S1_TFP`, `S01_SWEEP`, `S02_SNIPER`) emit directional candidates with structural invalidation stops and target R-multiples.
2. **Risk Gate:** Exact-decimal `$1.00` position sizing rounds quantity to exchange `stepSize` and verifies that actual risk, `2%–4%` stop distance, max open positions, and daily stop rules pass.
3. **3-Agent Council & Battle Arena:** `Vega`, `Arka`, and `Wardha` vote on the setup; only unanimous/majority `EXECUTE` verdicts with zero high-confidence vetoes proceed.
4. **Order Placement:** `OrderManager` writes state to `oms_state.json` and dispatches the order (`MockExchange` in `PAPER`, or `LIMIT GTX` in armed `LIVE`).

## 3. Supervision & Post-Mortem Learning

1. **Active Protection:** Conditional `STOP_MARKET` and `TAKE_PROFIT_MARKET` orders protect every filled position; `TrendFlipConfirmationGuard` exits early if consecutive closed bars confirm a reversal.
2. **Forensic Feedback Loop:** Closed trades trigger `generate_five_whys_forensics`, update Wilson 95% lower-bound win-rate statistics, and generate bounded tuning proposals validated offline via `apex backtest` and CPCV.
