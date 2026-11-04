# STRATEGIES — Spesifikasi Strategi (Apex Scalper RS)

> Versi 1.0 · 5 Nov 2026 (WIB).
> **Status semua strategi: HIPOTESIS.** Tidak ada satu pun yang diklaim profit sampai lolos seluruh gerbang validasi di Rules.md §4 (walk-forward, holdout tak tersentuh, stress biaya, Monte Carlo, Deflated Sharpe, plateau parameter, forward testnet).
> Sumber: 28 skrip TradingView di `pine/CATALOG.md`. Setiap strategi adalah **gabungan/modifikasi**, bukan replikasi 1:1. 1 skrip terkunci (SyWHPxSr) diimplementasi dari deskripsi publiknya saja.

## 0. Konvensi bersama (berlaku untuk S1–S5)

| Item | Aturan |
|---|---|
| Regime | Entry hanya bila SCREENER state = TREND_UP (long) / TREND_DOWN (short) pada simbol itu |
| Timeframe | **Struktur** dari 1h (dan 15m untuk S1/S2/S3). **Trigger** dari 15m atau 5m. Semua data hanya bar tertutup |
| SL | SL struktural = level invalidasi + buffer 0,15×ATR14(TF struktur). `SL% = |entry − SL| / entry`. Bila SL% < 2,00% atau > 4,00% → **SKIP** (SL tidak boleh digeser untuk memaksa masuk rentang) |
| Sizing | Rumus FEASIBILITY.md §1 (risk neto $1, qty dibulatkan ke bawah, margin = notional/25, SKIP bila risk aktual < $0,80) |
| TP | RR geometris tetap **2,0**: `TP = entry + side × 2 × |entry − SL|` (target harga 4–8%) |
| Manajemen | Baseline: tanpa trailing, tanpa BE, tanpa partial. Varian M1 (diuji terpisah, dihitung sebagai trial): pindah SL ke BE+fee setelah harga mark mencapai +1,2R. Varian M2: time-stop 48 jam |
| Entry | Default LIMIT GTX (post-only) dengan TTL 60 menit; bila tidak fill → batal, dicatat `EXPIRED`. S3 boleh MARKET (taker) dan biayanya dimodelkan taker+taker |
| Eksekusi SL/TP | `POST /fapi/v1/algoOrder` algoType=CONDITIONAL, STOP_MARKET & TAKE_PROFIT_MARKET, `closePosition=true`, `workingType=MARK_PRICE` |
| Delta | `delta = 2 × takerBuyBaseVolume − volume` dari klines (gratis, histori panjang). Footprint per level dari aggTrades (data.binance.vision) hanya untuk S2 |
| Arbitrase portofolio | Bila > 1 sinyal di bar yang sama dan slot tersisa < jumlah sinyal: prioritas = urutan peringkat validasi (ditetapkan di Fase 2, disimpan di `config/strategies.toml`), lalu ER screener tertinggi. Tidak ada pemilihan acak |
| Satu simbol | Maksimum 1 posisi/pending per simbol, lintas strategi |
| Anggaran trial | Setiap kombinasi parameter yang dijalankan = 1 trial di `trials_ledger` (dipakai Deflated Sharpe). Batas awal: ≤ 60 trial per strategi |

### Estimasi frekuensi (jujur)
Belum ada angka backtest. Yang diketahui dari FEASIBILITY §4: median waktu harga bergerak ±2% adalah 3–17 jam pada universe. Dengan screener memblok RANGING, perkiraan kasar = **0–2 setup per simbol per hari**, dibatasi oleh max 2 posisi. Angka pastinya wajib dilaporkan di Fase 2 sebagai `trades/hari` per strategi. Strategi dengan < 100 trade di IS dinyatakan **INCONCLUSIVE**.

---

## S1 — Trend FVG Pullback (TFP)

