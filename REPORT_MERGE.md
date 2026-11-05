# REPORT MERGE — T-G2 Integration & Validation (`integrate/v1` → `master`)

## 1. Ringkasan Branch & Commit Hash

| Branch / Tahap | Commit Hash | Deskripsi |
|---|---|---|
| `master` (sebelum merge) | `56a3f03` | `fix(test): align market reader test expectations with relaxed chop thresholds` |
| `codex/strategies` | `5bf3982` | `feat(strategies,backtest,oms): add S1 OFCC watch, S3 magnet filter, and S5 kNN grader` |
| Merge `codex/strategies` | `37d80c1` | `merge(integrate/v1): codex/strategies into integrate/v1` |
| `hermes/validation` | `0ef3793` | `feat(validation): enforce G-02 inconclusive rule and honesty fixes for G-01..G-11` |
| Merge `hermes/validation` | `d55c9b4` | `merge(integrate/v1): hermes/validation into integrate/v1` |
| `codex/sample` | `8cef9b5` | `feat(sample): support 40-symbol universe and 5m S1/S3 entry research` |
| `integrate/v1` (T-G2 final) | *(lihat `git log -1`)* | Penyamaan `VAL=2025-07-01`, default CHOP `15/15/0.25`, perbaikan DSR (`R` per trade, drop `<10` trade), Walk-Forward refit CHOP per fold di train, dan evaluasi gabungan Universe Default & Universe 40 |

- **Status `master` lokal**: Fast-forward (`--ff-only`) ke `integrate/v1`. Tidak ada `git push` yang dijalankan.
- **Target directory tunggal**: `target-msvc` (via `.\cargo-msvc.cmd`, toolchain `stable-x86_64-pc-windows-msvc`). Direktori `target/` lama telah dihapus.

---

## 2. Penyelesaian Konflik & Default Konfigurasi

### 2.1 `crates/backtest/src/engine.rs`
Menggabungkan seluruh fitur dari ketiga branch (`codex/strategies`, `hermes/validation`, `codex/sample`) tanpa kehilangan fungsi:
- **`codex/strategies` (`5bf3982`)**:
  - `Pending.ofcc_watch: Option<S1Watch>` untuk konfirmasi order-flow candle close (`S1_TFP`) sebelum peletakan limit order post-only (`GTX`), termasuk pengecekan ulang volume gate (`current_volume_ok`), regime reader, dan risk gate saat konfirmasi.
  - `S5_RMP` kNN causal grader (`knn_training_start`, `knn_training_config`, `knn_sample`, `knn::grade`): melatih seed kNN hanya pada replay IS sebelum `cfg.start` dengan biaya baseline (`fee_mult = 1.0`, `funding_mult = 1.0`, `extra_slip = 0.0`) dan memblokir overlap terhadap holdout terkunci (`2026-05-01..2026-10-31`). Label trade tertutup baru masuk memori setelah close bar (`closed_ms < t_close`).
- **`hermes/validation` (`0ef3793`)**:
  - `BacktestConfig.chop: ChopThresholds` diteruskan ke `RegimeTimeline::build(..., cfg.chop)`.
- **`codex/sample` (`8cef9b5`)**:
  - `BacktestConfig.entry_tf: Tf` (`15m` default, `5m` khusus riset `S1/S3` dengan `RegimeMode::Reader`), validasi kontinuitas bar `5m` (`valid_five_minute_bars`), histeresis reader tetap pada penutupan bar `15m` (`closed_reader_decision`), pengecekan umur listing minimal 30 hari (`listed_long_enough` sesuai Rules §4.3), dan validasi `symbol_trending_at_open` serta `account_blocks_fill` saat fill order pending/market.

### 2.2 `crates/market_reader/src/lib.rs` & `crates/market_reader/src/aggregate.rs`
- Mempertahankan `pub use aggregate::{aggregate, AltSample, ChopThresholds, RawVote};` di `crates/market_reader/src/lib.rs`.
- `MarketReader::evaluate_with_chop` dan `RegimeTimeline::build` menerima `chop: ChopThresholds` secara eksplisit.
- `ChopThresholds::default()` diperbarui menjadi:
  - `trend_share_min = 15.0` (15%)
  - `adx_max = 15.0`
  - `er_max = 0.25`
