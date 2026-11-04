# MARKET_READER.md — Tim Market Reader (Apex Scalper RS)

> Versi 0.1 · dibuat 2026-11-05 02:04 WIB (snapshot data 2026-11-04T19:04:37Z UTC).
> SSoT untuk: anggota tim reader, kontrak output reader, state machine regime, aturan gating regime → screener/strategi.
> BUKAN SSoT untuk: aturan risk ($1, 25x cross, 2 posisi, 6 loss/hari, cooldown 1 jam) [→ PRD.md §5 Functional Requirements], blacklist [→ BLACKLIST.md], volume gate [→ VOLUME_GATE.md], screener per-simbol [→ SCREENER.md], strategi [→ STRATEGIES.md].

## 0. Ringkasan
- Tim reader = 9 modul Rust, masing-masing SATU tugas, output JSON berstempel waktu. Satu aggregator mengubahnya jadi 1 dari 6 state regime.
- Regime hanya boleh **mengurangi** izin (sisi, universe, jumlah posisi). Regime TIDAK PERNAH menaikkan risk di atas $1/trade atau menambah posisi di atas 2.
- Data hilang/basi = `NO_TRADE`, bukan netral.
- Semua threshold di dokumen ini = hipotesis awal (tanda 🔺) sampai lolos validasi §7.

## 1. Temuan dari reader lama (apex) — apa yang SUDAH vs BELUM tervalidasi

Sumber: `docs/MARKET_READER_V3_TEST_REPORT.md`, `market_reader_v3_validation/validation_summary.json`, `event_study.csv`, `rust-engine/src/macro_reader.rs`, `scripts/market_context_v3.py`, `data/vike_client.py`, `scripts/vike_openapi.json`.

| ID | Item | Status | Bukti |
|---|---|---|---|
| F-01 | Rumus RSI Wilder, EMA, MACD, AO, pivot divergence, Fibonacci leg | ✅ benar secara teknis | 14 unittest lulus, 0 fail, 0 error |
| F-02 | Tanpa look-ahead (prefix replay, future-mutation, alignment `available_at`) | ✅ | 32 prefix replay, 8 future-mutation, 128.968 baris dicek, 0 pelanggaran |
| F-03 | Data uji | ⚠ Binance **Spot**, bukan Futures; BTC/ETH 1h/2h/4h/1d; 58.422 candle tertutup; 2025-01-01 s/d 2026-11-03 (1d dari 2024-02-07) | data_manifest.json |
| F-04 | Edge prediktif divergence/Fib/RSI-extreme | ❌ TIDAK terbukti | event study BTC 1h 24 jam: arah sesuai 48–55%, tanda return berbalik antar tahun (mis. hidden_bear +0,30% 2025 vs −0,08% 2026). Laporan sendiri: "tidak ada klaim edge prediktif" |
| F-05 | Makro BTC.D / TOTAL / TOTAL2 / TOTAL3 | ❌ belum diuji sama sekali | 16 kombinasi seri×TF "missing", status `technical_tests_passed_macro_history_not_tested` |
| F-06 | Label makro di `macro_reader.rs` (`RELATIVE_ALT_STRENGTH_CANDIDATE`, `BTC_LED_APPRECIATION_CANDIDATE`, `BROAD_WEAKNESS_CANDIDATE`, `MIXED`) | ⚠ logika ada, tanpa data historis, tanpa threshold (cuma tanda slope >0/<0) | macro_reader.rs |
| F-07 | Skor `rev_up/rev_down` + `market_gate_1h(block_th=40)` | ❌ heuristik, bobot manual (10/20/30/30, +12, +10, +8), belum divalidasi | market_context_v3.py |
| F-08 | Paritas dengan TradingView runtime | ❌ unverified | test report §6 |
| F-09 | Vike API | ⚠ OpenAPI berisi Hyperliquid (positions, OI snapshot, liquidations, liquidation heatmap, ohlcv), HL cohorts, on-chain wallet/token, Polymarket. **Tidak ada endpoint BTC.D/TOTAL1-3.** Dari box ini `GET /v1/hyperliquid/stats` tanpa kredensial → 401 (sengaja tanpa key); dari VPS user → Cloudflare 403 (laporan user) | vike_openapi.json, data/pub_data.vike.io_* |
| F-10 | Data futures 5m/15m (tv7) | ✅ tersedia untuk BTCUSDT & ETHUSDT saja, 2025-01-01 s/d 2026-11-04 14:25 UTC, sumber data.binance.vision (sha256 tercatat) + 2.111 event funding BTC | mrdata/manifest.json |