**Sumber:**
- FVG Mitigation & Target Zones [PhenLabs] (OLgiqKI5)
- Signed Realized Variance Trend (ylUDT58m)
- Bilateral Filter Trend (yY27v3eD)
- MTF Trend Matrix (NjcGEJ94; versi bar tertutup)
- Order Flow Candle Classifier (s5xafrd1)

**Hipotesis:** Pada tren yang terkonfirmasi, celah harga (FVG) yang tercipta oleh impuls searah tren sering dimitigasi sebagian lalu dilanjutkan. Masuk di tengah FVG memberi SL struktural di bawah asal impuls dengan jarak 2–4%.

| Item | Spesifikasi |
|---|---|
| TF | Struktur 1h, FVG 15m, konfirmasi 15m |
| Regime | Screener TREND searah + SRV 15m searah (\|SRV\| ≥ 0,2) |
| Setup | FVG bullish 15m: `low[t] > high[t−2]`, gap ≥ 0,30×ATR14(15m), candle tengah body ≥ 60% range dan delta candle tengah > 0 |
| Entry | Limit di **50% FVG** (midpoint) setelah FVG terbentuk (bar t close). Aktif maksimum 16 bar 15m (4 jam). Batal bila close 15m menembus tepi bawah FVG sebelum fill |
| Konfirmasi order flow | Bar 15m yang menyentuh zona tidak boleh "dorongan jual" menurut OFCC (delta ≤ −p90 dan close di 25% bawah range) |
| SL | Low terendah dari 3 bar pembentuk FVG dan swing 15m terakhir yang terkonfirmasi (pivot 3/3), dikurangi 0,15×ATR14(15m). SKIP bila ∉ [2%; 4%] |
| TP | 2R |
| Short | Cermin |
| Parameter riset | gap_min {0,2; 0,3; 0,45}×ATR; masa aktif {8; 16} bar; entry level {50%; 62%} FVG → 12 trial |

```
on_bar_close(15m, sym):
  if screener[sym].state != TREND_UP: return
  if srv_15m < 0.2: return
  if low[t] > high[t-2] and (low[t]-high[t-2]) >= gap_min*atr15 and body(t-1)>=0.6*range(t-1) and delta(t-1)>0:
      entry = high[t-2] + 0.5*(low[t]-high[t-2])
      sl    = min(low[t-2..t], last_confirmed_pivot_low_15m(3,3)) - 0.15*atr15
      if 0.02 <= (entry-sl)/entry <= 0.04: submit_pending(LONG, entry, sl, ttl=16 bars)
```

**Anti-repaint:**
- Bias HTF memakai bar 1h tertutup (setara `security(...[1], lookahead_on)` di sumber).
- Pivot 3/3 baru dipakai pada `close_time(t+3)`.

**Gagal bila:** ekspektansi IS dengan regime filter tidak ≥ +0,05R di atas versi tanpa filter (SCREENER §7).

---

## S2 — Liquidity Sweep Reclaim + Delta Trap (LSR)

**Sumber:**
- Liquidity Hunt Pro (CDQo43CF)
- Absorption Reversal (wMcOiPoG, strategy)
- Smart Money Absorption Radar (7XJBJBZH)
- Eye Liquidity Heatmap (bpiQxWOz)
- Delta Divergence Alert (s7MM2a5V)
- CandelaCharts Auto Anchored Footprint (SyWHPxSr, **terkunci**, ide imbalance 3:1 saja)

**Hipotesis:** Dalam tren naik, pullback yang menyapu swing low terdekat (pool stop) lalu diserap pembeli pasif menghasilkan reversal searah tren. Absorpsi terlihat sebagai delta sangat negatif tetapi harga close di atas.

