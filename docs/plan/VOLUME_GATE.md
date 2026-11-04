# VOLUME_GATE.md — Gate Likuiditas Scalping (Apex Scalper RS)

> Versi 0.1 · data: fapi snapshots 20261104T190437Z + /fapi/v1/depth limit=100 fetched 2026-11-04T19:12:14Z (ticker 2026-11-05 02:04 WIB, depth 02:12 WIB).
> SSoT untuk: threshold gate volume/likuiditas & universe saat ini. Mesin: `universe_now.json`, script `gate.py`.
> Input universe = allowlist crypto-native [→ BLACKLIST.md] (516 simbol).

## 1. Distribusi nyata (516 perp crypto USDT, 24 jam)
| Statistik quoteVolume 24h | Nilai |
|---|---:|
| Persentil 90 (top 10%) | $49,7 M |
| Persentil 75 | $8,8 M |
| Median | $2,79 M |
| Persentil 25 | $1,29 M |
| Persentil 10 | $0,89 M |
| Simbol ≥ $5 M / ≥ $10 M / ≥ $20 M / ≥ $50 M / ≥ $100 M | 183 / 123 / 87 / 51 / 29 |
| Median jumlah trade 24h | 59.004 |
| Total quoteVolume semua 516 | $49,2 B; 31 simbol lolos gate = $23,8 B (48,5%; BTCUSDT tidak lolos, lihat G-01) |

## 2. Kebutuhan ukuran (dari aturan user [→ PRD.md §5 Functional Requirements])
- risk $1, SL 2–4% → notional $25–$50; margin 25x → $1–$2 per posisi.
- Order $50 kecil dibanding likuiditas; gate di sini bukan soal "bisa terisi", tapi (a) biaya spread vs target, (b) slippage saat SL tersentuh di momen cepat, (c) manipulasi/wick di koin tipis, (d) pembulatan qty tidak membuat risk keluar dari [$0,80; $1,00].

## 3. Threshold (semua harus lolos) 🔺 = hipotesis awal

| ID | Metrik | Threshold | Sumber | Alasan |
|---|---|---|---|---|
| V-01 | quoteVolume 24h | ≥ $20 M 🔺 | `/fapi/v1/ticker/24hr` | ≈ top 17% (87 simbol) |
| V-02 | jumlah trade 24h | ≥ 100,000 🔺 | `/fapi/v1/ticker/24hr` `count` | aktivitas merata, bukan 1–2 whale |
| V-03 | spread bid-ask | ≤ 5 bps (0,05%) 🔺 | `/fapi/v1/ticker/bookTicker` | ≤ 2,5% dari jarak SL 2% |
| V-04 | depth min(bid,ask) dalam ±0,1% dari mid | ≥ $20,000 🔺 | `/fapi/v1/depth?limit=100` | ≥ 400× notional maks $50 |
| V-05 | tickSize / harga | ≤ 5 bps 🔺 | exchangeInfo `PRICE_FILTER` | SL/TP bisa ditempatkan presisi |
| V-06 | minNotional DAN minQty×harga | ≤ $25 | exchangeInfo `MIN_NOTIONAL`, `LOT_SIZE` | order terkecil (SL 4% → $25) harus sah |
| V-07 | stepSize×harga | ≤ $5 (20% × $25) | `MARKET_LOT_SIZE` | pembulatan ke bawah max 1 step → risk ≥ $0,80 |
| V-08 | umur listing | ≥ 14 hari 🔺 | exchangeInfo `onboardDate` | hindari volatilitas listing baru |
| V-09 | \|funding terakhir\| | ≤ 0.1% per interval 🔺 | `/fapi/v1/premiumIndex` | funding ekstrem = crowding/squeeze |

