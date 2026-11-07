# Apex Trade — Public Architecture Documentation

Index for the public technical architecture and trading pipeline documentation of **Apex Trade** (Rust Scalping Engine for Binance USDT-M Futures).

This documentation explains how the Rust workspace is structured, how a scalping candidate flows through each deterministic and multi-agent gate, and how orders and open positions are supervised. It intentionally excludes private credentials and operator secrets.

## Documents

| Document | Scope |
|---|---|
| [SYSTEM_OVERVIEW.md](SYSTEM_OVERVIEW.md) | Purpose, safe-by-default operating modes, and main Rust crates |
| [ARCHITECTURE.md](ARCHITECTURE.md) | Crate responsibilities, market data layer, 3-agent council, OMS, and persistence |
| [WORKFLOW.md](WORKFLOW.md) | Closed-bar scan cycle, trade lifecycle, and continuous post-mortem learning |
| [TRADING_PIPELINE.md](TRADING_PIPELINE.md) | Stage-by-stage scalping gate pipeline and fail-closed rejection branches |
| [RISK_AND_EXECUTION.md](RISK_AND_EXECUTION.md) | Exact-decimal `$1` risk sizing, ARM triple-lock, GTX execution, and anti-orphan protection |

## Indonesian Guide & Dark-Mode C4 Diagrams

Full Indonesian documentation and dark-mode C4 / workflow diagrams are available in [`docs/id/`](../id/README.md) and [`docs/diagrams/`](../diagrams/).
