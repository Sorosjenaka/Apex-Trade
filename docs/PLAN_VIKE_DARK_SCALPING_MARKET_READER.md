# Plan — Vike Dark Crypto Market Reader + Scalping Analysis

**Status:** PLAN / read-only design and implementation contract  
**Primary surface:** Monitor + Command/Inspect  
**Target project:** `C:\Users\FATHUR\Downloads\apex-scalper-rs\`  
**Target URL:** `http://127.0.0.1:8787/`

## 1. Koreksi visual wajib

Dashboard harus tetap **gelap**. Screenshot video dipakai sebagai referensi struktur informasi, bukan untuk menyalin warna terang atau identitas produknya.

Visual target:

- background charcoal/black;
- panel gelap dengan garis tipis biru-abu;
- hijau = buy/bullish/positive flow;
- merah = sell/bearish/liquidation/negative flow;
- amber = current price, warning, selected state;
- blue = neutral/context/order-book reference;
- dense terminal layout, bukan kartu marketing;
- tidak memakai angka performa palsu seperti equity `$1.6M`, PF `3.2`, atau win rate `67%`.

Patch CSS gelap untuk panel Heatmap disimpan di:

```text
dashboard/static/dashboard-upgrade.css
```

## 2. Bentuk layar yang akan dibuat

### A. Top market strip

Lima blok live yang berasal dari data nyata:

1. `MARKET STATE` — risk-on, risk-off, chop, shock, atau mixed.
2. `ACTIVE SYMBOL` — pair yang sedang dianalisis dan venue-nya.
3. `MARK / INDEX` — harga dari Binance, bukan harga Vike yang dicampur diam-diam.
4. `FLOW BIAS` — Binance taker delta + Vike cohort bias.
5. `FEED HEALTH` — umur data, latency, source, dan stale/error state.

Paper equity/statistics hanya tampil jika benar-benar dibaca dari state paper engine. Tidak ada KPI demo.

### B. Main three-column workspace

#### Kolom kiri — Vike Cost/Liquidity Map

- liquidation heatmap per price bin;
- long liquidation value vs short liquidation value;
- current Binance mark line;
- Vike snapshot time;
- cluster/order level jika tersedia;
- realized liquidation events di sekitar window scalping;
- label source: `VIKE / Hyperliquid`, bukan `BINANCE`.

#### Kolom tengah — Binance live microstructure

- live bid/ask depth ladder;
- spread dalam bps;
- cumulative bid/ask notional;
- wall lifelines;
- aggTrade/taker tape;
- buy/sell delta dan CVD;
- Binance mark/index/funding/OI.

#### Kolom kanan — Matrix + chart

- market matrix untuk 522 pair aktif Binance USD-M USDT perpetual;
- filter: volume, 24h move, funding, OI, Vike bias, Vike liquidation risk;
- pair yang dipilih diberi outline amber;
- chart candlestick 1m/5m/15m;
- default chart **tanpa** S02/SNIPER, LONG/SHORT marker, risk zone, entry/SL/TP;
- panel analisis read-only berada di bawah chart, bukan menggambar order overlay.

### C. Bottom orchestration strip

Bentuknya mengikuti topology pada video, tetapi isinya nyata dan audit-able:

```text
BINANCE FEED → VIKE CONTEXT → NORMALIZE → REGIME → SCALP READ → PAPER WATCH
```

Setiap node menampilkan:

- status `OK / STALE / ERROR / CONFLICT`;
- source;
- timestamp UTC/WIB;
- age data;
- alasan state;
- tidak ada node `EXECUTE LIVE`.

## 3. Sumber data dan batas venue

Vike yang dirujuk adalah data Hyperliquid/cohort analytics. Binance adalah venue market reader dan venue eksekusi/paper reference. Level harga Vike tidak boleh dipresentasikan sebagai order book Binance.

### Vike context endpoints

Endpoint yang direncanakan:

```text
/v1/hl-cohorts/heatmap?axis=size|pnl
/v1/hl-cohorts/symbols/{symbol}/cohort-metrics?axis=size|pnl&hours=...
/v1/hl-cohorts/symbols/{symbol}/indices
/v1/hl-cohorts/symbols/{symbol}/size-breakdown
/v1/hl-cohorts/symbols/{symbol}/large-fills?hours=...
/v1/hyperliquid/symbols/{symbol}/order-clusters?range_pct=...
/v1/hyperliquid/symbols/{symbol}/liquidation-heatmap?bins=...
/v1/hyperliquid/liquidations
/v1/hyperliquid/liquidations/risk
/v1/hyperliquid/funding-rates
/v1/hyperliquid/symbols/hourly
```

Sumber endpoint dan makna datanya dicatat pada Vike docs references [1]-[5].

### Binance execution/microstructure endpoints