- Nilai default yang sama dimasukkan ke `config/screener.toml` (`[chop]`) dan dipetakan melalui `apps/apex/src/cfg.rs` (`ScreenerFile.chop`) ke `BacktestConfig.chop`.

### 2.3 Verifikasi Default Konfigurasi & Keamanan
- **Universe 40 OFF di default**: `config/engine.toml` mempertahankan `universe = []` (11 altcoin default + `BTCUSDT`).
- **Entry 5m OFF di default**: `config/strategies.toml` mempertahankan `research_entry_5m = false` dan `s5_knn_enabled = false`.
- **Aturan risiko & safety gate tidak diubah**: `mode = "paper"`, `arm = false`, `risk_usd = 1.0`, `leverage = 25`, `rr = 2.0`, `sl_min_pct = 0.02`, `sl_max_pct = 0.04`, `max_positions = 2`, `cooldown_after_sl_sec = 3600`, `daily_sl_limit = 6`, `daily_loss_limit_usd = 6.0`.
- **Holdout tetap terkunci**: `2026-05-01..2026-10-31` tidak pernah diakses (`--holdout` / `--unlock-holdout` tidak digunakan).

---

## 3. Perbaikan Metodologi Validasi (Tugas T-G2 #2–#5)

1. **Penyamaan Definisi `VAL` dengan `Rules.md` (`apps/apex/src/bt.rs`)**:
   - Pecahan `2026-01-01` dihapus sepenuhnya.
   - `VAL_FROM = "2025-07-01"`, `FIT_TO = VAL_FROM`, dan `OOS_FROM = VAL_FROM`.
   - Segmen `IS` adalah `[from, 2025-07-01)` (`2025-02-01..2025-07-01`) dan segmen `VAL` adalah `[2025-07-01, 2026-05-01)`.
2. **Threshold CHOP Default `15 / 15 / 0.25`**:
   - Diterapkan di `ChopThresholds::default()`, `config/screener.toml`, `apps/apex/src/cfg.rs`, dan baseline `cmd_validate` di `apps/apex/src/bt.rs`.
3. **Perbaikan DSR (`apps/apex/src/bt.rs` & `crates/backtest/src/gates.rs`)**:
   - `trial_sr(rs: &[f64]) -> Option<f64>` menghitung Sharpe Ratio (`sharpe_ratio(rs)`) dari vektor **`R` per trade** (`t.r`) untuk setiap strategi pada suatu trial.
   - Trial dengan **`< 10` trade dibuang** (`None`, tidak dimasukkan ke `session_srs` dan diabaikan oleh `count_strategy_trials` pada pembacaan `trials_ledger.jsonl` melalui field `strategy_trades` / `trades`).
4. **Walk-Forward Anchored 6-Fold dengan Refit CHOP per Fold (`crates/backtest/src/gates.rs` & `apps/apex/src/bt.rs`)**:
   - Fungsi `refit_threshold_from_train` dan `refit_chop_fold` melakukan tuning ulang `ChopThresholds` (`trend_share_min` di `[0, 5, 10, 15, 20, 25, 30]`, `adx_max` di `[6, 9, 12, 15, 18, 24, 30]`, `er_max` di `[0.10, 0.15, 0.20, 0.25, 0.30, 0.35, 0.40]`) **hanya menggunakan data train `[train_start, train_end)` pada masing-masing fold** sebelum dievaluasi sekali pada periode test `[test_start, test_end)` fold tersebut.

---

## 4. Hasil `cargo test --workspace --release`

Dijalankan satu kali pada `target-msvc` (`.\cargo-msvc.cmd test --workspace --release`):

