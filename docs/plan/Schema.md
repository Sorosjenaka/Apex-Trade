# Skema Data: Apex Scalper RS

> v1.0 · 5 Nov 2026 (WIB). SQLite (WAL) file `data/apex.sqlite`. Semua timestamp = **UTC epoch milidetik (INTEGER)**. Nilai uang/harga/qty disimpan sebagai **TEXT desimal** (`rust_decimal`) agar tidak ada pembulatan float.

## 1. Database

### 1.1 `symbols_filters` (FR-003)
| field | tipe | wajib? | keterangan |
|---|---|---|---|
| symbol | TEXT PK | ya | mis. `ETHUSDT` |
| status | TEXT | ya | `TRADING` dll. |
| tick_size | TEXT | ya | PRICE_FILTER.tickSize |
| step_size | TEXT | ya | LOT_SIZE.stepSize |
| min_qty | TEXT | ya | LOT_SIZE.minQty |
| min_notional | TEXT | ya | MIN_NOTIONAL.notional |
| max_leverage_b1 | INTEGER | ya | bracket-1 |
| mmr_b1 | TEXT | ya | maint margin ratio bracket-1 |
| onboard_ms | INTEGER | ya | onboardDate |
| updated_ms | INTEGER | ya | waktu fetch |

### 1.2 `screener_snapshots` (FR-008)
| field | tipe | wajib? | keterangan |
|---|---|---|---|
| id | INTEGER PK | ya | autoincrement |
| ts_ms | INTEGER | ya | close_time bar 1h |
| symbol | TEXT | ya | |
| state | TEXT | ya | `TREND_UP` \| `TREND_DOWN` \| `RANGING` \| `BLOCKED` |
| dir_votes | INTEGER | ya | −4..4 |
| adx, er, chop, srv_1h, srv_4h, atr_pct_1h, atr_ratio | REAL | ya | SCREENER §4 |
| bft_state, decycler_state, mtf_score | INTEGER | ya | −1/0/1; mtf −3..3 |
| qv24_musd, spread_bps_med, funding_pct, chg24_pct | REAL | ya | SCREENER §3 |
| reasons | TEXT (JSON array) | ya | kode alasan, mis. `["CHOP>=55"]` |

### 1.3 `signals` (FR-010)
| field | tipe | wajib? | keterangan |
|---|---|---|---|
| id | TEXT PK | ya | `sig-{strategy}-{symbol}-{bar_close_ms}` |
| ts_ms | INTEGER | ya | close_time bar trigger |
| strategy | TEXT | ya | `S1`..`S5` |
| symbol | TEXT | ya | |
| side | TEXT | ya | `LONG` \| `SHORT` |
| entry, sl, tp | TEXT | ya | harga, sudah dibulatkan tickSize |
| sl_pct | TEXT | ya | 4 desimal persen, mis. `2.3417` |
| screener_state | TEXT | ya | state saat sinyal |
| decision | TEXT | ya | `ORDERED` \| `SKIP_REGIME` \| `SKIP_SL_RANGE` \| `SKIP_SIZE` \| `SKIP_MAX_POSITIONS` \| `SKIP_COOLDOWN` \| `SKIP_DAILY_STOP` \| `SKIP_EQUITY_FLOOR` \| `SKIP_SPREAD` \| `NOT_ARMED` |
| sizing | TEXT (JSON) | tidak | `{qty, notional, margin, risk_planned}` bila dihitung |

### 1.4 `orders` (FR-017, FR-018)
| field | tipe | wajib? | keterangan |
|---|---|---|---|
| client_id | TEXT PK | ya | `newClientOrderId` / `clientAlgoId` |
| exchange_id | TEXT | tidak | orderId / algoId |
| signal_id | TEXT FK | ya | |
| kind | TEXT | ya | `ENTRY` \| `SL` \| `TP` \| `CLOSE` |
| type | TEXT | ya | `LIMIT` \| `MARKET` \| `STOP_MARKET` \| `TAKE_PROFIT_MARKET` |
| side | TEXT | ya | `BUY` \| `SELL` |
| price | TEXT | tidak | limit price |
| trigger_price | TEXT | tidak | SL/TP |
| qty | TEXT | tidak | null untuk closePosition |
| status | TEXT | ya | `NEW` \| `PARTIALLY_FILLED` \| `FILLED` \| `CANCELED` \| `EXPIRED` \| `REJECTED` \| `TRIGGERED` |
| created_ms, updated_ms | INTEGER | ya | |
| error | TEXT | tidak | kode Binance, mis. `-5022` |

### 1.5 `positions` (FR-019, FR-021)
| field | tipe | wajib? | keterangan |
|---|---|---|---|
| id | TEXT PK | ya | = signal_id |
| symbol, side | TEXT | ya | |
| qty | TEXT | ya | |
| entry_avg | TEXT | ya | |
| sl, tp | TEXT | ya | |
| risk_planned | TEXT | ya | USDT, 4 desimal |
| margin | TEXT | ya | notional/25, 4 desimal |
| state | TEXT | ya | `OPEN_UNPROTECTED` \| `PROTECTED` \| `CLOSING` \| `CLOSED` |
| opened_ms, protected_ms, closed_ms | INTEGER | sebagian | |