| Item | Spesifikasi |
|---|---|
| TF | Pool likuiditas dari pivot 15m (5/5) dan 1h (3/3). Trigger 15m |
| Regime | Screener TREND_UP |
| Setup | Bar 15m `low < pool_level` (pool = pivot low terkonfirmasi, umur ≥ 10 bar, belum tersapu) dan close ≥ pool_level dalam ≤ 3 bar (reclaim) |
| Delta trap | Pada bar sweep atau bar sesudahnya: `delta ≤ −P95(|delta|, 500 bar)` **dan** close di 40% atas range (absorpsi), **atau** candle hijau dengan delta < 0 (divergensi s7MM2a5V) |
| Footprint (opsional, varian F) | Dari aggTrades: di 3 level harga terbawah bar sweep ada imbalance diagonal ask/bid ≥ 3:1 di sisi beli (ide SyWHPxSr, diimplementasi sendiri). Hanya dihitung bila data aggTrades tersedia |
| Entry | Limit di 50% body candle reclaim, aktif setelah candle itu close, TTL 4 bar 15m |
| SL | Ekstrem sweep − 0,15×ATR14(15m). SKIP bila ∉ [2%; 4%] |
| TP | 2R. Filter target: bila ada pool likuiditas lawan aktif (Eye Heatmap / Magnet) di antara entry dan 1R → SKIP |
| Short | Cermin (sweep pool high dalam TREND_DOWN) |
| Parameter riset | persentil {90; 95}; reclaim {2; 3} bar; umur pool {10; 20} → 8 trial (+8 untuk varian F) |

```
on_bar_close(15m, sym):
  if state != TREND_UP: return
  pool = nearest_unswept_confirmed_pivot_low(sym, age>=10)
  if low[t] < pool and close[t] >= pool:                       # sweep + reclaim di bar yang sama, atau ≤3 bar
     trap = (delta[t] <= -p95_abs_delta and close_pos[t] >= 0.6) or (close[t]>open[t] and delta[t]<0)
     if trap:
        entry = open[t] + 0.5*(close[t]-open[t]); sl = low[t] - 0.15*atr15
        if 0.02 <= (entry-sl)/entry <= 0.04 and not opposing_pool_within(entry, 1R): submit_pending(...)
```