Kesimpulan: yang bisa di-port apa adanya = **mesin indikator + kontrak waktu** (F-01, F-02). Yang harus dibuktikan ulang = semua logika regime/makro (F-04…F-07).

## 2. Peta sumber data (siapa memberi makan sinyal apa)

Semua sumber publik, tanpa API key. Snapshot bukti ada di `/workspace/scalp/data/` (fetch 2026-11-04T19:04:37Z).

| ID | Seri | Sumber UTAMA (live + histori) | Cadangan | Status cek |
|---|---|---|---|---|
| S-01 | BTCUSDT, ETHUSDT OHLCV 5m/15m/1h/4h | `fapi/v1/klines` + WS `kline_5m`; histori `data.binance.vision/futures/um` | — | 200 OK |
| S-02 | BTC.D (+ETH.D) | **TradingView `CRYPTOCAP:BTC.D`** (ETH.D sama): live = scanner `POST scanner.tradingview.com/cfd/scan`; histori = chart WS anonim (`unauthorized_user_token`) | `BTCDOMUSDT` index Binance (klines 1h, histori sejak 2021-06-17) — BUKAN skala sama, pakai return saja (lihat §2.2) | 200 OK; histori 1D sejak 2014-04-01, 1h 1.556 bar |
| S-03 | TOTAL | **TradingView `CRYPTOCAP:TOTAL`** (scanner + chart WS) | `ALLUSDT` index Binance (klines 1h, listing 2025-08-06) | 200 OK; 1h 7.388 bar sejak 2026-01-01; 1D sejak 2014-03-01 |
| S-04 | TOTAL2, TOTAL3 | **TradingView `CRYPTOCAP:TOTAL2`, `CRYPTOCAP:TOTAL3`** | proksi: median return 24h basket alt S-01 (tanpa BTC/ETH) | 200 OK; 1h 1.556 bar sejak 2026-09-01; 1D sejak 2014-04-01 / 2017-07-14 |
| S-05 | USDT.D | **TradingView `CRYPTOCAP:USDT.D`** | CoinGecko `/global` `market_cap_percentage.usdt` (snapshot) | 200 OK; 1D sejak 2015-03-08 |
| S-06 | ETH/BTC | rasio close ETHUSDT/BTCUSDT dari S-01 (bukan pair ETHBTC) — sama seperti reader v3 | — | — |
| S-07 | Funding | `fapi/v1/premiumIndex` (semua simbol, 1 call), `fapi/v1/fundingRate` histori | — | 200 OK |
| S-08 | OI | `futures/data/openInterestHist` (period 5m/15m/1h, histori ±30 hari) | WS tidak ada; poll | 200 OK |
| S-09 | Long/short & taker | `futures/data/globalLongShortAccountRatio`, `topLongShortPositionRatio`, `takerlongshortRatio` | — | 200 OK |
| S-10 | Likuidasi | WS `!forceOrder@arr` (Binance) | Vike `/v1/hyperliquid/liquidations` (nonaktif sampai 403 beres) | belum diuji di box |
| S-11 | OTHERS.D | **TradingView `CRYPTOCAP:OTHERS.D`** (opsional, fitur MR-04) | — (tanpa cadangan → fitur diabaikan + flag) | 200 OK; 1D sejak 2014-03-01 |
| S-12 | HL cohorts / smart money | Vike `/v1/hl-cohorts/*` | — | nonaktif (403/401) |
| S-13 | Jam server | `fapi/v1/time` | — | — |

A-01 🔺 BTCDOMUSDT ≠ CRYPTOCAP:BTC.D (komponen & bobot beda). Dipakai karena satu-satunya seri dominance publik dengan histori bar 1h tanpa key. Wajib dinamai `BTCDOM_PROXY`, bukan BTC.D.
A-02 🔺 ALLUSDT ≠ CRYPTOCAP:TOTAL (cakupan = koin futures Binance). Nama `TOTAL_PROXY`.
A-03 Snapshot CoinGecko direkam tiap 10 menit ke DB lokal sejak hari 1, supaya setelah ≥ 60 hari ada histori milik sendiri (timestamp = field `updated_at`, bukan waktu fetch).

