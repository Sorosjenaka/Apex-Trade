# Aturan Proyek: Apex Scalper RS

> v1.0 · 5 Nov 2026 (WIB). File ini adalah SSoT untuk angka aturan risiko, rumus, dan protokol validasi. `config/risk.toml` wajib sama dengan §1/§4; engine menolak start bila berbeda (test T-0.6).

## 1. Golden Rules (TIDAK BOLEH dilanggar)

| ID | Aturan | Ditegakkan oleh |
|---|---|---|
| GR-1 | **Risk per trade = $1,00 neto terencana** (termasuk fee masuk, fee keluar, slippage SL 0,02%). Tidak ada compounding, tidak ada risk % equity, tidak ada pengecualian | FR-011, U-01..U-05 |
| GR-2 | **Margin tidak tetap.** `notional ≈ $1 / jarak SL`; `margin = notional / 25`. Contoh: SL 2% → ±$47 notional → ±$1,89 margin; SL 4% → ±$24 → ±$0,97. **Tidak ada sizing acak**, tidak ada margin input manual. qty selalu dibulatkan **ke bawah** ke stepSize. SKIP bila risk aktual < $0,80, notional < minNotional, atau qty < minQty | FR-011, FR-013 |
| GR-3 | **SL struktural 2,00%–4,00%** dari harga entry (setelah pembulatan tick). Di luar rentang = **SKIP**. SL tidak boleh digeser untuk memaksa masuk rentang | FR-012 |
| GR-4 | **Leverage 25x, marginType CROSSED, position mode one-way.** Mismatch = tidak boleh ARM. ISOLATED dilarang karena likuidasi 3,0–3,6% < SL 4% [→ FEASIBILITY §3] | FR-004 |
| GR-5 | **Maksimum 2 posisi** (posisi terbuka + entry pending) di seluruh akun; maksimum 1 per simbol | FR-014 |
| GR-6 | **Stop harian:** setelah **6 exit SL** atau **PnL realized neto ≤ −$6,00** dalam hari UTC (00:00 UTC = 07:00 WIB), tidak ada entry baru sampai hari UTC berikutnya. Posisi terbuka tetap dikelola SL/TP | FR-016 |
| GR-7 | **Cooldown 60 menit global** setelah setiap exit SL, dihitung dari waktu fill SL, berlaku untuk semua simbol & strategi | FR-015 |
| GR-8 | **Setiap posisi wajib SL+TP di exchange** (`/fapi/v1/algoOrder`, `closePosition=true`, `MARK_PRICE`) terkonfirmasi ≤ 2 dtk setelah fill. Bila belum ada dalam 5 dtk → tutup MARKET reduceOnly + DISARM | FR-018, FR-019 |
| GR-9 | **ARM default `false`** di setiap start. Hanya user yang meng-ARM secara manual. Urutan wajib: replay → validasi PASS → **testnet** ≥ 14 hari/≥ 30 trade → mainnet shadow ≥ 7 hari → ARM mainnet. Hanya strategi berstatus GO yang boleh mengirim order | FR-002, FR-022, FR-034 |
| GR-10 | **Hanya trade di pasar trending** (state TREND_UP/TREND_DOWN searah sinyal) [→ SCREENER.md]. Tidak ada entry pada RANGING/BLOCKED. Equity < $40 → DISARM | FR-009, FR-024 |

## 2. Konvensi Coding

- Rust edition 2021, `cargo fmt` dan `cargo clippy -- -D warnings` wajib bersih.
- Penamaan: crate `snake_case`; tipe `PascalCase`; konstanta `SCREAMING_SNAKE_CASE`. ID requirement disebut di doc-comment fungsi terkait, mis. `/// FR-011`.
- Fungsi ≤ 60 baris; file ≤ 600 baris (kecuali port `portfolio.rs`, dipecah bertahap).
- Uang/harga/qty: `rust_decimal::Decimal`. `f64` hanya untuk indikator/fitur. Konversi lewat fungsi tunggal `core::num::to_dec_round_down()`.
- Tidak ada `unwrap()`/`expect()` di jalur engine live (`apps/engine`, `crates/exec`, `crates/risk`). Gunakan `thiserror` + `?`.
- Semua waktu `i64` epoch ms UTC. Tidak ada `Local::now()` di logika (REVIEW L5).
- Strategi adalah fungsi murni `fn on_bar_close(&State, &BarView) -> Option<Signal>`; tidak ada I/O.
- Komentar menjelaskan **kenapa**, bukan apa. Setiap ambang angka di kode merujuk ke config, tidak hardcode.

## 3. Konvensi Git