```text
/fapi/v1/exchangeInfo
/fapi/v1/ticker/price
/fapi/v1/ticker/24hr
/fapi/v1/klines
/fapi/v1/depth
/fapi/v1/premiumIndex
/fapi/v1/openInterest
/futures/data/globalLongShortAccountRatio
/futures/data/takerlongshortRatio
```

WebSocket depth dipakai bila sehat; REST fallback tetap disediakan. UI wajib menunjukkan `WS`, `REST`, atau `STALE`.

## 4. Data contract wajib

Setiap record yang masuk ke analisis harus membawa:

```text
symbol
venue
source
retrieved_at_ms
source_timestamp_ms / snapshot_ts
age_ms
latency_ms jika tersedia
payload_hash atau request identity jika diperlukan
stale
error
```

Aturan:

- `BTCUSDT` Binance dipetakan ke `BTC` Vike hanya untuk context comparison.
- Harga, depth, funding, dan OI tidak boleh dicampur lintas venue tanpa label.
- Jika Vike gagal/429/401, tampilkan `VIKE OFFLINE` atau last-good + umur cache.
- Tidak pernah mengarang liquidation level, cohort value, funding, atau signal.
- `VIKE_API_KEY` tetap server-side environment variable; tidak masuk browser, repo, log, atau chat.

## 5. Mode analisis scalping

Mode ini **read-only analysis**, bukan order executor. Outputnya adalah `market read`, `watch zone`, dan `invalidation`; bukan instruksi submit order.

### Timeframe hierarchy

```text
15m = context/regime
5m  = setup/structure
1m  = trigger/tape confirmation
```

Jika data Vike hanya hourly atau snapshot, umurnya ditampilkan dan bobotnya diturunkan; Vike tidak boleh diperlakukan sebagai tick feed.

### Feature groups

#### 1. Cohort bias — Vike

- size axis: whales/large/smaller accounts;
- pnl axis: smart-money/realized-PnL cohorts;
- `long_usd`, `short_usd`, `bias`, `share_of_symbol`;
- perubahan bias versus snapshot sebelumnya.

#### 2. Liquidation pressure — Vike

- long vs short liquidation value by price bin;
- distance dari Binance mark ke cluster liquidation;
- realized liquidation direction dan notional;
- liquidation risk share/open-interest context bila tersedia.

#### 3. Large flow — Vike + Binance

- Vike large fills by side/cohort;
- Binance aggTrade/taker delta;
- CVD direction and acceleration;
- divergence between large fills and public tape.

#### 4. Order clusters/walls — Vike + Binance

- Vike standing order clusters = context only;
- Binance depth20/walls = execution venue microstructure;
- support/resistance proximity;
- wall persistence and cancellation/decay.

#### 5. Funding/OI/crowding

- Binance funding and OI are execution-venue metrics;
- Vike funding/hourly context is separately labeled;
- rising OI + one-sided crowding is a squeeze-risk flag, not automatically a direction signal.

### Read-only context score v0

Setiap feature group dinormalisasi ke `[-1,+1]` dan ditampilkan bersama komponen mentahnya:

```text
B = cohort bias
L = liquidation asymmetry
F = large-flow/taker confirmation
O = order-cluster/wall context
C = funding + OI crowding
M = Binance candle/microstructure

ContextScore = 100 * (0.25*B + 0.20*L + 0.20*F + 0.15*O + 0.10*C + 0.10*M)
```

Interpretasi UI awal, **bukan klaim validasi**:

```text
+45..+100  bullish context
+20..+44   bullish lean
-19..+19   mixed / no edge
-44..-20   bearish lean
-100..-45  bearish context
```

Confidence terpisah dari score:

```text
confidence = f(data freshness, venue agreement, component coverage, conflict count)
```

Score tidak boleh berubah menjadi entry otomatis. Threshold wajib diuji kemudian pada data historis point-in-time.

### Output analisis per pair

Contoh struktur:

```text
ETHUSDT / Binance Futures
15m context: RISK_OFF / mixed
5m structure: bearish continuation / reversal watch
1m tape: sell delta accelerating
Vike smart cohort: short bias, age 42s
Vike whale cohort: neutral, age 38s
Vike liquidation risk: short-side cluster above / long-side cluster below
Binance depth: bid wall weak, ask wall persistent
ContextScore: -58
Confidence: 74/100
Read: SHORT-BIAS WATCH, not an order
Trigger: wait for 1m sell confirmation + no bid-wall refill
Invalidation: score conflict, stale Vike > threshold, or ask wall removed
```

Narrative harus terbentuk dari ledger komponen yang sama, bukan dari teks bebas yang tidak dapat diaudit.

## 6. Matrix seluruh market

### Universe layer