| Crate / Target Test | Passed | Failed | Durasi |
|---|---:|---:|---:|
| `apex` (bin unit tests) | 8 | 0 | 0.00s |
| `apex-backtest` | 21 | 0 | 4.05s |
| `apex-core` | 23 | 0 | 0.02s |
| `apex-data` | 6 | 0 | 0.02s |
| `apex-indicators` | 6 | 0 | 0.00s |
| `apex-market-reader` | 18 | 0 | 0.01s |
| `apex-oms` (unit tests) | 7 | 0 | 0.00s |
| `apex-oms` (`tests/state_machine.rs`) | 20 | 0 | 0.13s |
| `apex-screener` | 8 | 0 | 0.02s |
| `apex-strategies` | 17 | 0 | 1.67s |
| **TOTAL WORKSPACE** | **134** | **0** | **LULUS (100%)** |

---

## 5. Hasil `apex validate --strategies S1,S2,S3,S4,S5` (`2025-02-01` → `2026-05-01`, Tanpa Holdout)

Catatan aturan G-02 (`Rules.md` §7.2 & `crates/backtest/src/gates.rs`):
- Syarat minimum sampel: **`IS ≥ 100` trade** (`2025-02-01..2025-07-01`) **DAN `VAL ≥ 60` trade** (`2025-07-01..2026-05-01`).
- Apabila `G-02` berstatus `INCONCLUSIVE`, keseluruhan validasi strategi otomatis berstatus **`NO-GO (INCONCLUSIVE)`** (nilai mentah/raw pada G-03..G-10 tetap dilaporkan di bawah ini untuk transparansi).

### 5.1 Run 1 — Universe Default (11 Altcoin + `BTCUSDT`, `data/`, `reports/validate_default/validation_summary.json`)

#### Tabel Ringkasan Metrik & Gate (`Universe Default`)

| Strategi | IS Trades (min 100) | VAL Trades (min 60) | Total Trades | E[R] VAL (G-03 ≥ +0.10R) | PF VAL (G-04 ≥ 1.25) | Stress VAL E[R] / PF (G-05 >0 / ≥1.05) | WF Refit Positif / Terburuk (G-06 ≥4/6, ≥−0.15R) | MC p95 DD / P(<$40) (G-07 ≤15R, ≤5%) | DSR / N / varSR (G-08 ≥ 0.95) | Plateau G-09 (Tetangga >0 / Median vs 50% E_VAL) | Konsentrasi G-10 (Simbol ≤40% / Bulan ≤35%) | Keputusan |
|---|---:|---:|---:|---:|---:|---|---|---|---:|---|---|---|
| **S1_TFP** | 12 | 18 | 30 | `+0.129R` *(raw PASS)* | `1.20` *(raw FAIL)* | `+0.068R` / `1.10` *(raw PASS)* | `4/6` / `-1.006R` *(raw FAIL)* | `11.1R` / `0.0%` *(raw PASS)* | `0.040` (`N=107`, `prior=7`, `sesi=60`, `varSR=0.048995`) *(raw FAIL)* | `1/2 (50%)`, med `+0.114` vs `+0.064` *(raw FAIL)* | Simbol `62.2%` / Bulan `54.2%` *(raw FAIL)* | **NO-GO** (`G-02 INCONCLUSIVE`) |
| **S2_LSR** | 1 | 0 | 1 | `+0.000R` *(raw FAIL)* | `0.00` *(raw FAIL)* | `+0.000R` / `0.00` *(raw FAIL)* | `1/6` / `+0.000R` *(raw FAIL)* | `0.0R` / `0.0%` *(INCONCL)* | `0.000` (`N=47`, `prior=7`, `sesi=0`, `varSR=0.010000`) *(raw FAIL)* | `0/2 (0%)`, med `+0.000` vs `+0.000` *(raw FAIL)* | Simbol `100.0%` / Bulan `100.0%` *(raw FAIL)* | **NO-GO** (`G-02 INCONCLUSIVE`) |
| **S3_SBR** | 1 | 2 | 3 | `-0.998R` *(raw FAIL)* | `0.00` *(raw FAIL)* | `-1.046R` / `0.00` *(raw FAIL)* | `0/6` / `-1.000R` *(raw FAIL)* | `2.0R` / `0.0%` *(INCONCL)* | `0.000` (`N=47`, `prior=7`, `sesi=0`, `varSR=0.010000`) *(raw FAIL)* | `0/2 (0%)`, med `-0.998` vs `-0.499` *(raw FAIL)* | Simbol `0.0%` / Bulan `0.0%` (`net < 0`, *raw FAIL*) | **NO-GO** (`G-02 INCONCLUSIVE`) |
| **S4_WSL** | 0 | 1 | 1 | `-1.005R` *(raw FAIL)* | `0.00` *(raw FAIL)* | `-1.089R` / `0.00` *(raw FAIL)* | `0/6` / `-1.005R` *(raw FAIL)* | `1.0R` / `0.0%` *(INCONCL)* | `0.000` (`N=47`, `prior=7`, `sesi=0`, `varSR=0.010000`) *(raw FAIL)* | `0/2 (0%)`, med `+0.000` vs `+0.000` *(raw FAIL)* | Simbol `0.0%` / Bulan `0.0%` (`net < 0`, *raw FAIL*) | **NO-GO** (`G-02 INCONCLUSIVE`) |
| **S5_RMP** | 0 | 0 | 0 | `+0.000R` *(raw FAIL)* | `0.00` *(raw FAIL)* | `+0.000R` / `0.00` *(raw FAIL)* | `0/6` / `+0.000R` *(raw FAIL)* | `0.0R` / `0.0%` *(INCONCL)* | `0.000` (`N=47`, `prior=7`, `sesi=0`, `varSR=0.010000`) *(raw FAIL)* | `0/2 (0%)`, med `+0.000` vs `+0.000` *(raw FAIL)* | Simbol `0.0%` / Bulan `0.0%` (`net = 0`, *raw FAIL*) | **NO-GO** (`G-02 INCONCLUSIVE`) |

