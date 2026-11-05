# UNDERSTANDING.md — Ringkasan Pemahaman FASE 0 (Apex Scalper RS)

Tanggal: 2026-11-05

## 1. Struktur Workspace & Crate (`Cargo.toml`)

Workspace Rust (`edition = "2021"`, `rust-version = "1.85"`, toolchain MSVC dengan target tunggal `target-msvc`) terdiri dari 8 crate pustaka di `crates/` dan 1 aplikasi CLI di `apps/apex`:

| Crate | Path | Peran Utama | Dependensi Internal |
|---|---|---|---|
| `apex-core` | `crates/core` | Tipe dasar domain, konfigurasi (`AllConfig`, `RiskConfig`, `ScreenerConfig`, `StrategiesConfig`), numerik `Decimal`, aturan sizing risk $1,00 (GR-1..GR-3), dan tipe bersama | — |
| `apex-indicators` | `crates/indicators` | Indikator teknikal kausal (`f64` untuk fitur/indikator: EMA, ATR, ADX, Efficiency Ratio, choppiness, pivot, dsb.) | `apex-core` |
| `apex-data` | `crates/data` | Fetch data publik Binance USDT-M Futures (REST klines 15m/1h/1d/5m & histori funding rate, WebSocket stream, penyimpanan/pembacaan CSV) dengan penjagaan batas holdout | `apex-core` |
| `apex-market-reader` | `crates/market_reader` | Klasifikasi regime pasar global (`RegimeState`, histeresis, `ChopThresholds` RG-2) serta konteks shadow reversal multi-timeframe MR-09 (`reversal.rs`, `reversal_context.rs`) | `apex-core`, `apex-indicators` |
| `apex-screener` | `crates/screener` | Screener per simbol, allowlist crypto default-deny, klasifikasi state trend/ranging/blocked, dan volume gate `V-01..V-10` | `apex-core`, `apex-indicators` |
| `apex-strategies` | `crates/strategies` | Fungsi sinyal murni closed-bar untuk 5 strategi (`S1_TFP`, `S2_LSR`, `S3_SBR`, `S4_WSL`, `S5_RMP`), konfirmasi OFCC (`ofcc_watch.rs`), filter magnet pool S5, dan kNN grader kausal S5 (`knn.rs`) | `apex-core`, `apex-indicators`, `apex-screener` |
| `apex-backtest` | `crates/backtest` | Engine backtest event-driven bebas look-ahead (`engine.rs`), simulasi fill limit/SL-first/gap/funding/extra_slip, metrik statistik (`stats.rs`: PF, Expectancy R, Max DD, Sharpe, Monte Carlo, PSR/DSR), serta evaluasi gerbang validasi `G-01..G-11` (`gates.rs`) | `apex-core`, `apex-indicators`, `apex-screener`, `apex-market-reader`, `apex-strategies` |
| `apex-oms` | `crates/oms` | Order Management System Binance Futures & `MockExchange`: HMAC signing, time sync, algo order SL/TP (`closePosition=true`, `MARK_PRICE`), `cancel_beyond`, rekonsiliasi state machine, kill switch, audit log, dan live safety gate | `apex-core`, `apex-data` |
| `apex` (bin) | `apps/apex` | CLI utama (`fetch`, `reader`, `screen`, `backtest`, `validate`, `record`, `run`, `kill`, `status`) yang mengorkestrasi data, backtest, sweep/validasi gerbang `G-01..G-11` (`bt.rs`), dan paper/live runner (`live.rs`) | Semua 8 crate di atas |

---

## 2. Status Worktree & Branch

Hasil `git worktree list`, `git branch -a`, `git status`, dan `git log --oneline -15`:

