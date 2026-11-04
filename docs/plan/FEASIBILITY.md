# FEASIBILITY — Apex Scalper RS

> Versi 1.0 · disusun 5 Nov 2026 (WIB) · semua angka di bawah **diambil atau dihitung** dari data publik Binance yang diunduh pada **2026-11-04T18:58:11Z = 5 Nov 2026 01:58 WIB**. Tidak ada API key yang dipakai dan tidak ada order yang dikirim.
> Data mentah ada di `/workspace/scalp/binance/` (tidak wajib ikut zip; ada di `binance/` bila disertakan).

## 0. Sumber data (URL + waktu ambil)

| # | Data | URL | Waktu ambil |
|---|---|---|---|
| D1 | Filter simbol (tickSize, stepSize, minQty, minNotional, status) | https://fapi.binance.com/fapi/v1/exchangeInfo | 2026-11-04T18:58Z (01:58 WIB 5 Nov) |
| D2 | Volume 24 jam, perubahan 24 jam | https://fapi.binance.com/fapi/v1/ticker/24hr | idem |
| D3 | Mark price, funding terakhir | https://fapi.binance.com/fapi/v1/premiumIndex | idem |
| D4 | Bid/ask terbaik (spread) | https://fapi.binance.com/fapi/v1/ticker/bookTicker | idem |
| D5 | Leverage bracket + MMR (publik) | https://www.binance.com/bapi/futures/v1/friendly/future/common/brackets | idem (endpoint resmi `/fapi/v1/leverageBracket` butuh API key → 401, tidak dipakai) |
| D6 | Klines 5m/15m/1h, 1500 bar, 10 simbol | https://fapi.binance.com/fapi/v1/klines | idem; jendela 1h = 2026-09-03 08:00 UTC s/d 2026-11-04 18:00 UTC (≈62,5 hari); 15m ≈15,6 hari; 5m ≈5,2 hari |
| D7 | Fee VIP0 USDⓈ-M: maker 0,0200% / taker 0,0500% (diskon BNB 0,0180% / 0,0450%) | https://www.binance.com/en/fee/futureFee | dicek 5 Nov 2026 |
| D8 | Order kondisional (STOP_MARKET, TAKE_PROFIT_MARKET, dst.) wajib lewat `POST /fapi/v1/algoOrder` sejak 2025-12-09; `/fapi/v1/order` → error -4120 | https://developers.binance.com/docs/derivatives/change-log (entri 2025-11-06) | dicek 5 Nov 2026 |

Snapshot ini adalah **satu titik waktu**. Harga, filter, dan bracket berubah. Engine WAJIB memuat ulang D1 dan D5 saat start dan setiap 60 menit (FR-003 di PRD).

## 1. Aturan sizing yang diuji (Golden Rule GR-1/GR-2)

Risiko per trade = **$1,00 neto** (fee masuk + fee keluar + slippage stop sudah dihitung). Margin **tidak tetap**:

```
side            = +1 long, -1 short
stop_fill       = SL × (1 − side × slip)                 # slip model = 0,02%
loss_per_unit   = side × (entry − stop_fill) + entry × fee_in + stop_fill × fee_out
qty_raw         = 1.00 / loss_per_unit
qty             = floor(qty_raw / stepSize) × stepSize   # SELALU dibulatkan ke bawah
notional        = qty × entry
margin          = notional / 25
risk_aktual     = qty × loss_per_unit
SKIP jika: SL% ∉ [2,00%; 4,00%] ATAU risk_aktual < 0,80 ATAU notional < minNotional ATAU qty < minQty
```

### 1.1 Notional dan margin per jarak SL (taker masuk + taker keluar + slip 0,02%)

| Jarak SL | Notional | Margin @25x | Fee+slip saat kalah (R) | Hasil saat menang di RR 2 (R) | Win-rate impas |
|---|---|---|---|---|---|
| 2,0% | $47,19 | $1,89 | 0,047R | +1,830R | 35,3% |
| 2,5% | $38,19 | $1,53 | 0,038R | +1,863R | 34,9% |
| 3,0% | $32,07 | $1,28 | 0,032R | +1,885R | 34,7% |
| 3,5% | $27,64 | $1,11 | 0,027R | +1,901R | 34,5% |
| 4,0% | $24,28 | $0,97 | 0,024R | +1,913R | 34,3% |