### 1.6 `trades` (FR-027)
| field | tipe | wajib? | keterangan |
|---|---|---|---|
| id | TEXT PK | ya | = position id |
| strategy, symbol, side | TEXT | ya | |
| entry_avg, exit_avg | TEXT | ya | |
| exit_reason | TEXT | ya | `SL` \| `TP` \| `MANUAL` \| `SL_MISSING_CLOSE` \| `LIQUIDATION` |
| pnl_gross, fees, funding, pnl_net | TEXT | ya | USDT, 4 desimal |
| r_multiple | TEXT | ya | pnl_net / risk_planned, 3 desimal |
| slippage_r | TEXT | ya | (fill SL/TP aktual − trigger) dalam R |
| opened_ms, closed_ms | INTEGER | ya | |

### 1.7 `daily_risk` (FR-015, FR-016, FR-024)
| field | tipe | wajib? | keterangan |
|---|---|---|---|
| day_utc | TEXT PK | ya | `YYYY-MM-DD` |
| equity_start | TEXT | ya | 00:00 UTC |
| sl_count | INTEGER | ya | 0..6 |
| realized_net | TEXT | ya | USDT |
| daily_stop_at_ms | INTEGER | tidak | |
| cooldown_until_ms | INTEGER | tidak | global |

### 1.8 `events_audit` (FR-025)
| field | tipe | wajib? | keterangan |
|---|---|---|---|
| id | INTEGER PK | ya | |
| ts_ms | INTEGER | ya | |
| level | TEXT | ya | `INFO` \| `WARN` \| `CRIT` |
| code | TEXT | ya | mis. `ARM_ON`, `SL_MISSING`, `RECON_DIFF`, `RULE_VIOLATION`, `EQUITY_FLOOR`, `DAILY_STOP` |
| ref_id | TEXT | tidak | signal/order id |
| detail | TEXT (JSON) | ya | tanpa secret |

### 1.9 `trials_ledger` (FR-032)
| field | tipe | wajib? | keterangan |
|---|---|---|---|
| id | INTEGER PK | ya | |
| ts_ms | INTEGER | ya | |
| source | TEXT | ya | `apex_scalper` \| `apex_legacy` |
| strategy | TEXT | ya | |
| params_hash, data_hash | TEXT | ya | sha256 |
| period | TEXT | ya | `IS` \| `VAL` \| `HOLDOUT` \| `WF_k` |
| n_trades | INTEGER | ya | |
| exp_r, pf, sharpe_trade, max_dd_r | REAL | ya | |

## 2. Kontrak API (axum, `http://127.0.0.1:8787`)

Semua respons `application/json`. Error: `{"error":{"code":"STRING","message":"STRING"}}` dengan HTTP 4xx/5xx. POST wajib header `X-Apex-Token`.

| Method | Path | Request | Response 200 | FR |
|---|---|---|---|---|
| GET | `/api/health` | — | `{"status":"ok"}` | FR-001 |
| GET | `/api/status` | — | lihat 2.1 | FR-026 |
| GET | `/api/screener` | — | `{"ts_ms":…, "items":[ScreenerItem]}` | FR-028 |
| GET | `/api/trades?date=YYYY-MM-DD` | query `date` (hari UTC, wajib) | lihat 2.2 | FR-027 |
| GET | `/api/signals?limit=50` | `limit` 1..500 | `{"items":[Signal]}` | FR-010 |
| POST | `/api/arm` | `{"phrase":"ARM testnet"}` | `{"arm":true,"mode":"testnet","armed_at_ms":…}`; 409 `{"error":{"code":"ARM_PRECONDITION","message":"…"}}` | FR-022 |
| POST | `/api/disarm` | `{}` | `{"arm":false}` | FR-022 |
| POST | `/api/kill` | `{}` | `{"arm":false,"canceled_entries":2}` | FR-023 |

### 2.1 `GET /api/status`
```json
{
  "arm": false,
  "mode": "testnet",
  "server_time_offset_ms": 12,
  "equity_usdt": "60.0000",
  "wallet_usdt": "60.0000",
  "open_positions": [Position],
  "pending_entries": [Order],
  "day_utc": "2026-11-05",
  "daily_sl_count": 0,
  "daily_realized_net": "0.0000",
  "daily_stop_active": false,
  "cooldown_until_ms": null,
  "strategies_go": ["S1"],
  "last_bar_ms": 1793840100000,
  "account_check": {"position_mode":"ONE_WAY","margin_type_ok":true,"leverage_ok":true}
}
```

### 2.2 `GET /api/trades`
```json
{
  "day_utc": "2026-11-05",
  "items": [Trade],
  "summary": {"n": 3, "wins": 1, "losses": 2, "pnl_net": "-0.1400", "sum_r": "-0.140", "fees": "0.0702", "funding": "0.0011"}
}
```

## 3. State Frontend