#### Detail Walk-Forward 6-Fold dengan Refit CHOP (`Universe Default` — `S1_TFP`)

| Fold | Train Window | Train Trades / E[R] | Refit CHOP (`trend_share_min` / `adx_max` / `er_max`) | Test Window | Test Trades | Test E[R] | Test PF |
|---:|---|---|---|---|:---:|---:|---:|
| 1 | `2025-02-01` → `2025-04-06` | 5 / `-0.423R` | `15.0` / `15.0` / `0.25` | `2025-04-06` → `2025-06-10` | 6 | `+1.422R` | `9.75` |
| 2 | `2025-02-01` → `2025-06-10` | 13 / `+0.788R` | `10.0` / `15.0` / `0.25` | `2025-06-10` → `2025-08-14` | 5 | `+0.751R` | `2.83` |
| 3 | `2025-02-01` → `2025-08-14` | 18 / `+0.778R` | `10.0` / `12.0` / `0.25` | `2025-08-14` → `2025-10-18` | 4 | `-0.281R` | `0.64` |
| 4 | `2025-02-01` → `2025-10-18` | 22 / `+0.585R` | `10.0` / `15.0` / `0.25` | `2025-10-18` → `2025-12-22` | 4 | `+0.458R` | `1.89` |
| 5 | `2025-02-01` → `2025-12-22` | 26 / `+0.566R` | `10.0` / `15.0` / `0.25` | `2025-12-22` → `2026-02-25` | 4 | `+0.447R` | `1.89` |
| 6 | `2025-02-01` → `2026-02-25` | 30 / `+0.550R` | `10.0` / `15.0` / `0.25` | `2026-02-25` → `2026-05-01` | 2 | `-1.006R` | `0.00` |

---

### 5.2 Run 2 — Universe 40 (39 Altcoin + `BTCUSDT`, `market_clean/`, `reports/validate_u40/validation_summary.json`)

#### Tabel Ringkasan Metrik & Gate (`Universe 40`)