- Commit: `type(scope): pesan` — type ∈ `feat, fix, test, docs, refactor, chore, perf`; scope = nama crate atau `plan`. Contoh: `feat(risk): sizing risk $1 margin variabel (FR-011)`.
- Branch: `feat/<nama>`, `fix/<nama>`, `research/<strategi>-<tanggal>`.
- 1 tugas ROADMAP `T-x.x` = minimal 1 commit; pesan commit menyebut ID tugas.
- `main` dilindungi: CI (fmt, clippy, test, gitleaks) wajib hijau.

## 4. Aturan Bisnis

### 4.1 Sizing (SSoT rumus — GR-1, GR-2, GR-3)

```
fee_in  = 0,0002 (LIMIT GTX)  |  0,0005 (MARKET)
fee_out = 0,0005             # SL/TP STOP_MARKET/TAKE_PROFIT_MARKET = taker
slip    = 0,0002             # model slippage SL
side    = +1 LONG / −1 SHORT

entry, sl  dibulatkan tick (Schema §5)
sl_pct     = |entry − sl| / entry                       → SKIP_SL_RANGE bila ∉ [0,02; 0,04]
stop_fill  = sl × (1 − side × slip)
loss_unit  = side × (entry − stop_fill) + entry × fee_in + stop_fill × fee_out
qty        = floor((1,00 / loss_unit) / step) × step
notional   = qty × entry
margin     = notional / 25
risk       = qty × loss_unit                            → SKIP_SIZE bila < 0,80 atau notional < minNotional atau qty < minQty
tp         = entry + side × 2 × |entry − sl|            # RR 2,0
```

Kasus uji wajib (`crates/risk/tests/sizing.rs`):

| ID | Input | Hasil yang benar |
|---|---|---|
| U-01 | LONG entry 100, SL 98, step 0,001, LIMIT | qty 0,478 · notional 47,80 · margin 1,912 · risk ≈ 0,9983 |
| U-02 | LONG entry 100, SL 96, step 0,001, LIMIT | qty 0,244 · notional 24,40 · margin 0,976 · risk ≈ 0,9973 |
| U-03 | LONG ETH entry 3000, SL 2925, step 0,001, LIMIT | qty 0,012 · notional 36,00 · margin 1,44 · risk 0,9318 |
| U-04 | SL 1,99% | SKIP_SL_RANGE |
| U-05 | SL 4,01% | SKIP_SL_RANGE |
| U-06 | step menyebabkan risk < 0,80 (entry 717, SL 691,905 = 3,5%, step 0,01) | qty 0,03 · risk ≈ 0,7717 → SKIP_SIZE |
| U-07 | 2 sinyal dengan SL berbeda | margin berbeda (bukan nilai tetap) |
| U-08 | properti (proptest, 10.000 kasus): untuk semua input lolos → 0,80 ≤ risk ≤ 1,00 dan margin = notional/25 | lulus 100% |

### 4.2 Gate akun (GR-4..GR-7, GR-10)
Urutan cek tetap (gagal pertama → SKIP): ARM → regime → SL range → max posisi → cooldown → stop harian → equity floor → spread ≤ 3 bps → sizing.
- Hari = UTC. `daily_sl_count` dan `daily_realized_net` di-reset 00:00:00.000 UTC.
- Exit SL = exit oleh algo STOP_MARKET atau oleh FR-019. Exit TP/MANUAL tidak menambah `sl_count`, tetapi tetap masuk `realized_net`.

### 4.3 Protokol Validasi (standar quant)

**Pembagian data (UTC):**

| Segmen | Periode | Pemakaian |
|---|---|---|
| IS (pengembangan) | 2025-01-01 – 2025-12-01 | pemilihan parameter (tuning boleh di sini), maksimal 20 kombinasi untuk riset BTCUSDT+ETHUSDT |
| VAL (validasi) | 2025-12-01 – 2026-07-01 | evaluasi & pemilihan kandidat terpilih; walk-forward ikut memakai IS+VAL |
| TEST / HOLDOUT | 2026-07-01 – 2026-11-04 | **dikunci** (holdout lama 2026-05-01..2026-10-31 dibuka atas keputusan user 2026-11-05), dijalankan **sekali** untuk kandidat final tanpa tuning ulang |
| Forward | testnet ≥ 14 hari + shadow mainnet ≥ 7 hari | paritas eksekusi |

Simbol yang listing setelah awal IS hanya dipakai sejak listing + 30 hari. ETH 2025–2026 dilaporkan terpisah karena terkontaminasi riset apex (A-12).

**Model biaya baseline:** fee_in/fee_out §4.1, slippage SL 0,02%, funding aktual dari histori `fundingRate`, entry limit hanya terisi bila harga **menembus** limit minimal 1 tick (bukan sekadar menyentuh), SL-first bila SL & TP dalam satu bar, gap memakai open bar.