```ts
type Ui = {
  status: Status | null;              // dari /api/status, polling 5 dtk
  screener: ScreenerItem[];           // /api/screener, polling 60 dtk
  trades: { day: string; items: Trade[]; summary: Summary };
  token: string | null;               // sessionStorage, untuk POST
  loading: Record<'status'|'screener'|'trades', boolean>;
  error: string | null;
};
```
Tampilan waktu: konversi `ts_ms` → WIB (`Asia/Jakarta`) di frontend saja.

## 4. Contoh Data Nyata

Contoh angka berikut dihitung dari rumus Rules §4.1 dengan filter ETHUSDT snapshot 2026-11-04T18:58Z (tick 0,01; step 0,001; minNotional 20).

**Signal**
```json
{"id":"sig-S1-ETHUSDT-1793840100000","ts_ms":1793840100000,"strategy":"S1","symbol":"ETHUSDT","side":"LONG",
 "entry":"3000.00","sl":"2925.00","tp":"3150.00","sl_pct":"2.5000","screener_state":"TREND_UP","decision":"ORDERED",
 "sizing":{"qty":"0.012","notional":"36.0000","margin":"1.4400","risk_planned":"0.9318"}}
```
Perhitungan:
- stop_fill = 2925 × (1 − 0,0002) = 2924,415
- loss_per_unit = (3000 − 2924,415) + 3000×0,0002 + 2924,415×0,0005 = 75,585 + 0,6 + 1,4622 = 77,6472
- qty_raw = 0,01288 → qty = 0,012
- risk = 0,012 × 77,6472 = **0,9318** (≥ 0,80 → lolos)
- notional = 0,012 × 3000 = 36,00 (≥ minNotional 20)

Margin = 36 / 25 = 1,44 USDT (tidak tetap; mengikuti SL). Ini kasus uji U-03 di Rules §4.1.

**Position**
```json
{"id":"sig-S1-ETHUSDT-1793840100000","symbol":"ETHUSDT","side":"LONG","qty":"0.012","entry_avg":"3000.00","sl":"2925.00","tp":"3150.00",
 "risk_planned":"0.9318","margin":"1.4400","state":"PROTECTED","opened_ms":1793840220000,"protected_ms":1793840220850,"closed_ms":null}
```

**Trade** (TP kena: gross 0,012×150 = 1,80; fee maker masuk 0,0072 + taker keluar 0,0189 = 0,0261; 1× funding 0,01% × 36 = 0,0036)
```json
{"id":"sig-S1-ETHUSDT-1793840100000","strategy":"S1","symbol":"ETHUSDT","side":"LONG","entry_avg":"3000.00","exit_avg":"3150.00",
 "exit_reason":"TP","pnl_gross":"1.8000","fees":"0.0261","funding":"0.0036","pnl_net":"1.7703","r_multiple":"1.900","slippage_r":"0.000",
 "opened_ms":1793840220000,"closed_ms":1793883420000}
```

**ScreenerItem**
```json
{"symbol":"SOLUSDT","state":"TREND_UP","dir_votes":4,"adx":31.2,"er":0.41,"chop":38.5,"srv_1h":0.52,"srv_4h":0.33,
 "atr_pct_1h":0.92,"atr_ratio":1.18,"qv24_musd":2102.2,"spread_bps_med":1.05,"funding_pct":0.01,"chg24_pct":4.1,"reasons":[]}
```

## 5. Validasi

| Data | Aturan | Presisi |
|---|---|---|
| Harga (entry, sl, tp, trigger) | > 0; kelipatan `tick_size`. SL long dibulatkan **ke bawah**, SL short **ke atas**; TP long ke bawah, short ke atas; entry limit long ke bawah, short ke atas | sesuai tickSize simbol |
| qty | > 0; `floor(qty_raw / step) × step`; ≥ min_qty | sesuai stepSize |
| notional | qty × entry ≥ min_notional | USDT 4 desimal |
| sl_pct | `|entry − sl| / entry × 100` ∈ **[2,0000; 4,0000]** (inklusif), dihitung dari harga setelah pembulatan tick | persen 4 desimal |
| risk_planned | qty × loss_per_unit ∈ **[0,8000; 1,0000]** USDT | USDT 4 desimal |
| margin | **= notional / 25**; tidak pernah input bebas; tidak ada nilai tetap; rentang hasil yang diharapkan 0,9700–1,8900 USDT | USDT 4 desimal |
| leverage | = 25 (integer) | — |
| margin_type | = `CROSSED` | — |
| posisi+pending | ≤ 2 total, ≤ 1 per simbol | — |
| daily_sl_count | 0..6 | integer |
| daily_realized_net | stop bila ≤ −6,0000 | USDT 4 desimal |
| cooldown | `now_ms ≥ last_sl_fill_ms + 3_600_000` | ms |
| equity floor | equity ≥ 40,0000 | USDT 4 desimal |
| r_multiple | pnl_net / risk_planned | 3 desimal |
| persentase tampilan | 2 desimal di UI | — |
| timestamp | epoch ms UTC; hari = UTC (`YYYY-MM-DD`) | ms |
| `date` query | regex `^\d{4}-\d{2}-\d{2}$` | — |
| `phrase` ARM | tepat `ARM testnet` atau `ARM mainnet` dan sama dengan `APEX_MODE` | — |