Aturan pendukung:
- VG-01 Re-evaluasi: V-01/V-02/V-06/V-07/V-08 tiap 15 menit (ticker rolling 24h); V-03/V-04 kontinu dari WS `bookTicker` & `depth20@500ms` untuk kandidat screener, dinilai dengan **median 5 menit**, bukan satu snapshot (snapshot ini memakai 1 sampel → noise, mis. CRVUSDT gagal dengan depth $19,8k).
- VG-02 Hysteresis: simbol keluar universe bila gagal 3 evaluasi berturut-turut; masuk bila lolos 2 berturut-turut.
- VG-03 Cek per-trade (di luar gate, wajib di order engine): `qty = floor((1/SL%)/harga / step)·step`; tolak bila `qty·harga < minNotional` atau risk aktual < $0,80 atau > $1,00.
- VG-04 Leverage 25x: maksimum leverage per simbol hanya tersedia via `/fapi/v1/leverageBracket` (endpoint USER_DATA, butuh key) → **belum dicek**; simbol dengan max leverage < 25x atau bracket pertama < $50 notional ditolak saat startup live.
- VG-05 Biaya: dengan taker 0,05%/sisi 🔺 (cek tier akun), biaya round-trip ≈ 0,10% notional = $0,025–$0,05 per trade = 2,5–5% dari risk $1.

## 4. Hasil sekarang: **31 simbol lolos** (dari 87 yang lolos V-01+V-02)

Hitungan gagal per aturan (dari 516; satu simbol bisa gagal beberapa): V-06_minNotional 1, V-07_step 6, V-03_spread 262, V-05_tick 112, V-04_depth 477, V-01_qv 429, V-02_trades 341, V-09_funding 6