| Strategi | IS Trades (min 100) | VAL Trades (min 60) | Total Trades | E[R] VAL (G-03 ≥ +0.10R) | PF VAL (G-04 ≥ 1.25) | Stress VAL E[R] / PF (G-05 >0 / ≥1.05) | WF Refit Positif / Terburuk (G-06 ≥4/6, ≥−0.15R) | MC p95 DD / P(<$40) (G-07 ≤15R, ≤5%) | DSR / N / varSR (G-08 ≥ 0.95) | Plateau G-09 (Tetangga >0 / Median vs 50% E_VAL) | Konsentrasi G-10 (Simbol ≤40% / Bulan ≤35%) | Keputusan |
|---|---:|---:|---:|---:|---:|---|---|---|---:|---|---|---|
| **S1_TFP** | 29 | 37 | 66 | `+0.015R` *(raw FAIL)* | `1.02` *(raw FAIL)* | `-0.053R` / `0.93` *(raw FAIL)* | `4/6` / `-1.005R` *(raw FAIL)* | `18.1R` / `1.7%` *(raw FAIL: p95DD>15R)* | `0.001` (`N=109`, `prior=7`, `sesi=62`, `varSR=0.042401`) *(raw FAIL)* | `0/2 (0%)`, med `-0.086` vs `+0.007` *(raw FAIL)* | Simbol `31.8%` *(ok)* / Bulan `63.3%` *(raw FAIL)* | **NO-GO** (`G-02 INCONCLUSIVE`) |
| **S2_LSR** | 1 | 0 | 1 | `+0.000R` *(raw FAIL)* | `0.00` *(raw FAIL)* | `+0.000R` / `0.00` *(raw FAIL)* | `1/6` / `+0.000R` *(raw FAIL)* | `0.0R` / `0.0%` *(INCONCL)* | `0.000` (`N=47`, `prior=7`, `sesi=0`, `varSR=0.010000`) *(raw FAIL)* | `0/2 (0%)`, med `+0.000` vs `+0.000` *(raw FAIL)* | Simbol `100.0%` / Bulan `100.0%` *(raw FAIL)* | **NO-GO** (`G-02 INCONCLUSIVE`) |
| **S3_SBR** | 3 | 3 | 6 | `-0.030R` *(raw FAIL)* | `1.01` *(raw FAIL)* | `-0.092R` / `0.92` *(raw FAIL)* | `1/6` / `-1.000R` *(raw FAIL)* | `4.0R` / `0.0%` *(INCONCL)* | `0.289` (`N=47`, `prior=7`, `sesi=0`, `varSR=0.010000`) *(raw FAIL)* | `2/2 (100%)`, med `+0.443` vs `+0.221` *(raw PASS)* | Simbol `0.0%` / Bulan `0.0%` (`net < 0`, *raw FAIL*) | **NO-GO** (`G-02 INCONCLUSIVE`) |
| **S4_WSL** | 0 | 1 | 1 | `-1.005R` *(raw FAIL)* | `0.00` *(raw FAIL)* | `-1.089R` / `0.00` *(raw FAIL)* | `0/6` / `-1.005R` *(raw FAIL)* | `1.0R` / `0.0%` *(INCONCL)* | `0.000` (`N=47`, `prior=7`, `sesi=0`, `varSR=0.010000`) *(raw FAIL)* | `0/2 (0%)`, med `+0.000` vs `+0.000` *(raw FAIL)* | Simbol `0.0%` / Bulan `0.0%` (`net < 0`, *raw FAIL*) | **NO-GO** (`G-02 INCONCLUSIVE`) |
| **S5_RMP** | 1 | 2 | 3 | `-1.002R` *(raw FAIL)* | `0.00` *(raw FAIL)* | `-1.053R` / `0.00` *(raw FAIL)* | `0/6` / `-1.002R` *(raw FAIL)* | `2.0R` / `0.0%` *(INCONCL)* | `0.000` (`N=47`, `prior=7`, `sesi=0`, `varSR=0.010000`) *(raw FAIL)* | `0/2 (0%)`, med `-0.501` vs `-0.501` *(raw FAIL)* | Simbol `0.0%` / Bulan `0.0%` (`net < 0`, *raw FAIL*) | **NO-GO** (`G-02 INCONCLUSIVE`) |