Rentang margin per posisi: **$0,97 – $1,89**. Dua posisi terbuka memakai margin total **$1,94 – $3,78** dari wallet $60.

### 1.2 Pengaruh tipe order terhadap biaya (R per trade kalah)

| Jarak SL | maker+maker | maker masuk + taker keluar | taker+taker |
|---|---|---|---|
| 2% | 0,019R | 0,033R | 0,047R |
| 3% | 0,013R | 0,022R | 0,032R |
| 4% | 0,010R | 0,017R | 0,024R |

Win-rate impas di RR 2 untuk semua kombinasi: **33,7% – 35,3%**. SL dan TP lewat `algoOrder` (STOP_MARKET/TAKE_PROFIT_MARKET) **selalu taker**. Jadi skenario realistis = "maker masuk + taker keluar" (entry limit GTX) atau "taker+taker" (entry market).

### 1.3 Funding
Funding terakhir untuk sebagian besar simbol layak ≈ **0,01% per 8 jam** (D3). Pada SL 2% (notional $47,19), itu ≈ **0,0047R per 8 jam = 0,0142R per hari** per posisi. Ini kecil dibanding fee, tetapi wajib masuk model biaya karena holding diperkirakan berjam-jam (§4).

### 1.4 Efek pembulatan stepSize
Pembulatan ke bawah membuat risk aktual sedikit di bawah $1. Rasio "nilai 1 step / notional" menentukan seberapa besar turunnya. Contoh dari snapshot:
- **NEAR**: 1 step = $4,02 → risk 0,88 di SL 3%.
- **UNI**: 1 step = $7,62 → risk 0,83 di SL 3,5%.
- **PAXG**: 1 step = $4,06 → risk 0,83 di SL 4%.
- **BNB**: 1 step = $7,17 → risk 0,78 di SL 3,5% → **SKIP** untuk SL itu.
- **AVAX**: 1 step = $8,81 → risk 0,73 di SL 4% → SKIP.
- **AAVE**: 1 step = $13,16 → risk 0,54 di SL 4% → SKIP.

Karena itu kelayakan dicek **per trade**, bukan per simbol saja.

## 2. Kelayakan simbol (top 30 perp USDT berdasarkan volume 24 jam dari 522 perp USDT berstatus TRADING)

Kriteria: lev maks bracket-1 ≥ 25x **dan** di semua grid SL 2/2,5/3/3,5/4%, notional ≥ minNotional, qty ≥ minQty, risk aktual ≥ $0,80. Biaya: taker+taker + slip 0,02%. Kolom notional/risk menampilkan SL 2% · 3% · 4%; ✗ berarti gagal.