### 2.1 TradingView sebagai sumber primer (keputusan user 2026-11-05)

Diuji 2026-11-05 02:30 WIB, anonim, tanpa login/key. Real-time (bar 5m terakhir = bar berjalan, `update_mode=streaming`), tidak delay.

**Histori chart WS anonim** (1 request, `request_more_data` tidak menambah bar kecuali TOTAL). Inventori: `data/tradingview/history_inventory.json`.

| Simbol | 5m | 15m | 1h | 4h | 1D |
|---|---|---|---|---|---|
| BTC.D, ETH.D, USDT.D, OTHERS.D, TOTAL2, TOTAL3 | ≈2.827 bar sejak 2026-10-26 | ≈2.287 sejak 2026-10-12 | 1.556 sejak 2026-09-01 | 1.847 sejak 2026-01-01 | sejak 2014-04 (BTC.D/TOTAL2), 2017-07-14 (ETH.D/TOTAL3), 2015-03-08 (USDT.D), 2014-03 (OTHERS.D) |
| TOTAL | 6.859 sejak 2026-10-12 | 6.223 sejak 2026-09-01 | 7.388 sejak 2026-01-01 | 6.233 sejak 2024-01-01 | 4.629 sejak 2014-03-01 |

Konsekuensi: histori intraday TV hanya 1–10 bulan → **TV tidak cukup untuk IS 2025 (VAL-05)**. Rekam sendiri (DC-01) mulai hari ini; untuk 2025 pakai cadangan Binance (BTCDOMUSDT/ALLUSDT) + basket alt, lalu cek konsistensi di overlap.

**Kolom scanner live (MTF via sufiks `|interval`, terverifikasi):** `close, change, EMA20, EMA50, EMA200, RSI, ADX` (TF default 1D) dan `close|15, change|15, RSI|15, close|60, change|60, RSI|60, EMA50|60, change|240, RSI|240`, `update_mode`. Interval valid: 1, 5, 15, 30, 60, 120, 240, 1W, 1M.

### 2.2 TV BTC.D vs Binance BTCDOMUSDT (cadangan dikuantifikasi)

File: `data/tradingview/btcd_vs_btcdom.json`.

| TF | n | korelasi return | arah return sama | arah slope EMA50 sama | rasio level BTCDOM/BTC.D (CV) |
|---|---|---|---|---|---|
| 5m | 1.499 | 0,682 | 74,0% | 75,1% | 92,8 (0,33%) |
| 1h | 1.500 | 0,785 | 80,7% | 82,1% | 88,5 (2,7%) |
| 1d | 1.500 (sejak 2022-09-27) | 0,859 | 85,2% | 83,1% | 51,1 (37,5%) |

Aturan: level **tidak boleh** dipertukarkan (rasio berubah karena BTCDOM = BTC vs 20 alt, rebalancing). Cadangan hanya memakai return/slope. Saat MR-03 jalan di cadangan: `source=fallback`, `max_positions=1`, flag `dom_fallback=true` (≈18–20% bar 1h beda arah).

### 2.3 Aturan sumber, staleness, throttle

| ID | Aturan |
|---|---|
| TV-01 | Scanner: 1 request batch (7 simbol, semua kolom) per 60 s; minimum jeda 10 s antar request; timeout 10 s |
| TV-02 | Chart WS: maks 1 sesi per 3 s, maks 1 sesi paralel; hanya untuk backfill saat start & tiap 6 jam |
| TV-03 | Error HTTP 429/5xx atau timeout: backoff 30 s → 60 s → 120 s → 300 s (maks), reset setelah 1 sukses |
| TV-04 | Data TV dianggap STALE bila umur nilai > 180 s (scanner) atau bar 1h terakhir tutup > 75 menit lalu |
| TV-05 | Sanity: BTC.D ∈ [30, 80], USDT.D ∈ [1, 20], TOTAL ∈ [$0,5T, $10T], TOTAL3 < TOTAL2 < TOTAL; gagal → ERROR |
| TV-06 | Urutan: TV OK → pakai TV. TV STALE/ERROR → cadangan Binance (`source=fallback`, `max_positions=1`). Keduanya gagal → MR-03/MR-04 `Unavailable` → RD-00.3 `NO_TRADE` |
| TV-07 | Kembali ke TV hanya setelah 3 poll berturut-turut OK (hysteresis) |