| # | Simbol | QV 24h ($M) | Trades 24h | Spread (bps) | Depth ±0,1% min ($) | Tick (bps) | minNotional ($) | Step×harga ($) | Umur (hari) | Funding (%) |
|---|---|---:|---:|---:|---:|---:|---:|---:|---:|---:|
| 1 | ETHUSDT | 12,714.0 | 6,098,664 | 0.04 | 3,653,019 | 0.04 | 20 | 2.417 | 2534 | +0.0100 |
| 2 | ZECUSDT | 2,886.0 | 7,349,200 | 0.08 | 355,472 | 0.08 | 5 | 1.281 | 2464 | +0.0100 |
| 3 | SOLUSDT | 2,111.6 | 2,507,055 | 1.05 | 1,908,390 | 1.05 | 5 | 0.950 | 2242 | +0.0100 |
| 4 | HYPEUSDT | 1,151.8 | 2,733,479 | 0.12 | 413,996 | 0.12 | 5 | 0.844 | 523 | +0.0050 |
| 5 | XRPUSDT | 934.1 | 1,777,695 | 0.77 | 1,106,126 | 0.77 | 5 | 0.130 | 2494 | +0.0100 |
| 6 | NEARUSDT | 733.9 | 1,556,408 | 2.48 | 235,073 | 2.48 | 5 | 4.031 | 2211 | +0.0100 |
| 7 | 1000PEPEUSDT | 485.5 | 2,800,379 | 0.20 | 50,910 | 0.20 | 5 | 0.005 | 1279 | +0.0100 |
| 8 | DOGEUSDT | 440.4 | 1,071,408 | 1.31 | 467,037 | 1.31 | 5 | 0.077 | 2308 | +0.0100 |
| 9 | ENAUSDT | 377.0 | 1,812,408 | 0.55 | 68,502 | 0.55 | 5 | 0.181 | 946 | +0.0050 |
| 10 | WLDUSDT | 262.3 | 891,860 | 2.66 | 76,323 | 2.66 | 5 | 0.377 | 1199 | -0.0353 |
| 11 | SUIUSDT | 247.6 | 976,570 | 1.08 | 180,741 | 1.08 | 5 | 0.093 | 1281 | +0.0100 |
| 12 | LINKUSDT | 201.2 | 662,002 | 0.87 | 131,104 | 0.87 | 20 | 0.115 | 2483 | +0.0100 |
| 13 | PUMPUSDT | 174.3 | 878,683 | 2.40 | 49,061 | 2.40 | 5 | 0.004 | 482 | +0.0050 |
| 14 | TAOUSDT | 114.5 | 684,925 | 0.43 | 113,271 | 0.43 | 5 | 0.234 | 936 | +0.0050 |
| 15 | BCHUSDT | 96.1 | 514,592 | 0.36 | 64,241 | 0.36 | 20 | 0.280 | 2512 | +0.0088 |
| 16 | XMRUSDT | 91.5 | 662,484 | 0.16 | 37,772 | 0.16 | 5 | 0.638 | 2466 | +0.0156 |
| 17 | FILUSDT | 85.5 | 570,318 | 1.17 | 105,699 | 1.17 | 5 | 0.086 | 2210 | +0.0100 |
| 18 | LTCUSDT | 81.8 | 325,895 | 1.79 | 108,405 | 1.79 | 20 | 0.056 | 2491 | +0.0100 |
| 19 | ARBUSDT | 79.6 | 707,576 | 0.62 | 34,363 | 0.62 | 5 | 0.016 | 1322 | +0.0100 |
| 20 | INJUSDT | 79.5 | 665,896 | 1.27 | 26,952 | 1.27 | 5 | 0.786 | 1541 | +0.0100 |
| 21 | ONDOUSDT | 72.9 | 375,264 | 2.45 | 40,354 | 2.45 | 5 | 0.041 | 1019 | +0.0050 |
| 22 | TRXUSDT | 68.0 | 392,826 | 0.31 | 128,905 | 0.31 | 5 | 0.320 | 2485 | +0.0000 |
| 23 | PENGUUSDT | 64.2 | 524,133 | 1.37 | 40,269 | 1.37 | 5 | 0.007 | 686 | +0.0050 |
| 24 | DOTUSDT | 55.8 | 550,684 | 1.05 | 34,546 | 1.05 | 5 | 0.096 | 2265 | -0.0049 |
| 25 | XLMUSDT | 55.5 | 451,126 | 0.60 | 86,852 | 0.60 | 5 | 0.167 | 2480 | +0.0100 |
| 26 | ASTERUSDT | 37.3 | 206,967 | 1.42 | 46,692 | 1.42 | 5 | 0.705 | 411 | +0.0050 |
| 27 | HBARUSDT | 35.6 | 279,116 | 1.39 | 46,024 | 1.39 | 5 | 0.072 | 2058 | -0.0002 |
| 28 | OPUSDT | 32.6 | 440,804 | 0.91 | 27,915 | 0.91 | 5 | 0.011 | 1617 | +0.0100 |
| 29 | 1000SHIBUSDT | 31.8 | 200,181 | 2.12 | 74,951 | 2.12 | 5 | 0.005 | 2004 | +0.0100 |
| 30 | ETCUSDT | 24.9 | 148,232 | 1.37 | 46,795 | 1.37 | 20 | 0.073 | 2484 | +0.0100 |
| 31 | SANDUSDT | 20.4 | 210,136 | 2.19 | 31,602 | 2.19 | 5 | 0.046 | 2116 | -0.0109 |