| Simbol | Harga | Vol 24j (juta USDT) | Spread (bps) | tickSize | stepSize | minQty | minNotional | Nilai 1 step (USDT) | Lev maks bracket-1 | MMR bracket-1 | Funding terakhir (%) | Notional/risk aktual @SL 2% · 3% · 4% | Layak? |
|---|---|---|---|---|---|---|---|---|---|---|---|---|---|
| BTCUSDT | 80297.9 | 18911.1 | 0.01 | 0.1 | 0.001 | 0.001 | 50.0 | 80.2979 | 150x | 0.40% | 0.0094 | $0.0/0.00✗ · $0.0/0.00✗ · $0.0/0.00✗ | TIDAK (gagal di SL 2%, SL 2.5%, SL 3%, SL 3.5%, SL 4%) |
| ETHUSDT | 2421.91 | 12645.9 | 0.04 | 0.01 | 0.001 | 0.001 | 20.0 | 2.4219 | 150x | 0.40% | 0.01 | $46.02/0.97 · $31.48/0.98 · $24.22/1.00 | YA |
| ZECUSDT | 1283.88 | 2882.3 | 0.08 | 0.01 | 0.001 | 0.001 | 5.0 | 1.2839 | 75x | 1.00% | 0.01 | $46.22/0.98 · $30.81/0.96 · $23.11/0.95 | YA |
| SOLUSDT | 95.21 | 2102.2 | 1.05 | 0.01 | 0.01 | 0.01 | 5.0 | 0.9521 | 100x | 0.50% | 0.01 | $46.65/0.99 · $31.42/0.98 · $23.8/0.98 | YA |
| HYPEUSDT | 84.346 | 1156.5 | 0.12 | 0.001 | 0.01 | 0.01 | 5.0 | 0.8435 | 75x | 1.00% | 0.005 | $46.39/0.98 · $32.05/1.00 · $23.62/0.97 | YA |
| XRPUSDT | 1.306 | 930.0 | 0.77 | 0.0001 | 0.1 | 0.1 | 5.0 | 0.1306 | 100x | 0.50% | 0.01 | $47.15/1.00 · $32.0/1.00 · $24.16/0.99 | YA |
| NEARUSDT | 4.018 | 732.8 | 2.49 | 0.001 | 1.0 | 1.0 | 5.0 | 4.018 | 75x | 1.00% | 0.01 | $44.2/0.94 · $28.13/0.88 · $24.11/0.99 | YA |
| BNBUSDT | 717.2 | 518.0 | 0.14 | 0.01 | 0.01 | 0.01 | 5.0 | 7.172 | 75x | 0.50% | 0.0159 | $43.03/0.91 · $28.69/0.89 · $21.52/0.89 | TIDAK (gagal di SL 3.5%) |
| 1000PEPEUSDT | 0.0049741 | 484.2 | 0.2 | 1e-07 | 1.0 | 1.0 | 5.0 | 0.005 | 75x | 0.65% | 0.01 | $47.2/1.00 · $32.07/1.00 · $24.28/1.00 | YA |
| DOGEUSDT | 0.07656 | 438.0 | 1.31 | 1e-05 | 1.0 | 1.0 | 5.0 | 0.0766 | 75x | 0.65% | 0.01 | $47.16/1.00 · $32.0/1.00 · $24.27/1.00 | YA |
| ENAUSDT | 0.18032 | 373.9 | 0.55 | 1e-05 | 1.0 | 1.0 | 5.0 | 0.1803 | 75x | 1.00% | 0.005 | $47.06/1.00 · $31.92/0.99 · $24.16/0.99 | YA |
| UNIUSDT | 7.622 | 352.4 | 1.31 | 0.001 | 1.0 | 1.0 | 5.0 | 7.622 | 75x | 0.60% | 0.01 | $45.73/0.97 · $30.49/0.95 · $22.87/0.94 | YA |
| CLANKERUSDT | 23.61 | 347.6 | 8.43 | 0.01 | 0.01 | 0.01 | 5.0 | 0.2361 | 25x | 2.00% | 0.0414 | $46.98/0.99 · $31.87/0.99 · $24.08/0.99 | YA |
| TRUMPUSDT | 1.295 | 282.3 | 7.72 | 0.001 | 0.01 | 0.01 | 5.0 | 0.0129 | 75x | 1.00% | -0.0409 | $47.19/1.00 · $32.06/1.00 · $24.28/1.00 | YA |
| WLDUSDT | 0.3765 | 261.9 | 2.66 | 0.0001 | 1.0 | 1.0 | 5.0 | 0.3765 | 75x | 1.00% | -0.0344 | $47.06/1.00 · $32.0/1.00 · $24.1/0.99 | YA |
| SUIUSDT | 0.9286 | 246.5 | 1.08 | 0.0001 | 0.1 | 0.1 | 5.0 | 0.0929 | 75x | 1.00% | 0.01 | $47.17/1.00 · $32.04/1.00 · $24.24/1.00 | YA |
| HOTUSDT | 0.0004668 | 221.5 | 2.14 | 1e-07 | 1.0 | 1.0 | 5.0 | 0.0005 | 50x | 1.50% | -0.0353 | $47.2/1.00 · $32.07/1.00 · $24.29/1.00 | YA |
| LINKUSDT | 11.466 | 199.7 | 0.87 | 0.001 | 0.01 | 0.01 | 20.0 | 0.1147 | 75x | 0.50% | 0.01 | $47.13/1.00 · $31.99/1.00 · $24.19/1.00 | YA |
| BMTUSDT | 0.02866 | 181.6 | 3.49 | 1e-05 | 1.0 | 1.0 | 5.0 | 0.0287 | 20x | 2.50% | -0.3065 | $47.17/1.00 · $32.07/1.00 · $24.28/1.00 | TIDAK (lev maks 20x < 25x) |
| ADAUSDT | 0.1953 | 174.5 | 5.12 | 0.0001 | 1.0 | 1.0 | 5.0 | 0.1953 | 75x | 0.50% | 0.01 | $47.07/1.00 · $32.03/1.00 · $24.22/1.00 | YA |
| PUMPUSDT | 0.004152 | 173.8 | 2.41 | 1e-06 | 1.0 | 1.0 | 5.0 | 0.0042 | 75x | 1.00% | 0.005 | $47.2/1.00 · $32.07/1.00 · $24.29/1.00 | YA |
| AKEUSDT | 0.002608 | 169.0 | 3.84 | 1e-06 | 1.0 | 1.0 | 5.0 | 0.0026 | 10x | 5.00% | -0.024 | $47.2/1.00 · $32.07/1.00 · $24.29/1.00 | TIDAK (lev maks 10x < 25x) |
| JCTUSDT | 0.005439 | 163.1 | 3.68 | 1e-06 | 1.0 | 1.0 | 5.0 | 0.0054 | 20x | 4.00% | 0.005 | $47.2/1.00 · $32.07/1.00 · $24.29/1.00 | TIDAK (lev maks 20x < 25x) |
| AVAXUSDT | 8.814 | 143.2 | 1.13 | 0.001 | 1.0 | 1.0 | 5.0 | 8.814 | 75x | 0.50% | 0.01 | $44.07/0.93 · $26.44/0.82 · $17.63/0.73✗ | TIDAK (gagal di SL 4%) |
| PAXGUSDT | 4055.62 | 129.3 | 0.02 | 0.01 | 0.001 | 0.001 | 5.0 | 4.0556 | 75x | 1.00% | 0.005 | $44.61/0.94 · $28.39/0.89 · $20.28/0.83 | YA |
| IDOLUSDT | 0.023134 | 118.1 | 1.3 | 1e-06 | 1.0 | 1.0 | 5.0 | 0.0231 | 20x | 2.50% | 0.0285 | $47.19/1.00 · $32.06/1.00 · $24.27/1.00 | TIDAK (lev maks 20x < 25x) |
| AAVEUSDT | 131.55 | 117.2 | 0.76 | 0.01 | 0.1 | 0.1 | 5.0 | 13.155 | 75x | 1.00% | 0.01 | $39.47/0.84 · $26.31/0.82 · $13.16/0.54✗ | TIDAK (gagal di SL 2.5%, SL 4%) |
| WLFIUSDT | 0.04712 | 115.5 | 2.12 | 1e-05 | 1.0 | 1.0 | 5.0 | 0.0471 | 75x | 1.00% | 0.005 | $47.17/1.00 · $32.04/1.00 · $24.27/1.00 | YA |
| TAOUSDT | 234.62 | 114.1 | 0.43 | 0.01 | 0.001 | 0.001 | 5.0 | 0.2346 | 75x | 1.00% | 0.005 | $47.16/1.00 · $31.91/0.99 · $24.17/0.99 | YA |
| LITUSDT | 3.4288 | 107.6 | 1.75 | 0.0001 | 0.1 | 0.1 | 5.0 | 0.3429 | 75x | 1.00% | 0.0048 | $46.97/0.99 · $31.89/0.99 · $24.0/0.99 | YA |
### 2.1 Ringkasan kelayakan
- **BTCUSDT TIDAK LAYAK** di modal ini. minNotional $50 dan minQty 0,001 × $80.297,9 = **$80,30** notional minimum. Itu > $47,19 (notional maksimum pada SL 2% dengan risk $1). BTC tetap dipakai sebagai **data konteks/regime**, bukan simbol trading.
- **Layak di semua grid SL (snapshot)**: ETH, ZEC, SOL, HYPE, XRP, NEAR, 1000PEPE, DOGE, ENA, UNI, CLANKER, TRUMP, WLD, SUI, HOT, LINK, ADA, PUMP, PAXG, WLFI, TAO, LIT.
- **Layak bersyarat** (cek per trade): BNB (gagal hanya di SL 3,5%), AVAX (gagal di 4%).
- **Tidak layak**:
  - AAVE (step terlalu kasar).
  - BMT, JCT, IDOL (lev maks 20x); AKE (10x).