### 2.4 Kolektor data (DC)

| ID | Komponen |
|---|---|
| DC-01 | Poll scanner 60 s → simpan baris `(ts_fetch_ms, symbol, close, change|15, change|60, RSI|60, EMA50|60, …)` ke SQLite `tv_snapshot` |
| DC-02 | Tiap 6 jam: backfill chart WS 5m/15m/1h/4h/1D untuk 7 simbol → upsert `tv_bar(symbol, tf, open_time)`; gap > 2 bar → log `TV_GAP` |
| DC-03 | Harian: ekspor CSV ke `data/tradingview/` dengan kolom `# fetched_utc` |
| DC-04 | Paralel: rekam BTCDOMUSDT/ALLUSDT 1h untuk overlap VAL-08 |

### 2.5 Risiko TradingView

| ID | Risiko | Mitigasi |
|---|---|---|
| RK-TV1 | Endpoint tidak resmi, tanpa SLA; format bisa berubah tanpa pemberitahuan | parser defensif + TV-05 + cadangan TV-06 |
| RK-TV2 | Rate-limit/blokir IP | TV-01…TV-03; tidak scraping massal |
| RK-TV3 | Syarat penggunaan TradingView melarang penggunaan data otomatis/komersial tanpa izin | pemakaian pribadi, volume rendah; user menerima risiko; jangan redistribusi data |
| RK-TV4 | Histori intraday pendek (1–10 bulan) | DC-01/DC-02 rekam sendiri sejak 2026-11-05 |

## 3. Anggota tim reader

Kontrak umum semua reader (RD-00):
```
ReaderOutput { reader_id, as_of_ms, inputs_available_at_ms{seri:ms}, status: OK|STALE|UNAVAILABLE|ERROR,
               values{...}, flags[...], source{seri:provider}, settings_version }
```
- RD-00.1 Hanya bar dengan `available_at = open_time + durasi_TF ≤ as_of` yang boleh dipakai.
- RD-00.2 `STALE` bila bar terakhir yang seharusnya sudah tersedia terlambat > 1 durasi TF (snapshot: > 2× cadence).
- RD-00.3 Status ≠ OK pada reader WAJIB (MR-01, MR-03, MR-04, MR-07, MR-08) → aggregator keluarkan `NO_TRADE`. Reader OPSIONAL (MR-05, MR-06, MR-09) gagal → fitur diabaikan + flag, regime tetap jalan tapi `max_positions=1`.

