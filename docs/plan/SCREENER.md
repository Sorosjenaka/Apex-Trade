# SCREENER — Penyaring Pasar Trending/Ekspansi (Apex Scalper RS)

> Versi 1.0 · 5 Nov 2026 (WIB). Tujuan tunggal: **hanya mengizinkan entry pada simbol yang sedang trending atau ekspansi, dan memblokir pasar sideways/ranging**. Semua ambang di bawah adalah **nilai awal (hipotesis)**. Nilai final dikunci setelah Fase 2 dengan uji plateau (Rules.md §4). Bila berubah, ambang di sini adalah SSoT dan `config/screener.toml` wajib mengikutinya.

> Catatan integrasi: ID komponen regime per-simbol di dokumen ini memakai prefiks `RC-x` (sebelumnya `RG-x`) agar tidak bentrok dengan state regime global `RG-0..RG-10` di MARKET_READER.md. Lihat §9 untuk pembagian SSoT.

## 1. Alur

```
setiap 1h close (+5 dtk)          setiap 15 menit                    setiap sinyal entry
┌─────────────────────────┐   ┌──────────────────────────────┐   ┌───────────────────────────┐
│ L1 Kelayakan (statis)    │→ │ L2 Likuiditas & Biaya         │→ │ L4 Cek ulang per trade     │
│ L3 Regime (1h + 4h)      │   │ (vol 24j, spread, funding)    │   │ (sizing, spread live, state)│
└─────────────────────────┘   └──────────────────────────────┘   └───────────────────────────┘
          ↓ state per simbol: TREND_UP | TREND_DOWN | RANGING | BLOCKED
```

Semua perhitungan hanya memakai **bar yang sudah close** (`close_time < now`). Bar 4h dibentuk dari 4 bar 1h tertutup yang selaras dengan 00:00 UTC (= 07:00 WIB).

## 2. Lapisan L1 — Kelayakan instrumen (refresh: start + tiap 60 menit)

| ID | Syarat | Sumber | Lulus jika |
|---|---|---|---|
| SC-01 | Kontrak | exchangeInfo | `contractType = PERPETUAL`, `quoteAsset = USDT`, `status = TRADING` |
| SC-02 | Leverage | bracket-1 | `initialLeverage` maks ≥ 25 |
| SC-03 | Sizing | filter LOT_SIZE / MIN_NOTIONAL | untuk SL 2,0/2,5/3,0/3,5/4,0%: qty ≥ minQty, notional ≥ minNotional, risk aktual ≥ $0,80 di **≥ 4 dari 5** grid SL (cek final tetap per trade di L4) |
| SC-04 | Umur listing | `onboardDate` | ≥ 30 hari |
| SC-05 | Bukan aset non-kripto | daftar manual | tidak masuk `config/blocklist.toml` (awal: PAXGUSDT, XAUUSDT, XAGUSDT, saham tokenisasi) |
| SC-06 | Data cukup | klines lokal | ≥ 500 bar 1h tersedia tanpa celah > 1 bar |

## 3. Lapisan L2 — Likuiditas dan biaya (refresh: tiap 15 menit)

| ID | Syarat | Lulus jika | Alasan |
|---|---|---|---|
| SC-10 | Volume 24 jam (quote) | ≥ $100 juta | Slippage stop market terkendali pada notional ≤ $47,2 |
| SC-11 | Spread median 15 menit (bookTicker, sampel tiap 10 dtk) | ≤ 3,0 bps | Spread 3 bps ≈ 0,014R pada SL 2%. Di snapshot, CLANKER 8,43, TRUMP 7,72, ADA 5,12 bps → diblok |
| SC-12 | Funding terakhir | \|funding\| ≤ 0,05% per 8 jam | Funding ekstrem = crowding; biaya 0,024R per 8 jam pada SL 2% |
| SC-13 | Perubahan 24 jam | \|chg24\| ≤ 25% | Hindari pump/dump listing baru (contoh CLANKER +103%) |
| SC-14 | Mark vs last | \|mark − last\| / last ≤ 0,15% | Trigger SL memakai MARK_PRICE; deviasi besar → trigger tak terduga |

## 4. Lapisan L3 — Regime (inti anti-sideways; refresh tiap 1h close)

Dihitung pada **1h** (utama) dan **4h** (konfirmasi). Setiap komponen memberi suara arah (+1/−1/0) atau lulus/gagal.