### A. Worktree
1. `C:\Users\FATHUR\Downloads\apex-scalper-rs` — repo utama, saat ini berada di branch `integrate/v1` (`d55c9b4`) dengan **3 file termodifikasi belum di-commit**:
   - `apps/apex/src/bt.rs` (helper `trial_sr` membuang trial `< 10` trade untuk DSR)
   - `crates/backtest/src/gates.rs` (`count_strategy_trials` memfilter `"trades" >= 10`)
   - `crates/market_reader/src/aggregate.rs` (`ChopThresholds::default()` diubah ke `adx_max: 15.0, er_max: 0.25, trend_share_min: 15.0`)
2. `C:\Users\FATHUR\Downloads\apex-wt-codex` — worktree Codex, berada di branch `codex/sample` (`8cef9b5`), working tree **clean**.
3. `C:\Users\FATHUR\Downloads\apex-wt-hermes` — worktree Hermes, berada di branch `hermes/validation` (`0ef3793`), working tree clean kecuali 1 file untracked `HANDOFF_GROK.md`.

### B. Ringkasan Posisi Branch
- `codex/strategies` (`5bf3982`):
  - Berasal dari `07a768a`.
  - Memuat T-C2 (`b1b8c6a` OMS `cancel_beyond`) dan T-C1 (`0506dc7`, `8b3650f`, `0f09d6a` MR-09 shadow scores & stage/Fib; `f1275a9` S1 OFCC; `cc8715c` S5 magnet pool filter; `ca6ee2b` S5 kNN grader kausal; `ef7a8da` & `5bf3982` dokumentasi laporan).
- `hermes/validation` (`0ef3793`):
  - Berasal dari `07a768a`.
  - Memuat penyatuan target dir `target-msvc` (`451ca0d`, `f05ad5e`), implementasi `gates.rs` G-02..G-11 + `ChopThresholds` (`e1cfca8`), runner `apex validate` & sweep CHOP (`d7dff24`, `6e312ca`), serta penyesuaian gate `G-02..G-11` (`0ef3793`).
- `integrate/v1` (`d55c9b4`):
  - Merge `codex/strategies` (`37d80c1`) lalu merge `hermes/validation` (`d55c9b4`) di atas `07a768a`.
- `master` (`221c94c`) & `codex/integration-review` (`221c94c`):
  - Berada 1 commit di atas `integrate/v1` (`d55c9b4`), yaitu commit `221c94c` (*"Fail closed on unverified validation gates"* pada `apps/apex/src/bt.rs` dan `REPORT_INTEGRATION_REVIEW.md`).
- `codex/sample` (`8cef9b5`):
  - Berada 6 commit di atas `master` (`221c94c`):
    - `cfd36d2`: `fix(apex): align validation split and fail closed on exploratory plateau` (mengubah `VAL_SPLIT_MS` ke `2025-07-01` di `bt.rs`, memisahkan `SWEEP_OOS_SPLIT_MS` `2026-01-01`).
    - `c1ee768`: `fix(data): keep funding fetch outside locked holdout` (`crates/data/src/rest.rs`).
    - `9892281` & `66b0d0c`: skrip & hasil riset T-C3 universe 40 pair (`research/`).
    - `15da076`: `feat(apex): T-C3 fetch public 5m research candles` (`--only-5m` di `apps/apex/src/main.rs` & `live.rs`).
    - `8cef9b5`: `research(T-C3): record default-off 5m entry after workspace tests` (`research_entry_5m = false` di `config/strategies.toml`, dukungan replay 5m di `crates/backtest/src/engine.rs`, `crates/strategies/`, `apps/apex/src/bt.rs`, `cfg.rs`).

---

## 3. File yang Overlap & Potensi Konflik saat Merge / Integrasi

