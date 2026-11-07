# Risk and Execution

## 1. Fixed `$1.00` Structural Sizing

Position sizing is governed by deterministic fixed-dollar risk rather than arbitrary fixed notional:

- Given entry price $E$ and invalidation stop $S$, raw quantity is $Q = \frac{\$1.00}{|E - S|}$, floored to the symbol's `stepSize` using exact `rust_decimal` arithmetic.
- Stop distance $|E - S| / E$ must lie strictly within `[2.0%, 4.0%]`.
- After rounding to exchange `tickSize` and `stepSize`, effective dollar risk must remain within tolerance (`<= $1.00`) and satisfy exchange `minNotional`.

## 2. Portfolio & Session Safeguards

- **Concurrent Exposure Cap:** Open positions plus pending entry orders cannot exceed configured `max_positions`.
- **Daily Loss & SL Circuit Breaker:** Reaching the daily realized loss cap or maximum daily stop-loss count halts new entries until `00:00 UTC`.
- **Symbol Cooldown:** A stop-loss enforces a mandatory cooldown before re-entering the same symbol.
- **Kill Switch (`state/KILL`):** Presence of the kill file immediately blocks new entries and optionally flattens open exposure.

## 3. Triple-Locked Live Execution Gate

No order can reach production Binance USDT-M unless three independent conditions hold simultaneously:

1. `arm = true` in `config/engine.toml` (no CLI flag exists to toggle `arm`).
2. `--mode live` passed explicitly to `apex run`.
3. `APEX_LIVE_CONFIRM=I_UNDERSTAND_LIVE_ORDERS` set in the process environment.

By default, `mode = "paper"` and `arm = false`.

## 4. Order Lifecycle & Anti-Orphan Protection

- **Write-Ahead State:** Every `ManagedTrade` is written to `oms_state.json` prior to network submission with a deterministic `apx...` `newClientOrderId`.
- **Maker Preference:** Entries default to `LIMIT GTX` (post-only) with bounded TTL expiration.
- **Exchange-Side Protection:** Upon fill confirmation, conditional `STOP_MARKET` and `TAKE_PROFIT_MARKET` orders (`workingType = MARK_PRICE`, `closePosition = true`) are placed immediately; if stop placement fails after retries, the engine flattens the position and activates the kill switch.
- **Periodic Reconciliation:** Boot and periodic reconcilers detect any orphan exchange position and attach emergency protection according to `orphan_policy`.