- **Disaring oleh screener, bukan oleh sizing**:
  - CLANKER: lev maks tepat 25x, spread 8,43 bps, naik +103% dalam 24 jam.
  - TRUMP: spread 7,72 bps.
  - ADA: spread 5,12 bps.
  - PAXG: emas, bukan kripto-tren.
  - Batas spread ≤ 3 bps dan perubahan 24 jam ≤ 25% (lihat SCREENER.md).
- **Universe awal yang direkomendasikan** (likuid, spread kecil): ETH, SOL, XRP, DOGE, HYPE, SUI, LINK, ENA, ZEC, NEAR, 1000PEPE, WLD. Semua tetap disaring ulang oleh screener setiap jam.

## 3. Likuidasi pada CROSS 25x dengan wallet $60

Pendekatan: jarak harga yang membuat margin balance = maintenance margin, dengan n posisi berlawanan arah pasar yang sama besar dan bergerak bersamaan (skenario terburuk, korelasi = 1):

```
x = (W − n·N·m) / (n·N·(1 − m))
W = wallet, N = notional per posisi, m = MMR bracket-1, n = jumlah posisi
```

| Wallet | Notional per posisi | Posisi | MMR 0,4–1,0% | Gerak harga sampai likuidasi |
|---|---|---|---|---|
| $60 | $47,2 (SL 2%) | 1 | — | ≈127% |
| **$60** | **$47,2 (SL 2%)** | **2** | — | **≈63%** |
| $60 | $24,3 (SL 4%) | 2 | — | ≈124% |
| $54 (setelah stop harian −$6) | $47,2 | 2 | — | ≈57% (dihitung: (54−0,94)/(94,4×0,99)) |
| $50 | $47,2 | 2 | — | ≈52,6% |
| $100 | $47,2 | 2 | — | ≈106% |

