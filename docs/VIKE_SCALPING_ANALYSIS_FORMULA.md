# Vike Scalping Analysis Formula — Read Only

This is the auditable v0 analysis contract for the dark market reader. It is not a live trading rule and has not been validated as profitable.

## Venue separation

- Binance: execution/reference venue — price, candles, depth, aggTrade, funding, OI.
- Vike: Hyperliquid cohort and liquidation context — smart/whale bias, liquidation map, large fills, order clusters, cohort history.
- Never use a Vike price level as a Binance order-book level without a cross-venue basis label.

## Component normalization

For each component produce a signed score in `[-1,+1]`, a raw payload, `source`, `snapshot_ts`, `age_ms`, `stale`, and `coverage`.

```text
B = cohort bias
L = liquidation asymmetry
F = Vike large fills + Binance taker/CVD confirmation
O = Vike order clusters + Binance depth/wall persistence
C = funding/OI/crowding context
M = Binance candle/microstructure direction

ContextScore = 100*(0.25*B + 0.20*L + 0.20*F + 0.15*O + 0.10*C + 0.10*M)
```

## Confidence

```text
freshness = weighted freshness of all components
coverage = available component count / required component count
venue_agreement = agreement between Binance microstructure and Vike context
conflict_penalty = count of materially opposing components

confidence = clamp(100*(0.40*freshness + 0.25*coverage + 0.25*venue_agreement - 0.10*conflict_penalty),0,100)
```

The exact freshness decay and component thresholds are configuration hypotheses and must be locked before a future point-in-time backtest.

## Display bands

```text
score >= +45: BULLISH CONTEXT
+20..+44:    BULLISH LEAN
-19..+19:    MIXED / NO EDGE
-44..-20:    BEARISH LEAN
score <= -45: BEARISH CONTEXT
```

## Scalping hierarchy

```text
15m: context and crowding regime
5m: setup/structure and liquidity interaction
1m: tape/aggTrade/depth confirmation
```

## Output contract

```json
{
  "symbol": "ETHUSDT",
  "venue": "BINANCE_FUTURES",
  "context_score": -58.0,
  "confidence": 74.0,
  "state": "BEARISH_LEAN",
  "read": "SHORT_BIAS_WATCH",
  "components": {
    "cohort_bias": {},
    "liquidation_asymmetry": {},
    "large_flow": {},
    "order_clusters": {},
    "funding_oi": {},
    "binance_microstructure": {}
  },
  "conflicts": [],
  "invalidation": [],
  "source_age_ms": {},
  "stale": false,
  "execution": "DISABLED_READ_ONLY"
}
```

## Hard rules

1. Never output a directional read without listing the component evidence.
2. Never hide stale data; downgrade confidence and show the age.
3. A conflict state is `MIXED / NO EDGE`, not a forced long/short.
4. No component alone opens a position.
5. No API key, order, signing, or wallet mutation is part of this contract.
