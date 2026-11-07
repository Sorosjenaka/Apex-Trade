# System Overview

**Apex Trade** is an automated crypto perpetual futures scalping system for **Binance USDT-M**, written in **Rust**. It evaluates a curated universe of liquid perpetual contracts on closed-bar boundaries, computes multi-timeframe microstructure and orderflow signals, enforces exact-decimal `$1.00` risk sizing, convenes a deterministic 3-agent pre-entry council plus directional thesis arena, and manages open positions with exchange-side protective orders and trend-flip guards.

The architecture is strictly dedicated to **Scalping (`SCALP`)** — no multi-day swing (`RSW`) strategies are active.

## Design Principles

- **Strict closed-bar causality.** Indicators, regime classifiers, and signal engines consume only completed bars (`5m`, `15m`, `1h`), preventing look-ahead leakage between live execution and backtesting.
- **Deterministic gates first, fail-closed everywhere.** Stale market data, non-crypto contracts, `CHOP_NO_TRADE` regimes, out-of-band stop distances (`< 2%` or `> 4%`), or a single high-confidence council veto immediately reject the candidate.
- **Triple-locked live execution.** Live exchange orders require `arm = true` in `config/engine.toml`, `--mode live` on the CLI, and `APEX_LIVE_CONFIRM=I_UNDERSTAND_LIVE_ORDERS` in the process environment.
- **Write-ahead OMS persistence.** Position intent and deterministic `clientOrderId` values are persisted atomically before network transmission.
- **Full auditability.** Every cycle, candidate trace, council vote, skip reason, fill, and forensic post-mortem is recorded to append-only JSONL logs.

## Operating Modes

| Mode | Behavior |
|---|---|
| `PAPER` *(default)* | Full pipeline runs against live public Binance market data using `MockExchange`. Zero orders are sent to Binance. |
| `TESTNET` | Connects to Binance Futures Demo/Testnet (`demo-fapi.binance.com`); requires `arm = true` and `GO` strategy status. |
| `LIVE` | Connects to production Binance USDT-M (`fapi.binance.com`); gated by the triple-lock (`arm = true` + `--mode live` + `APEX_LIVE_CONFIRM`). |

## Main Rust Workspace Crates

| Crate / Path | Role |
|---|---|
| `crates/core` | Domain types, `rust_decimal` `$1` risk sizing, exchange filter rounding, risk gate, and JSONL audit logger |
| `crates/indicators` | Closed-bar EMA, ZLEMA, ATR, ADX, Choppiness, 48-bin Volume Profile (`POC`/`VAH`/`VAL`), Fibonacci retracement, and MRV stretch gate |
| `crates/data` | `rustls`-backed Binance REST & WebSocket clients, kline CSV storage, and public market fetchers |
| `crates/market_reader` | Global market regime (`RISK_ON_ALTS`, `BTC_LED`, `RISK_OFF`, `CHOP_NO_TRADE`, `SHOCK_HALT`) |
| `crates/screener` | Deny-by-default crypto `AssetClassGuard`, `VolumeGate`, 7-gate candidate scanner, liquidation map, and multi-window CVD delta |
| `crates/strategies` | Closed-bar scalping strategies (`S1_TFP`, `S01_SWEEP`, `S02_SNIPER`) and causal `k-NN` analog probability lookup |
| `crates/oms` | Order Management System, `MockExchange`, 3-Agent Council (`Vega`, `Arka`, `Wardha`), Rate-Limit Guard, Trend-Flip Guard, and 5-Whys Forensics |
| `crates/backtest` | Event-driven zero-look-ahead simulator, fee stress testing, Monte Carlo drawdown, PSR/DSR, and CPCV validation |
| `apps/apex` | Async Tokio CLI binary orchestrating `run`, `screen`, `reader`, `fetch`, `backtest`, `validate`, `status`, and `kill` |
| `dashboard/` | Read-only Python HTTP + SSE dark web terminal (`127.0.0.1:8787`) |
