# Trading Pipeline

Every scalping candidate in **Apex Trade** traverses a strict sequential pipeline. Failure at any stage terminates evaluation for that candidate and writes an explicit reason to `audit.jsonl`.

| Stage | Gate | Rejection Condition |
|---|---|---|
| 1 | **Asset Class Guard** | Non-crypto contract (equity, commodity, index, FX, fan token) or manual blacklist |
| 2 | **Volume & Liquidity Gate** | 24h quote volume or orderbook depth below minimum threshold |
| 3 | **Global Regime & Macro Gate** | `CHOP_NO_TRADE`, `SHOCK_HALT`, or directional conflict with the 6-state `BTC.D × BTC` matrix |
| 4 | **Volatility Shield & MRV Stretch** | Extreme ATR spike, wide spread, or overextended `5m` mean-reversion stretch |
| 5 | **Closed-Bar Strategy Trigger** | Setup conditions for `S1_TFP`, `S01_SWEEP`, or `S02_SNIPER` not satisfied on the latest closed bar |
| 6 | **Deterministic `$1` Risk Gate** | Stop distance outside `2%–4%`, rounded `stepSize` risk outside `$0.80–$1.00`, max positions full, cooldown active, or daily stop hit |
| 7 | **3-Agent Pre-Entry Council** | Any `NO_GO` vote with confidence `> 70`, `< 2` `GO` votes, or average confidence `< 60` (`ARTATRADE_COUNCIL_REJECT`) |
| 8 | **Mode & ARM Gate** | Simulated locally in `PAPER`; blocked from reaching live exchange unless `arm = true` + `--mode live` + `APEX_LIVE_CONFIRM` |
| 9 | **OMS Entry & Protection** | Post-only `LIMIT GTX` rejected/expired or inability to attach exchange-side SL protection (triggers emergency flatten) |