### Hampir lolos (lolos V-01+V-02 tapi gagal lainnya)
| Simbol | Gagal di | QV ($M) | Spread (bps) | Depth min ($) | minQty×harga ($) |
|---|---|---:|---:|---:|---:|
| BTCUSDT | V-06_minNotional, V-07_step | 18,948.4 | 0.01 | 2,079,903 | 80.2 |
| ADAUSDT | V-03_spread, V-05_tick | 176.2 | 5.11 | 148,056 | 0.2 |
| DASHUSDT | V-04_depth | 28.2 | 2.07 | 12,962 | 0.05 |
| BNBUSDT | V-07_step | 519.0 | 0.14 | 635,832 | 7.17 |
| CRVUSDT | V-04_depth | 33.6 | 3.65 | 19,782 | 0.03 |
| RUNEUSDT | V-04_depth | 43.8 | 3.12 | 6,447 | 0.64 |
| UNIUSDT | V-07_step | 353.0 | 1.31 | 126,291 | 7.63 |
| AVAXUSDT | V-07_step | 143.6 | 1.14 | 65,465 | 8.81 |
| AAVEUSDT | V-07_step | 117.5 | 0.76 | 97,105 | 13.18 |
| HOTUSDT | V-04_depth | 221.1 | 2.14 | 5,702 | 0.0 |
| APTUSDT | V-04_depth | 44.5 | 1.58 | 18,065 | 0.06 |
| QNTUSDT | V-07_step | 68.7 | 0.49 | 27,744 | 20.35 |
| FETUSDT | V-03_spread, V-05_tick | 35.4 | 6.22 | 25,258 | 0.16 |
| STXUSDT | V-04_depth | 29.9 | 2.81 | 5,743 | 0.36 |
| TIAUSDT | V-04_depth | 40.9 | 2.22 | 16,504 | 0.45 |
| BEAMXUSDT | V-04_depth | 54.2 | 4.49 | 2,629 | 0.0 |
| 1000BONKUSDT | V-04_depth | 27.3 | 3.28 | 9,887 | 0.0 |
| LSKUSDT | V-04_depth, V-09_funding | 69.0 | 1.51 | 2,773 | 0.2 |
| STRKUSDT | V-04_depth | 37.8 | 2.32 | 14,011 | 0.0 |
| ETHFIUSDT | V-04_depth | 24.4 | 1.69 | 10,019 | 0.06 |
| ZROUSDT | V-04_depth | 68.9 | 0.56 | 8,857 | 0.18 |
| AEROUSDT | V-04_depth | 29.2 | 1.56 | 7,790 | 0.06 |
| FARTCOINUSDT | V-03_spread, V-04_depth, V-05_tick | 34.5 | 8.0 | 1,370 | 0.01 |
| TRUMPUSDT | V-03_spread, V-04_depth, V-05_tick | 284.4 | 7.74 | 6,744 | 0.01 |
| VVVUSDT | V-04_depth | 58.0 | 1.36 | 13,506 | 0.22 |
| BMTUSDT | V-04_depth, V-09_funding | 181.7 | 3.52 | 5,289 | 0.03 |
| INITUSDT | V-04_depth | 20.2 | 2.06 | 1,022 | 0.15 |
| HAEDALUSDT | V-04_depth | 20.2 | 3.55 | 346 | 0.03 |
| AGTUSDT | V-04_depth | 90.1 | 3.11 | 508 | 0.03 |
| HUMAUSDT | V-04_depth, V-09_funding | 52.7 | 4.7 | 1,600 | 0.02 |
| ICNTUSDT | V-04_depth | 39.6 | 4.76 | 886 | 0.21 |
| IDOLUSDT | V-04_depth | 117.9 | 2.19 | 2,285 | 0.02 |
| AINUSDT | V-04_depth | 76.5 | 4.87 | 2,486 | 0.31 |
| ESPORTSUSDT | V-04_depth | 42.7 | 0.91 | 3,041 | 0.01 |
| AIOUSDT | V-03_spread, V-04_depth | 46.2 | 8.41 | 317 | 0.04 |
| USELESSUSDT | V-04_depth | 49.7 | 4.98 | 4,955 | 0.14 |
| XPLUSDT | V-04_depth | 43.4 | 1.56 | 12,378 | 0.06 |
| WLFIUSDT | V-04_depth | 115.8 | 2.12 | 4,510 | 0.05 |
| QUSDT | V-03_spread, V-04_depth | 31.9 | 6.79 | 744 | 0.03 |
| ARIAUSDT | V-04_depth | 41.9 | 3.16 | 1,535 | 0.13 |
| FLUIDUSDT | V-04_depth | 22.9 | 4.16 | 1,280 | 0.24 |
| AKEUSDT | V-03_spread, V-04_depth | 168.9 | 7.62 | 1,098 | 0.0 |
| EVAAUSDT | V-03_spread, V-04_depth | 77.1 | 6.18 | 2,215 | 0.16 |
| LYNUSDT | V-03_spread, V-04_depth, V-05_tick | 45.8 | 5.72 | 485 | 0.02 |
| APRUSDT | V-04_depth | 28.0 | 2.4 | 2,468 | 0.04 |
| UAIUSDT | V-03_spread, V-04_depth, V-05_tick | 38.8 | 12.08 | 359 | 0.08 |
| JCTUSDT | V-03_spread, V-04_depth | 163.3 | 5.53 | 4,125 | 0.01 |
| CLANKERUSDT | V-03_spread, V-04_depth | 353.2 | 12.49 | 2,799 | 0.24 |
| LITUSDT | V-04_depth | 107.4 | 0.87 | 4,307 | 0.35 |
| ELSAUSDT | V-03_spread, V-04_depth | 20.6 | 7.24 | 5,817 | 0.08 |
| BIRBUSDT | V-04_depth | 48.7 | 1.15 | 1,774 | 0.09 |
| BTWUSDT | V-04_depth | 60.7 | 0.95 | 2,919 | 1.06 |
| ARXUSDT | V-03_spread, V-04_depth | 28.2 | 8.1 | 595 | 0.25 |
| CAPUSDT | V-04_depth | 70.9 | 2.16 | 2,399 | 0.09 |
| 牛来USDT | V-04_depth | 28.4 | 3.14 | 0 | 0.06 |
| PONSUSDT | V-04_depth | 34.3 | 4.48 | 1,040 | 0.22 |