#### Detail Walk-Forward 6-Fold dengan Refit CHOP (`Universe 40` — `S1_TFP`)

| Fold | Train Window | Train Trades / E[R] | Refit CHOP (`trend_share_min` / `adx_max` / `er_max`) | Test Window | Test Trades | Test E[R] | Test PF |
|---:|---|---|---|---|:---:|---:|---:|
| 1 | `2025-02-01` → `2025-04-06` | 12 / `-0.285R` | `15.0` / `15.0` / `0.25` | `2025-04-06` → `2025-06-10` | 15 | `+0.933R` | `3.74` |
| 2 | `2025-02-01` → `2025-06-10` | 27 / `+0.392R` | `15.0` / `15.0` / `0.25` | `2025-06-10` → `2025-08-14` | 11 | `+1.107R` | `4.94` |
| 3 | `2025-02-01` → `2025-08-14` | 38 / `+0.599R` | `15.0` / `12.0` / `0.25` | `2025-08-14` → `2025-10-18` | 5 | `+0.152R` | `1.27` |
| 4 | `2025-02-01` → `2025-10-18` | 43 / `+0.547R` | `15.0` / `15.0` / `0.25` | `2025-10-18` → `2025-12-22` | 9 | `-0.688R` | `0.23` |
| 5 | `2025-02-01` → `2025-12-22` | 52 / `+0.333R` | `15.0` / `15.0` / `0.25` | `2025-12-22` → `2026-02-25` | 8 | `+0.456R` | `1.97` |
| 6 | `2025-02-01` → `2026-02-25` | 60 / `+0.349R` | `15.0` / `15.0` / `0.25` | `2026-02-25` → `2026-05-01` | 6 | `-1.005R` | `0.00` |

---

## 6. Keputusan Akhir: GO / NO-GO

- **Status Sistem / Engineering (`integrate/v1` & `master`)**: **INTEGRATED & VERIFIED** (`134/134` test release lulus, bebas konflik, deterministik, anti look-ahead, holdout tetap terkunci, safety gate `paper` + `arm=false` utuh).
- **Keputusan Validasi Strategi (`G-01..G-11`)**: **NO-GO untuk live/testnet deployment pada seluruh strategi (`S1_TFP`, `S2_LSR`, `S3_SBR`, `S4_WSL`, `S5_RMP`)**, baik pada **Universe Default** maupun **Universe 40**, karena:
  1. **G-02 (`INCONCLUSIVE`)**: Bahkan dengan perluasan ke Universe 40 dan pelonggaran CHOP `15/15/0.25`, strategi dengan sampel terbanyak (`S1_TFP`) menghasilkan `IS = 29` trade (syarat `≥ 100`) dan `VAL = 37` trade (syarat `≥ 60`) pada timeframe `15m`, sementara `S2..S5` hanya menghasilkan `1..6` trade total.
  2. **Performa OOS/VAL & Ketahanan (`G-03..G-10`)**:
     - Pada **Universe Default**, `S1_TFP` memiliki expectancy VAL positif (`E[R] = +0.129R`, stress `+0.068R / PF 1.10`, WF `4/6` fold positif, MC `p95DD = 11.1R`), namun belum memenuhi `PF VAL ≥ 1.25` (`1.20`), fold terburuk WF (`-1.006R < -0.15R`), DSR (`0.040 < 0.95`), dan batas konsentrasi G-10 (`62.2%` simbol / `54.2%` bulan).
     - Pada **Universe 40**, penambahan 28 altcoin menaikkan jumlah trade `S1_TFP` dari `30` ke `66` (`+120%`) dan menurunkan konsentrasi simbol ke `31.8%` (`≤ 40%`), tetapi mendilusi expectancy VAL ke `+0.015R` (`PF = 1.02`, stress `-0.053R / PF 0.93`) dan menaikkan `MC p95 DD` ke `18.1R`.
  3. **G-11 (`LOCKED`)**: Holdout `2026-05-01..2026-10-31` tetap terkunci dan tidak boleh dibuka sebelum ada kandidat strategi yang lulus `G-01..G-10`.