1. **`crates/backtest/src/engine.rs`**:
   - `codex/strategies` menambahkan OFCC watch, magnet pool, kNN sample collector/warming, dan `reversal_shadow`.
   - `hermes/validation` menambahkan `chop_thresholds: Option<ChopThresholds>` dan `extra_slip_bps` pada `BacktestConfig` & `MarketReader::with_chop`.
   - `codex/sample` (`8cef9b5`) menambahkan dukungan `research_entry_5m`, validasi listing age 30 hari, pembatalan pending entry saat gate/regime/trend berubah, dan test prefix/no-look-ahead 5m.
   - *Resolusi*: Pertahankan seluruh fitur (`chop_thresholds`, `extra_slip_bps`, OFCC, magnet, kNN, `reversal_shadow`, `research_entry_5m` default `false`, pembatalan pending & cek listing age).
2. **`crates/market_reader/src/lib.rs` & `crates/market_reader/src/aggregate.rs`**:
   - `codex/strategies` mengekspor modul `reversal` dan `reversal_context`.
   - `hermes/validation` mengekspor `ChopThresholds`, `raw_state_with`, dan `MarketReader::with_chop`.
   - Uncommitted diff di `integrate/v1` mengubah `ChopThresholds::default()` ke `15 / 15 / 0.25`, sedangkan config default (`config/screener.toml` / `AllConfig`) juga perlu dicek agar threshold CHOP `15 / 15 / 0.25` terpasang di config default sesuai instruksi FASE 1.
3. **`apps/apex/src/bt.rs` & `crates/backtest/src/gates.rs`**:
   - `integrate/v1` memiliki uncommitted diff untuk `trial_sr` (buang trial `< 10` trade) di `bt.rs` dan `count_strategy_trials` (`trades >= 10`) di `gates.rs`.
   - `master` (`221c94c`) dan `codex/sample` (`cfd36d2`, `8cef9b5`) memodifikasi `apps/apex/src/bt.rs` (pemisahan `VAL_SPLIT_MS = 2025-07-01` vs `SWEEP_OOS_SPLIT_MS = 2026-01-01`, `research_entry_5m`, stress VAL, dsb.).
   - Sesuai instruksi FASE 1:
     - Samakan definisi VAL dengan `Rules.md` (VAL mulai `2025-07-01`) di gate runner dan **hapus pecahan `2026-01-01`**.
     - Perbaiki DSR: SR per trial dari R per trade, buang trial `< 10` trade.
     - Tambahkan walk-forward dengan **refit per fold** (tuning CHOP ulang hanya di data train tiap fold).
4. **`apps/apex/src/live.rs` & `apps/apex/src/main.rs`**:
   - `codex/strategies` menambahkan `cancel_beyond` & shadow reversal log di `live.rs`.
   - `hermes/validation` menambahkan subcommand `validate` di `main.rs` dan penyesuaian tanda tangan di `live.rs`.
   - `codex/sample` (`15da076`) menambahkan flag `--only-5m` pada subcommand `fetch`.

---

## 4. Aturan Keamanan & Larangan Mutlak (`AGENTS.md` & `docs/plan/Rules.md`)

1. **DILARANG mengirim order ke Binance** (baik live maupun testnet). Hanya `MockExchange` / mode paper lokal.
2. **`arm = false` dan `mode = "paper"` tidak boleh diubah**, termasuk kode live safety gate (`APEX_LIVE_CONFIRM`, host check, dsb.).
3. **DILARANG membaca, menyalin, membuka, atau mencetak `.env` dan API key/secret apa pun.**
4. **Holdout `2026-05-01..2026-10-31` terkunci**: dilarang menjalankan `--holdout` atau `--unlock-holdout`, dan dilarang menyentuh data holdout.
5. **Risk rules (`config/risk.toml` & Golden Rules GR-1..GR-10) tidak boleh diubah** (hanya threshold CHOP `15 / 15 / 0.25` yang dimasukkan ke config default). Universe 40 dan entry 5m tetap **OFF** di default.
6. **Tanpa `git push`**. Semua operasi git hanya lokal.
7. **Setiap klaim wajib menggunakan angka nyata dari output eksekusi** (`cargo test --workspace --release` dan `apex validate`).