| ID | Reader | Satu tugas | Input / sumber | TF & cadence | Komputasi | Output kunci | Gagal → |
|---|---|---|---|---|---|---|---|
| MR-01 | BTC Trend Reader | arah & kualitas tren BTC | S-01 BTCUSDT | 15m & 1h, tiap close 5m | EMA21/50/200, ADX14 Wilder, ER20 = \|C−C₋₂₀\|/Σ\|ΔC\|, slope EMA50 24 bar, ATR14% & persentil ATR% 30 hari | `btc_trend∈{UP,DOWN,NONE}`, `adx_1h`, `er_15m`, `atr_pct_pctile_1h` | NO_TRADE |
| MR-02 | ETH & ETH/BTC Reader | arah ETH + kekuatan relatif | S-01, S-06 | 15m & 1h | sama MR-01 untuk ETH; `ethbtc_mom24_pct = R/R₋₂₄−1` (1h) | `eth_trend`, `ethbtc_mom24_pct` | flag, ETH dianggap NONE |
| MR-03 | Dominance Reader | arah dominance BTC | S-02 TV `BTC.D` (cadangan BTCDOMUSDT) | 1h, tiap close 1h; scanner poll 60 s | `dom_chg_24h_pct = P/P₋₂₄−1` di TV BTC.D (cadangan: BTCDOMUSDT, threshold sama karena berbasis return); `dom_slope = slope EMA20 1h` | `dom_dir∈{RISING,FALLING,FLAT}` (\|chg\| < 0,25% 🔺 = FLAT) | NO_TRADE |
| MR-04 | Total Cap Reader | arah kapitalisasi total & alt | S-03, S-04, S-05, S-11 (TV primer) | 1h; scanner poll 60 s | `total_chg_24h` di TV TOTAL (cadangan ALLUSDT); `alt_chg_24h` di TV TOTAL3 (cadangan: median return basket alt); `t2_chg_24h` TV TOTAL2; `usdt_d_chg_24h` TV USDT.D (cadangan CoinGecko snapshot); `others_d` opsional | `total_dir`, `alt_dir`, `usdt_d_dir` (FLAT bila \|chg\| < 0,5% 🔺) | NO_TRADE |
| MR-05 | Positioning Reader | crowding derivatif | S-07, S-08, S-09 untuk BTC, ETH, dan kandidat screener | 5m (OI/taker), funding tiap 8 jam/interval | `funding`, `oi_chg_1h_pct`, `ls_ratio`, `taker_ratio_1h` | flag `CROWDED_LONG` (funding > +0,05% DAN ls_ratio > 2,0 🔺), `CROWDED_SHORT` (funding < −0,03% DAN ls_ratio < 0,7 🔺), `OI_FLUSH` (oi_chg_1h < −5% 🔺) | abaikan + max_pos=1 |
| MR-06 | Liquidation Reader | deteksi kaskade likuidasi | S-10 | agregasi 1m & 5m | jumlah notional likuidasi semua simbol per 5m; z-score vs median 7 hari | flag `LIQ_CASCADE` (z ≥ 4 🔺) | abaikan + max_pos=1 |
| MR-07 | Breadth Reader | apakah pasar bergerak serempak | klines 1h semua simbol universe [→ VOLUME_GATE.md] | 1h | `breadth_up = %simbol close > EMA50 1h`; `trend_share = %simbol ER20_1h ≥ 0,35` | `breadth_up`, `trend_share` | NO_TRADE |
| MR-08 | Data & Clock Sentinel | kesehatan data | semua reader + S-13 | tiap 5 s | freshness tiap seri, gap, \|server−lokal\| | `data_ok` (semua wajib OK DAN drift < 1000 ms) | NO_TRADE |
| MR-09 | Reversal Context Reader (port v3) | konteks divergence/Fib multi-TF BTC & ETH | S-01 (futures, bukan spot) | 1h/2h/4h/1d, tiap close 1h | port `mtf_verdict_v3` → `rev_up`, `rev_down` (0–100) | skor + `stage` | abaikan |
| MR-10 | Shock Detector | proteksi spike | S-01 BTC 1m | tiap close 1m | \|ret 1m\| & \|ret 5m\| | flag `SHOCK` bila \|ret_5m BTC\| ≥ 1,5% 🔺 atau `LIQ_CASCADE` | — (selalu aktif, lokal) |

RD-01 MR-09 berstatus **SHADOW**: dihitung & dilog, bobot gating = 0 sampai lolos VAL-04 (karena F-04, F-07).
RD-02 MR-05/MR-06 berstatus **SHADOW** untuk izin entry di fase awal; hanya boleh menurunkan `max_positions` 2→1.
RD-03 Vike (S-12) tidak dipakai di jalur keputusan. Bila nanti 403 beres → masuk sebagai reader opsional baru (MR-11) dengan status SHADOW.

## 4. State machine regime (aggregator AG)

Dihitung tiap close bar 5m (as_of = close time bar 5m). Fitur 1h memakai bar 1h terakhir yang sudah tutup.

Definisi turunan:
- `CHOP` ⇔ `adx_1h < 18` DAN `er_15m < 0,25` 🔺 ATAU `trend_share < 20%` 🔺
- `EXPANDING` ⇔ `atr_pct_pctile_1h ≥ 40` 🔺 (volatilitas tidak sedang terkompres)

Urutan evaluasi (aturan pertama yang cocok menang):