- Leverage efektif akun maksimum = 2 × $47,2 / $60 ≈ **1,57x**, walaupun leverage simbol 25x.
- Nilai >100% berarti posisi long praktis tidak bisa terlikuidasi; untuk short, harga perlu naik sebesar itu.
- **Kesimpulan:** dengan CROSS, SL 2–4% selalu tersentuh jauh sebelum likuidasi.
- **Peringatan ISOLATED:** pada 25x, likuidasi ≈ 1/25 − MMR ≈ **3,0–3,6%**, yaitu DI DALAM SL 4%. Karena itu GR-4 mewajibkan CROSSED. Engine menolak trading bila marginType ≠ CROSSED.
- **Risiko nyata di CROSS** bukan likuidasi, tetapi **SL gagal terpasang/terpicu** (gap, API error), karena seluruh wallet menjadi jaminan. Mitigasinya ada di GR-8: SL di exchange wajib terkonfirmasi ≤ 2 detik setelah fill. Bila tidak, posisi ditutup market dan sistem HALT.

## 4. Volatilitas vs SL 2–4% (inti pertanyaan "apakah ini scalping?")

ATR14 dalam % harga (median), dari klines D6:

| Simbol | ATR14% median 5m | 15m | 1h | SL 2% = ×ATR 5m | SL 2% = ×ATR 15m | SL 2% = ×ATR 1h | SL 4% = ×ATR 1h | Median jam sampai harga bergerak ±2% | ±4% |
|---|---|---|---|---|---|---|---|---|---|
| BTCUSDT | 0.11 | 0.208 | 0.494 | 18.2 | 9.6 | 4.0 | 8.1 | 31 | 110.0 |
| ETHUSDT | 0.149 | 0.303 | 0.677 | 13.5 | 6.6 | 3.0 | 5.9 | 17 | 59.5 |
| SOLUSDT | 0.194 | 0.352 | 0.841 | 10.3 | 5.7 | 2.4 | 4.8 | 12 | 41.0 |
| XRPUSDT | 0.148 | 0.336 | 0.833 | 13.5 | 5.9 | 2.4 | 4.8 | 11.0 | 37 |
| DOGEUSDT | 0.172 | 0.352 | 0.918 | 11.7 | 5.7 | 2.2 | 4.4 | 9 | 31.0 |
| BNBUSDT | 0.13 | 0.239 | 0.541 | 15.4 | 8.4 | 3.7 | 7.4 | 21.0 | 79.0 |
| HYPEUSDT | 0.298 | 0.488 | 1.022 | 6.7 | 4.1 | 2.0 | 3.9 | 8.0 | 26.0 |
| SUIUSDT | 0.244 | 0.498 | 1.343 | 8.2 | 4.0 | 1.5 | 3.0 | 5 | 16.0 |
| LINKUSDT | 0.257 | 0.445 | 1.042 | 7.8 | 4.5 | 1.9 | 3.8 | 7 | 21.0 |
| ENAUSDT | 0.435 | 0.902 | 1.926 | 4.6 | 2.2 | 1.0 | 2.1 | 3 | 9.0 |
**Kesimpulan:**
- Untuk ETH, SL 2% = **13,5 ATR 5m**. Median waktu sampai harga bergerak ±2% ≈ **17 jam**, ±4% ≈ **59,5 jam**.
- SL 2–4% **bukan scalping menit-an**. Ini **intraday swing** dengan entry presisi di 5m/15m dan struktur SL dari 1h/4h.
- Konsekuensi untuk desain:
  1. Struktur SL diambil dari swing **15m/1h** (bukan 1m/5m). Bila swing terdekat < 2%, setup di-SKIP; SL tidak boleh dilebarkan buatan.
  2. Simbol bervolatilitas lebih tinggi (ENA 1,93%/jam, SUI 1,34%, LINK 1,04%, HYPE 1,02%) mencapai 2% jauh lebih sering (3–8 jam) daripada ETH (17 jam).
  3. Frekuensi trade diperkirakan rendah, yaitu orde beberapa per hari untuk seluruh universe, bukan puluhan. Angka pastinya baru diketahui di Fase 2 (backtest).
  4. Funding (§1.3) ikut dihitung karena holding bisa melewati 1–3 jendela funding.
  5. Cooldown 60 menit dan batas 2 posisi jarang menjadi faktor pembatas utama; pembatas utamanya adalah ketersediaan setup.