| ID | Komponen (asal skrip) | Rumus ringkas | Ambang TREND | Ambang RANGING (blok) |
|---|---|---|---|---|
| RC-1 | ADX14 Wilder (apex `indicators.rs::adx_wilder`) | standar Wilder | ADX ≥ 25 dan naik vs 3 bar lalu, atau ADX ≥ 30 | ADX < 20 |
| RC-2 | Efficiency Ratio Kaufman, n=20 | \|C_t − C_{t−20}\| / Σ\|C_i − C_{i−1}\| | ER ≥ 0,35 | ER < 0,25 |
| RC-3 | Choppiness Index 14 | 100·log10(ΣATR1 / (HH−LL)) / log10(14) | CHOP ≤ 45 | CHOP ≥ 55 |
| RC-4 | Signed Realized Variance (ylUDT58m), len 30 | EMA(sign(r)·r²)/EMA(r²) | \|SRV\| ≥ 0,35; tanda = arah | \|SRV\| < 0,20 |
| RC-5 | Bilateral Filter Trend (yY27v3eD), preset Smooth 60/12/1,5/1,75 | state tren kausal | state = arah suara | — (hanya arah) |
| RC-6 | Decycler HL (dOcUNfyv) | close di luar kanal decycler + slope | state = arah suara | — |
| RC-7 | MTF Trend Matrix (NjcGEJ94), versi bar tertutup: 15m/1h/4h | EMA21 vs EMA55 dan close vs EMA21 | ≥ 2 dari 3 TF searah | 3 TF campur (tanpa mayoritas) |
| RC-8 | Ekspansi volatilitas | ATR14%(1h) / median ATR14%(1h, 30 hari) | rasio ≥ 1,0 (normal/ekspansi) | rasio < 0,7 (kompresi) |
| RC-9 | Rentang ATR absolut | ATR14% 1h | 0,6% ≤ ATR% ≤ 3,0% | ATR% < 0,6% (SL 2% > 3,3 ATR 1h) |
| RC-10 | Konteks BTC | RC-4 pada BTCUSDT 1h | bila \|SRV_BTC\| ≥ 0,5 dan berlawanan arah → blok sisi itu | — |

### 4.1 Keputusan state (deterministik)

```
dir_votes   = sign(RC-4) + state(RC-5) + state(RC-6) + majority(RC-7)      # ∈ [-4, 4]
trend_ok    = RC-1 lulus AND (RC-2 lulus OR RC-3 lulus) AND RC-8 lulus AND RC-9 lulus
range_flag  = jumlah kondisi RANGING terpenuhi di {RC-1, RC-2, RC-3, RC-4, RC-7, RC-8} ≥ 2

if L1 gagal or L2 gagal:              state = BLOCKED(reason)
elif range_flag:                      state = RANGING
elif trend_ok and dir_votes ≥ +3 and 4h: sign(SRV_4h) ≥ 0:  state = TREND_UP
elif trend_ok and dir_votes ≤ −3 and 4h: sign(SRV_4h) ≤ 0:  state = TREND_DOWN
else:                                 state = RANGING   # default aman = tidak trading
```

**Histeresis** (mencegah flip tiap jam):
- Masuk TREND_* butuh syarat terpenuhi pada **2 bar 1h tertutup berturut-turut**.
- Keluar ke RANGING bila `range_flag` terpenuhi pada **1 bar**. Keluar butuh 1 bar, masuk butuh 2 bar.

**Arah:** TREND_UP hanya mengizinkan sinyal LONG. TREND_DOWN hanya mengizinkan SHORT. RANGING/BLOCKED tidak mengizinkan entry baru. Posisi yang sudah terbuka tidak ditutup oleh perubahan state; posisi dikelola SL/TP di exchange.

## 5. Lapisan L4 — Cek per sinyal (saat entry)

1. State simbol masih TREND searah sinyal (dibaca dari snapshot terakhir, umur ≤ 65 menit).
2. Spread live (bookTicker WS) ≤ 3,0 bps.
3. Sizing: SL% ∈ [2,00; 4,00], risk aktual ≥ $0,80, notional ≥ minNotional, qty ≥ minQty.
4. Global risk gate: posisi+pending < 2, tidak dalam cooldown, stop harian belum tercapai, ARM = true (Rules.md).

## 6. Output (Schema.md tabel `screener_snapshots`)

Per simbol per jam: `ts_utc, symbol, state, dir_votes, adx, er, chop, srv_1h, srv_4h, bft_state, decycler_state, mtf_score, atr_pct_1h, atr_ratio, qv24_musd, spread_bps_med, funding_pct, chg24_pct, reasons[]`. Endpoint `GET /api/screener` mengembalikan snapshot terakhir. Watchlist = simbol berstate TREND_* diurutkan dari `|dir_votes|`, lalu ER, maksimum **8 simbol**.

## 7. Validasi screener (wajib sebelum dipakai)