**11 gerbang (FR-031), semua harus PASS:**

| ID | Gerbang | PASS bila |
|---|---|---|
| G-01 | Integritas & kausalitas | 0 celah data, checksum cocok, prefix test 100% identik (NFR-18) |
| G-02 | Jumlah sampel | trade IS ≥ 100, VAL ≥ 60, HOLDOUT ≥ 40 (kurang → INCONCLUSIVE) |
| G-03 | Ekspektansi VAL (biaya 1×) | ≥ +0,10R per trade |
| G-04 | Profit factor VAL | ≥ 1,25 |
| G-05 | Stress biaya | fee 2×, slip 0,05%, funding 2× → ekspektansi ≥ +0,03R **dan** PF ≥ 1,10 |
| G-06 | Walk-forward | anchored 6 fold di IS+VAL (re-fit hanya dari grid yang sama) → ≥ 4 fold ekspektansi > 0; fold terburuk ≥ −0,15R |
| G-07 | Monte Carlo | 10.000 bootstrap urutan trade VAL (ukuran = 1 tahun trade) → p95 max DD ≤ 15R dan P(equity < $40 dari $60) ≤ 5% |
| G-08 | Deflated Sharpe | PSR terdeflasi ≥ 0,95 dengan N = total trial di `trials_ledger` untuk strategi itu + 40 trial `apex_legacy` |
| G-09 | Plateau parameter | ≥ 80% tetangga ±1 langkah grid punya ekspektansi VAL > 0, dan median tetangga ≥ 50% ekspektansi terpilih |
| G-10 | Konsentrasi | tidak ada simbol > 40% profit neto; tidak ada bulan > 35% profit neto |
| G-11 | Holdout sekali jalan | ekspektansi > 0 dan PF ≥ 1,10; gagal = strategi **ditolak permanen** untuk versi parameter itu |

**Gerbang forward (sebelum ARM mainnet):** testnet ≥ 30 trade, `mean(realized_slip_R − model_slip_R) ≤ 0,05R`, 0 `SL_MISSING`, 0 `RULE_VIOLATION`.

**Pelaporan wajib per strategi:** trades/hari, holding time median & p90, win rate, ekspektansi, PF, max DD (R & $), funnel skip (berapa sinyal gugur di tiap gate), kontribusi per simbol/bulan, long vs short.

## 5. Definisi "Selesai"

□ Semua FR terkait lolos acceptance criteria US-nya [→ PRD §4.2]
□ Semua NFR terkait lolos Verifikasi [→ PRD §6] dengan bukti angka
□ 1 happy-path test per FR P0 lulus (`cargo test` output dilampirkan di CHANGELOG)
□ Zero item Out-of-Scope [→ PRD §7.2] ikut terimplementasi
□ README.md masih akurat dengan cara run terbaru
□ (tambahan proyek) `cargo clippy -D warnings` bersih dan `gitleaks detect` 0 temuan
□ (tambahan proyek) Untuk tugas eksekusi: diuji di testnet, bukan mainnet

## 6. Larangan

1. **Jangan pernah membaca, membuka, mencetak, atau menyalin isi `.env` atau secret apa pun**, termasuk oleh agen AI. Gunakan `.env.example` saja. Jangan commit `.env`.
2. Jangan mengirim order mainnet dari kode test, skrip riset, atau CI. Test eksekusi hanya ke mock (`wiremock`) atau testnet `demo-fapi.binance.com`.
3. Jangan mengubah ARM dari kode selain handler `POST /api/arm|disarm|kill` dan guard otomatis yang hanya boleh **men-DISARM**.
4. Jangan mengubah angka GR (§1) tanpa REVISI eksplisit dari user + update PRD §8 + `config/risk.toml`.
5. Jangan mengubah parameter strategi yang sudah GO tanpa menjalankan ulang seluruh gerbang §4.3 (REVIEW H8).
6. Jangan menjalankan HOLDOUT lebih dari sekali per versi parameter. Jangan "mengintip" holdout untuk memilih parameter.
7. Jangan menambah dependency di luar architecture §1 tanpa izin 1 baris dari user.
8. Jangan mengubah Schema.md tanpa update kode & test terkait pada commit yang sama.
9. Jangan memasang LLM/agen AI di jalur keputusan trade (REVIEW C7).
10. Jangan memakai `/fapi/v1/order` untuk STOP_MARKET/TAKE_PROFIT_MARKET (ditolak -4120 sejak 2025-12-09); wajib `/fapi/v1/algoOrder`.
11. Jangan menyalin source skrip TradingView ke repo sebagai kode produksi. Port adalah implementasi sendiri; lisensi asli dihormati, dan skrip terkunci tidak di-reverse-engineer.