## 5. Matematika modal $60 (risk tetap $1, tanpa compounding)

| Besaran | Nilai |
|---|---|
| Risk per trade | $1 = 1,67% dari $60 |
| Risk terbuka maksimum (2 posisi) | $2 = 3,33% |
| Stop harian (6 SL atau −$6) | −$6 = **10%** dari $60 |
| 3 hari stop harian berturut-turut | −$18 = 30% |
| 5 hari stop harian berturut-turut | −$30 = 50% |
| Ekspektansi +0,15R × 2 trade/hari (contoh hipotetis, BELUM tervalidasi) | +$0,30/hari ≈ 0,5% dari $60 |
| Kebutuhan agar sistem bernilai | Ekspektansi neto ≥ +0,10R setelah biaya, pada ≥ 200 trade out-of-sample |

Pertumbuhan bersifat **linier** karena risk tetap $1. Pada contoh di atas, menggandakan $60 butuh ±200 hari. Bila user kelak ingin compounding, itu keputusan terpisah yang mengubah GR-1. Sistem ini tidak melakukannya.

Di bawah wallet ≈ $40, likuidasi 2 posisi SL 2% ≈ 41%. Itu masih jauh dari SL, tetapi Rules.md memasang **hard floor equity $40**: engine DISARM dan butuh keputusan user.

## 6. Status bukti dari riset apex sebelumnya

- REVIEW-apex (C8): backtest lama tidak valid. PF = 0,39 setelah stress biaya, dan modul `backtest.rs` mengandung look-ahead (H1).
- Riset `tv7_scalp_20261104` memakai IS 2025 / OOS 2026 untuk BTC/ETH. Data 2026 untuk BTC/ETH sudah "terlihat" oleh ≈40 versi skrip pencarian, jadi **tidak lagi bersih sebagai holdout** untuk BTC/ETH. Holdout baru didefinisikan di Rules.md §4.
- **Belum ada strategi yang terbukti profit.** Semua strategi di STRATEGIES.md berstatus **HIPOTESIS** sampai lolos gerbang validasi.

## 7. Daftar peringatan utama

1. BTC tidak bisa ditradingkan dengan risk $1 (min notional efektif $80,30).
2. SL 2–4% = 10–27 ATR 5m pada mayor. Holding diperkirakan berjam-jam, bukan menit.
3. Risk aktual turun sampai 0,78–0,88 pada simbol dengan step kasar. Aturan SKIP < $0,80 aktif (BNB, AVAX, AAVE).
4. Biaya taker+taker = 0,047R per trade kalah pada SL 2%. Win-rate impas di RR 2 = 34–35%.
5. ISOLATED 25x terlikuidasi di 3,0–3,6% (< SL 4%), jadi wajib CROSS.
6. Stop harian −$6 = 10% wallet. Lima hari buruk berturut-turut = −50%.
7. SL/TP wajib lewat `/fapi/v1/algoOrder` (sejak 2025-12-09). Order kondisional tidak bisa di-modify (cancel lalu buat ulang) dan tidak dicek margin sebelum terpicu.