- **Uji nilai tambah:** jalankan setiap strategi dengan dan tanpa filter regime pada IS. Screener dipertahankan hanya bila ekspektansi neto per trade dengan filter ≥ tanpa filter + 0,05R dan jumlah trade ≥ 50% versi tanpa filter.
- **Uji plateau:** ADX 22/25/28, ER 0,30/0,35/0,40, CHOP 40/45/50. Ekspektansi di ≥ 80% kombinasi tetangga harus > 0.
- **Uji kausal:** state pada jam t dari data terpotong di t harus identik dengan state dari data penuh (prefix test, pola `tests/scalp_causality.rs`).
- Frekuensi state TREND_* per simbol dilaporkan (persen jam). Bila < 10% jam, simbol dihapus dari universe karena terlalu jarang trending untuk bernilai.

## 8. Universe awal (dari FEASIBILITY.md §2, snapshot 2026-11-04T18:58Z)

ETHUSDT, SOLUSDT, XRPUSDT, DOGEUSDT, HYPEUSDT, SUIUSDT, LINKUSDT, ENAUSDT, ZECUSDT, NEARUSDT, 1000PEPEUSDT, WLDUSDT. BNBUSDT dan AVAXUSDT bersyarat (L4 sizing). BTCUSDT hanya konteks (RC-10).

## 9. Integrasi dengan BLACKLIST.md, VOLUME_GATE.md, MARKET_READER.md

Tiga dokumen ini ditulis terpisah (5 Nov 2026 02:04–02:16 WIB, snapshot fapi 2026-11-04T19:04:37Z) dan berada di folder yang sama. Pembagian SSoT:

| Topik | SSoT | Efek pada dokumen ini |
|---|---|---|
| Aset non-kripto (saham, ETF, komoditas, forex, indeks, stablecoin) | BLACKLIST.md (`blacklist.json`) | SC-05 diganti: simbol wajib ada di allowlist BLACKLIST.md. `config/blocklist.toml` tidak dipakai |
| Ambang volume, jumlah trade, spread, depth, tick, umur listing, funding | VOLUME_GATE.md (V-01..V-09, VG-01..VG-05) | SC-04, SC-10, SC-11, SC-12 tidak berlaku sendiri; yang berlaku V-08, V-01/V-02, V-03, V-09. Bila angka di §2–§3 berbeda dengan VOLUME_GATE.md, VOLUME_GATE.md yang menang |
| Regime pasar global (BTC, dominasi, breadth) dan batas sisi/jumlah posisi | MARKET_READER.md (RG-0..RG-10, GT-01..GT-06) | Dijalankan SEBELUM L3. L3 (RC-1..RC-10) tetap wajib per simbol (GT-05). RC-10 boleh dihapus bila MARKET_READER aktif, karena konteks BTC sudah ditangani di sana |
| Tren per simbol (RC-1..RC-9), L4 per trade, sizing | Dokumen ini + Rules.md §4.1 | Tidak berubah |

Urutan final: `allowlist (BLACKLIST)` ∩ `universe_pass (VOLUME_GATE)` ∩ `regime.universe (MARKET_READER)` → SC-01/SC-02/SC-03/SC-06/SC-13/SC-14 → L3 RC-x → L4.

**Isi celah VG-04 (leverage belum dicek di VOLUME_GATE.md):** bracket-1 dari endpoint publik `https://www.binance.com/bapi/futures/v1/friendly/future/common/brackets` (snapshot `binance/lev2.json`, diambil 2026-11-04T18:58Z = 5 Nov 01:58 WIB) menunjukkan **31 dari 31** simbol lolos VOLUME_GATE.md §4 mengizinkan leverage ≥ 25x (ETH 150x, SOL dan XRP 100x, lainnya 75x). Endpoint ini bukan API resmi; cek final tetap via `/fapi/v1/leverageBracket` saat startup testnet (T-3.1).

**Cek SC-03 pada 31 simbol tersebut** (harga dari `universe_now.json`, filter dari `binance/exinfo.json`, fee taker/taker, grid SL 2,0/2,5/3,0/3,5/4,0%): semua lolos minQty dan minNotional di kelima titik grid. Risk aktual terendah per simbol ada di rentang $0,875 (NEARUSDT, step×harga $4,03) sampai $1,000. Tidak ada yang di bawah $0,80, tetapi NEARUSDT dan ETHUSDT ($0,949) dan ZECUSDT ($0,949) kehilangan ≥ 5% dari target $1 karena pembulatan.

Universe kerja = 31 simbol VOLUME_GATE.md §4, menggantikan daftar 12 simbol di §8 (§8 tetap dicatat sebagai snapshot lama dengan filter volume ≥ $100 juta).