| ID | State | Kondisi (semua harus benar) | Sisi | Universe | max_pos |
|---|---|---|---|---|---|
| RG-0 | `DATA_INVALID` | `data_ok=false` ATAU reader wajib ≠ OK | — | none | 0 |
| RG-1 | `SHOCK_HALT` | flag `SHOCK` aktif; bertahan 30 menit sejak flag terakhir | — | none | 0 |
| RG-2 | `CHOP_NO_TRADE` | `CHOP` ATAU `EXPANDING=false` ATAU `btc_trend=NONE` | — | none | 0 |
| RG-3 | `RISK_OFF` | `btc_trend=DOWN` DAN `total_dir≠RISING` DAN `breadth_up ≤ 40%` | SHORT only | semua lolos gate | 2 |
| RG-4 | `RISK_ON_ALTS` | `btc_trend=UP` DAN `dom_dir=FALLING` DAN `alt_dir=RISING` DAN `breadth_up ≥ 60%` | LONG only | semua lolos gate | 2 |
| RG-5 | `BTC_LED` | `btc_trend≠NONE` DAN (`dom_dir=RISING` ATAU kondisi RG-3/RG-4 tidak lengkap) | searah `btc_trend` | MAJORS = 10 simbol teratas quoteVolume dari universe lolos gate | 1 |

Catatan aturan:
- RG-6 Hysteresis: perpindahan ke state baru yang lebih longgar (RG-2→RG-3/4/5, RG-5→RG-3/4) butuh kondisi terpenuhi 2 close 15m berturut-turut; perpindahan ke state lebih ketat (→RG-0/1/2) berlaku segera.
- RG-7 Dwell minimum 30 menit di RG-3/4/5 sebelum boleh berganti antar-trend-state (bukan ke state ketat).
- RG-8 Override crowding: `CROWDED_LONG` → di RG-4 max_pos=1; `CROWDED_SHORT` → di RG-3 max_pos=1.
- RG-9 Posisi yang sudah terbuka TIDAK ditutup paksa oleh perubahan regime (SL/TP di exchange tetap berlaku [→ architecture.md §4 Alur Data Kritis]). Regime hanya mengatur entry baru.
- RG-10 Setiap perubahan state dilog dengan semua nilai input + `available_at` (audit/replay).
- A-04 🔺 BTCUSDT sendiri tidak bisa dieksekusi dengan aturan $1 risk (lihat VOLUME_GATE §3, minQty notional ≈ $80); BTC dipakai sebagai PEMBACA, bukan target.

## 5. Gating ke screener & strategi

| ID | Aturan (biner) |
|---|---|
| GT-01 | Screener [→ SCREENER.md] hanya menerima simbol di `allowlist` [→ BLACKLIST.md] ∩ `universe_pass` [→ VOLUME_GATE.md] ∩ `regime.universe`. |
| GT-02 | Sinyal strategi dengan sisi ∉ `regime.sides` → ditolak, alasan `REGIME_SIDE`. |
| GT-03 | Entry baru ditolak bila `open_positions ≥ min(2, regime.max_pos)`. |
| GT-04 | Regime TIDAK punya field ukuran; sizing selalu `notional = $1 / SL%` [→ PRD.md §5 Functional Requirements]. Unit test: untuk semua state, `risk_usd ≤ 1,00`. |
| GT-05 | Arah tren per simbol tetap wajib dari screener (ADX/ER simbol itu); regime = syarat perlu, bukan cukup. |
| GT-06 | Regime `DATA_INVALID/SHOCK_HALT/CHOP_NO_TRADE` → 0 entry baru, termasuk "sinyal kuat". |

## 6. Port dari kode apex

| ID | Dari | Ke (Rust) | Cara |
|---|---|---|---|
| PT-01 | `rust-engine/src/indicators.rs` (rsi_wilder, ema, macd, ao, atr_wilder, adx_wilder, slope_percent) | `crates/indicators` | salin + test paritas vs output Python v3 pada 8 dataset (toleransi 1e-9) |
| PT-02 | `macro_history.rs::validate_series` (gap count, non-finite) | `market_reader::data` | salin; tambah `available_at` & status STALE |
| PT-03 | `macro_reader.rs::macro_context` (4 label tanda-slope) | MR-03/MR-04 + AG | diganti threshold FLAT + hysteresis §4; label lama dipetakan: RELATIVE_ALT_STRENGTH→RISK_ON_ALTS, BTC_LED_APPRECIATION→BTC_LED, BROAD_WEAKNESS→RISK_OFF, MIXED→CHOP_NO_TRADE/BTC_LED |
| PT-04 | `market_context_v3.py::mtf_verdict_v3`, `market_gate_1h` | MR-09 (shadow) | port 1:1 lalu uji prefix-replay seperti F-02 |
| PT-05 | `validate_market_reader_v3.py` (prefix replay, future mutation) | `tests/no_lookahead.rs` | wajib untuk SEMUA reader baru |
| PT-06 | `vike_client.py` | tidak di-port sekarang | RD-03 |
| PT-07 | `agents/agent_hl_cohorts.py` (whale_dominance ±10/20, divergence ±30) | MR-11 kelak, shadow | butuh Vike |