Temuan penting:
- G-01 **BTCUSDT tidak bisa dieksekusi** dengan aturan user: minNotional $50 dan minQty 0,001 × harga ≈ $80 → butuh SL ≤ 1,25% untuk risk $1. BTC jadi pembaca saja [→ MARKET_READER.md §4 A-04].
- G-02 BNBUSDT, UNIUSDT, AVAXUSDT, AAVEUSDT, QNTUSDT gagal V-07 (step $7–$20): pembulatan bisa memotong risk di bawah $0,80.
- G-03 ADAUSDT gagal V-03/V-05: tick 0,0001 pada harga ≈ $0,2 → spread minimal 5,1 bps.

## 5. Validasi threshold
| ID | Uji | Lolos jika |
|---|---|---|
| VG-V1 | Rekam bookTicker/depth kandidat 14 hari (WS) → distribusi spread & depth per jam (WIB) | V-03/V-04 stabil: simbol lolos ≥ 80% jam aktif |
| VG-V2 | Backtest strategi [→ STRATEGIES.md] per bucket likuiditas (V-01 $5–20M vs $20–100M vs > $100M) dengan slippage = spread median + 1 tick | expectancy bucket ≥ $20M ≥ bucket $5–20M; kalau tidak, V-01 diturunkan |
| VG-V3 | Universe point-in-time dari klines harian data.binance.vision (quoteVolume, count) | universe historis tersimpan per tanggal [→ MARKET_READER.md §7 VAL-02] |
| VG-V4 | Paper/shadow: slippage nyata entry & SL vs harga sinyal | median slippage ≤ 2 bps, p95 ≤ 10 bps |

## 6. Asumsi
- A-10 Response `exchangeInfo` dari CDN memuat `serverTime` basi (01:01:01Z, ±18 jam sebelum fetch). Umur listing dihitung dari serverTime itu (error ±1 hari, tidak berpengaruh ke V-08 14 hari). Jam acuan live wajib `/fapi/v1/time`.
- A-11 🔺 Depth & spread = 1 snapshot (02:12 WIB). Jumlah 31 bisa berubah ±beberapa simbol per jam.