- 522 active USDT perpetuals dari Binance `exchangeInfo` pada snapshot terakhir;
- refresh listings berkala;
- price/24h/volume semua pair secara ringan;
- Vike deep analytics hanya untuk watchlist/top candidates agar tidak membakar quota.

### Watchlist/deep layer

```text
Layer A: 522 pair — price, 24h %, quote volume, status
Layer B: top 50 — funding, OI, taker ratio, volatility
Layer C: 1–3 pair aktif — depth, aggTrade, Vike cohorts, liq map, analysis
```

Pair ranking harus memakai volume/liquidity dan freshness, bukan hanya persentase gerak.

## 7. Vike API/rate/caching

- Semua request Vike lewat server proxy.
- Cache last-good dengan TTL per endpoint.
- Exponential backoff untuk 401/403/429/network failure.
- UI menampilkan `LIVE`, `CACHED`, `STALE`, `ERROR`.
- Endpoint mahal seperti heatmap/cohort history tidak dipanggil setiap frame.
- `health` mengeluarkan feed age, error terakhir, HTTP code, source, dan cache age.
- Key presence boleh ditampilkan sebagai `configured=true/false` dan panjang key saja; nilai key tidak pernah ditampilkan.

## 8. Fase implementasi

### Fase 0 — Dark correction dan kontrak data

- Revert seluruh light-theme override.
- Lock dark palette.
- Pastikan overlay strategy default OFF.
- Tambah precision/tick metadata ke symbol API.
- Tambah source/age/stale badge.

### Fase 1 — Dark video-style shell

- top KPI market strip;
- three-column workspace;
- bottom orchestration strip;
- responsive layout;
- no fake account performance.

### Fase 2 — Vike adapter

- normalisasi endpoint dan payload;
- strict server-side key handling;
- cache/backoff;
- unit fixtures untuk heatmap, cohort metrics, indices, large fills, liquidation events, risk, funding.

### Fase 3 — Binance microstructure adapter

- depth20/order book;
- aggTrade/tape;
- CVD/delta;
- funding/OI/long-short/taker ratio;
- WebSocket + REST fallback.

### Fase 4 — Scalping analysis ledger

- 15m/5m/1m hierarchy;
- feature normalization;
- ContextScore + confidence;
- conflict/stale logic;
- narrative generation;
- no execution side effect.

### Fase 5 — Verification

- test payload parser dan venue mapping;
- test no-secret logging;
- test cache stale behavior;
- test prefix/future mutation untuk timestamp availability;
- test score dari fixture manual;
- browser screenshot dark;
- live health probe untuk Binance dan Vike;
- verify matrix 522 pair dan deep panel ETH/BTC.

## 9. Acceptance criteria

Pekerjaan dianggap selesai jika:

- [ ] Heatmap tampil gelap, bukan beige/terang.
- [ ] Screenshot-style composition tersedia: KPI strip, liq map, L2, matrix, candle, topology.
- [ ] Tidak ada angka performa demo/fabrikasi.
- [ ] Setiap Vike value punya source, timestamp, age, dan stale/error status.
- [ ] Binance dan Vike venue tidak tercampur tanpa label.
- [ ] Vike key tidak pernah masuk client/log/repo.
- [ ] Vike 401/403/429/network error menghasilkan state jujur, bukan data kosong yang dianggap live.
- [ ] ContextScore dapat diaudit dari enam komponen mentah.
- [ ] Scalping mode hanya analysis/watch/read; tidak mengirim order.
- [ ] Overlay S02/SNIPER/LONG/SHORT/SL/TP default tetap OFF.
- [ ] 522-pair matrix dan 1–3 pair deep analysis bekerja bersamaan tanpa rate-limit loop.
- [ ] Browser smoke test dan endpoint smoke test lulus.

## 10. Batasan yang harus selalu terlihat

- Vike cohort/liquidation data yang dipakai berasal dari Hyperliquid analytics, bukan native Binance order book.
- Vike context bukan bukti harga akan bergerak ke satu arah.
- ContextScore v0 adalah hypothesis/read-only until backtested.
- Tidak ada jaminan profit scalping.
- Live market data bisa stale karena network, API quota, exchange outage, atau provider backoff.
- Tidak ada live order, wallet action, atau credential transfer pada fase ini.

## References

[1] Vike HL Cohorts endpoints: https://vike.io/docs/data/hl-cohorts/endpoints  
[2] Vike cohort heatmap: https://vike.io/docs/data/hl-cohorts/endpoints/heatmap  
[3] Vike cohort history: https://vike.io/docs/data/hl-cohorts/endpoints/segments-cohort-history  
[4] Vike daily brief workflow: https://vike.io/docs/data/cross-product/examples/daily-brief  
[5] Vike crowded-trade heatmap: https://vike.io/docs/data/hl-cohorts/examples/most-crowded-trade-heatmap