## 7. Rencana validasi (wajib sebelum regime boleh gating live)

Data: futures 5m/15m/1h dari data.binance.vision, 2025-01-01 → terbaru; BTCDOMUSDT 1h; ALLUSDT 1h (sejak 2025-08-06); universe point-in-time.

| ID | Uji | Lolos jika |
|---|---|---|
| VAL-01 | No look-ahead: prefix replay ≥ 32 titik + future-mutation ≥ 8 per reader (format PT-05) | 0 pelanggaran |
| VAL-02 | Universe point-in-time: gate volume dihitung dari quoteVolume harian per tanggal (bukan daftar hari ini) — anti-survivorship | daftar per tanggal tersimpan & di-hash |
| VAL-03 | Event study regime: untuk tiap state, ukur 4 jam ke depan dari universe-nya: median ER20_15m, median \|ret\|/ATR | RG-3/4/5 > RG-2 pada ER median ≥ +0,05 di IS DAN OOS 🔺 |
| VAL-04 | Uji gating pada strategi [→ STRATEGIES.md]: PF & expectancy setelah biaya (taker 0,05%/sisi 🔺, slippage 1 tick) dengan vs tanpa gate | OOS: PF_gated − PF_ungated ≥ +0,10 DAN coverage (waktu tidak NO_TRADE) ≥ 25% DAN lebih baik dari 200 gate acak dengan coverage sama (p < 0,05) |
| VAL-05 | Split: IS 2025-01-01 → 2025-12-31, OOS 2026-01-01 → 2026-10-31; threshold 🔺 hanya di-tune di IS; OOS dibuka SEKALI [→ apex research/tv7_scalp_20261104/RESEARCH_RULES.md] | dicatat hash parameter sebelum buka OOS |
| VAL-06 | Stabilitas: geser semua threshold ±20% | VAL-04 tetap lolos di ≥ 70% kombinasi |
| VAL-07 | Shadow live ≥ 14 hari: regime dihitung live, dibandingkan replay offline dari data yang sama | kecocokan state ≥ 99% bar 5m |
| VAL-08 | Data dominance: TV BTC.D vs BTCDOMUSDT (overlap 1h sejak 2026-09-01, 1D sejak 2022-09) | korelasi return 1h ≥ 0,70 (hasil awal 0,785 ✅) dan 1d ≥ 0,80 (0,859 ✅); ulang tiap 30 hari |
| VAL-09 | MR-03/MR-04 di TV vs cadangan: replay 2026-09-01 → sekarang, label `dom_dir`/`alt_dir` | kecocokan label ≥ 80% bar 1h; kalau gagal → cadangan hanya boleh `max_positions=0` (NO_TRADE) |
| VAL-10 | Kontribusi BTC.D/TOTAL2/TOTAL3 1D (histori 2017→) ke regime: uji regime harian di IS 2018–2024, OOS 2025–2026 | regime RISK_ON_ALTS punya median return alt basket > regime RISK_OFF di OOS (bootstrap p < 0,05) |
| VAL-11 | Uptime TV ≥ 30 hari shadow | poll sukses ≥ 99%; fallback aktif < 2% waktu |

## 8. Asumsi
- A-01, A-02, A-03, A-04 (di atas).
- A-05 🔺 Semua threshold bertanda 🔺 = tebakan awal berbasis praktik umum, bukan hasil fit.
- A-06 🔺 Stream `!forceOrder@arr` Binance hanya mengirim snapshot likuidasi terakhir per simbol per 1000 ms → jumlah likuidasi under-count; MR-06 hanya untuk deteksi lonjakan relatif.
- A-07 Funding interval tiap simbol bisa berbeda (1/4/8 jam); funding dinormalisasi per 8 jam sebelum dibandingkan.