**Anti-repaint:**
- Pivot pool baru aktif setelah konfirmasi.
- Delta asli Binance menggantikan `security_lower_tf` dan `request.footprint` TradingView, yang hasilnya berubah antar periode history (CATALOG #5, #17).
- Heatmap Eye adalah estimasi OHLCV, bukan order book.

---

## S3 — Structure Break Pressure Retest (SBR)

**Sumber:**
- Structure Break Pressure Oscillator (JvhjGvJq)
- Algo Market Structure (C4vopA91; bar tertutup)
- Market Structure Oscillator [BigBeluga] (mFMTYEYR)
- Decycler High-Low Breakout (dOcUNfyv)
- Strong Pressure Radar (HtufiPGL)
- Key Level Confluence (JBG8SZY9)

Turunan langsung dari "Strategi 1 Structure Pressure Scalper" di apex `research/tv7_scalp_20261104`.

**Hipotesis:** Break struktur (BoS) yang searah tren 1h dan bertekanan tinggi (jarak break/ATR × RVOL), lalu diretest dan ditahan, cenderung berlanjut sampai 2R.

| Item | Spesifikasi |
|---|---|
| TF | Struktur & BoS 15m (pivot 5/5), konteks 1h |
| Regime | Screener TREND searah + Decycler 1h state searah |
| Setup | Close 15m menembus pivot high terkonfirmasi → `break_level`. SBPO (skala 5–50) ≥ +0,5 pada bar break |
| Retest | Dalam ≤ 8 bar: `low ≤ break_level + 0,25×ATR` dan `close ≥ break_level` dan close > open |
| Votes (≥ 2 dari 4) | (a) Pressure Radar ≥ 60; (b) delta 3 bar terakhir > 0; (c) ≥ 2 key level (prev-day H/L/C, pivot harian, swing) dalam 0,6×ATR dari break_level; (d) MSO > sinyalnya |
| Entry | Varian L: limit 50% body candle konfirmasi (TTL 4 bar). Varian M: MARKET saat close candle konfirmasi (taker) |
| SL | Low terendah retest − 0,15×ATR14(15m). SKIP bila ∉ [2%; 4%] |
| TP | 2R |
| Parameter riset | SBPO {0,4; 0,5; 0,6}; retest {8; 16}; votes {2; 3}; entry {L; M} → 24 trial |

**Anti-repaint:**
- Algo Market Structure di sumber memakai `security` tanpa `[1]`, sehingga di port wajib memakai bar HTF tertutup.
- SBPO memakai pivot len/len, jadi event berlaku pada `close_time(t+len)`.

---

## S4 — Wyckoff Range-Exit SOS→LPS (WSL)

**Sumber:**
- Wyckoff Accumulation/Distribution, Spring, UTAD, SOS (y9jjEUGv)
- Nexus Profile Candles (dRPdp2hr)
- EMMA Supply & Demand Volume Profile (WFCwhWZP)
- Order Flow Candle Classifier (s5xafrd1)

**Kompatibilitas dengan aturan anti-sideways:** S4 TIDAK masuk di dalam range. S4 hanya masuk pada **LPS setelah SOS**, yaitu setelah harga keluar dari range akumulasi dan screener sudah berubah menjadi TREND_UP (histeresis 2 bar 1h). Fase range hanya dipakai untuk menggambar level.

**Hipotesis:** Breakout dari range akumulasi yang didahului spring dan dikonfirmasi SOS bervolume tinggi, lalu retest (LPS) ke atas VAH/AR, punya kelanjutan ≥ 2R.

| Item | Spesifikasi |
|---|---|
| TF | Range & event 1h (pivot 3/3), LPS 15m |
| Range | SC → AR → ST terkonfirmasi, durasi range 10–60 bar 1h, tinggi range ≥ 4% (agar SL 2–4% muat) |
| SOS | Close 1h > AR + 0,2×ATR(1h), volume ≥ 1,5×SMA20, delta bar > 0 |
| LPS | Dalam ≤ 24 bar 15m setelah SOS: pullback ke [AR − 0,3×ATR1h; AR + 0,3×ATR1h] atau ke VAH profil range (volume profile 24 baris, VA 70%), dengan bar 15m bukan "dorongan jual" (OFCC) |
| Gerbang screener | State simbol = TREND_UP saat LPS |
| Entry | Limit di AR (atau VAH bila lebih tinggi), TTL 8 bar 15m |
| SL | Di bawah low LPS dan di bawah VAL−0,15×ATR1h; pilih yang lebih dekat tetapi masih di bawah low LPS. SKIP bila ∉ [2%; 4%] |
| TP | 2R; kalau target tinggi range (proyeksi Wyckoff) < 2R → SKIP |
| Short | UTAD → SOW → LPSY |
| Parameter riset | vol SOS {1,3; 1,5; 1,8}; LPS window {12; 24}; spring wajib {ya; tidak} → 12 trial |

**Anti-repaint:**
- Event Wyckoff valid setelah pivot konfirmasi.
- Profil volume memakai **range yang sudah selesai** (tidak dihitung ulang tiap bar seperti EMMA asli).

---

## S5 — Rank Momentum Pullback + kNN Grader (RMP)

**Sumber:**
- Distributional Rank (gGTzHW8l)
- ApexWave Live Sentiment (KduAs9nf)
- Fibonacci & RSI Aligned (jLIlXavw)
- RSI Signals Entries (MXaEHFCb)
- Multi-MA Trend Lines (8xFk64Tm)
- Strong Magnet Zones (lja4dnnZ)
- Unbroke Signal Grader kNN (sd69zJf2)

**Hipotesis:** Saat rank distribusional 1h tinggi (harga di persentil atas jendela 80 bar) dan MA bertumpuk naik, pullback ke Fibonacci 0,5–0,618 dari impuls terakhir dengan RSI turun ke 40–50 adalah titik lanjutan. Grader kNN berbasis sinyal masa lalu (hanya yang sudah selesai) menyaring setup berkualitas rendah.

| Item | Spesifikasi |
|---|---|
| TF | Regime & impuls 1h, trigger 15m |
| Regime | Screener TREND_UP + DistRank 1h ≥ 70 + ApexWave bull% ≥ 60 + EMA20 > EMA50 > EMA200 (1h) dan EMA50 slope > 0 |
| Impuls | Swing low → swing high 1h terkonfirmasi (pivot 3/3), tinggi ≥ 2,5×ATR1h |
| Pullback | Harga 15m masuk zona Fib 0,5–0,618 impuls; RSI14(15m) menyentuh 35–50 |
| Trigger | Close 15m kembali di atas EMA21(15m) dengan body ≥ 1,2× rata-rata body 20 (RSI Signals "candle kuat") |
| Filter magnet | Tidak ada Magnet Zone lawan aktif antara entry dan 1R |
| kNN grader (varian K) | k=15, fitur = 5 percent-rank (300 bar) dari: ER, SRV, RSI, ATR%, volume relatif. Label = hasil trade sinyal masa lalu yang **sudah tertutup** sebelum t. Ambil hanya bila ≥ 55% tetangga menang. Memori dilatih hanya di IS; di OOS hanya menambah sinyal yang telah selesai |
| Entry | Limit di 50% body candle trigger, TTL 4 bar |
| SL | Low pullback − 0,15×ATR14(15m); bila < 2% dari entry pakai Fib 0,786 − 0,15×ATR jika itu membuat SL% ∈ [2%; 4%], selain itu SKIP |
| TP | 2R |
| Parameter riset | DistRank {65; 70; 75}; Fib zone {0,5–0,618; 0,382–0,618}; kNN {off; on} → 12 trial |

**Anti-repaint:**
- Label kNN memakai sinyal yang selesai sebelum t (risiko look-ahead terbesar di strategi ini; ada tes khusus T-2.9 di ROADMAP).
- Pivot Fib baru aktif setelah konfirmasi.

---

## 6. Skrip yang dipakai sebagai komponen pendukung (bukan generator)

| Skrip | Peran |
|---|---|
| RCV Risk Manager (kisiqlcI) | Rumusnya diganti Risk Manager Rust (risk $1 tetap, margin variabel). Tidak dipakai langsung |
| RSI Signals Entries (MXaEHFCb) | Hanya "candle kuat" & zona RSI. SL pips tetapnya ditolak karena melanggar GR-3 |
| Nexus/EMMA volume profile | Level VAH/VAL/POC untuk S4 dan filter target |
| Eye Heatmap / Magnet Zones | Filter "lawan dalam 1R" untuk S2 dan S5 |

## 7. Kriteria lulus per strategi (ringkasan; detail di Rules.md §4)

| Gerbang | Ambang |
|---|---|
| Trade IS / OOS / holdout | ≥ 100 / ≥ 60 / ≥ 40 |
| Ekspektansi neto OOS (biaya 1×) | ≥ +0,10R |
| Ekspektansi neto OOS (biaya 2× + slip 0,05%) | ≥ +0,03R |
| Profit factor OOS (biaya 1×) | ≥ 1,25 |
| Walk-forward 6 fold | ≥ 4 fold ekspektansi > 0 |
| Monte Carlo 10.000× (bootstrap urutan trade) | p95 max drawdown ≤ 15R; P(equity < $40) ≤ 5% |
| Deflated Sharpe | PSR terdeflasi ≥ 0,95 dengan N = total trial di ledger |
| Plateau | ≥ 80% tetangga ±1 langkah grid memiliki ekspektansi > 0 |
| Konsentrasi | Tidak ada simbol menyumbang > 40% profit neto |
| Holdout final | Dijalankan **sekali**. Gagal = strategi ditolak, tidak di-tuning ulang |
