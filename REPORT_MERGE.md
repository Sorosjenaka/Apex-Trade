# REPORT MERGE & AUDIT DOSSIER — Tugas T-G2 (`integrate/v1` → `master`)

Dokumen ini disusun sebagai laporan audit lengkap untuk **Grok** atas pengerjaan **FASE 0** dan **FASE 1 (Tugas T-G2)** pada repositori utama `C:\Users\FATHUR\Downloads\apex-scalper-rs`.

---

## 0. Checklist Audit Kepatuhan Tugas & Aturan Mutlak

| Item Tugas / Aturan | Status | Bukti File / Baris / Commit |
|---|---|---|
| **FASE 0**: Ringkasan struktur crate, status branch/worktree, potensi konflik, aturan keamanan | **LULUS** | `C:\Users\FATHUR\Downloads\apex-scalper-rs\UNDERSTANDING.md` |
| **T-G2 #1**: Branch `integrate/v1` dari `master`, merge `codex/strategies`, `hermes/validation`, `codex/sample` (`8cef9b5`), resolusi `engine.rs` & `market_reader/lib.rs`, Universe 40 & entry 5m OFF di default | **LULUS** | Merge commits `37d80c1`, `d55c9b4`, `8cef9b5`; `config/engine.toml` (`universe = []`), `config/strategies.toml` (`research_entry_5m = false`) |
| **T-G2 #2**: Samakan definisi `VAL` dengan `Rules.md` (`VAL` mulai `2025-07-01`) di gate runner; hapus pecahan `2026-01-01` | **LULUS** | `apps/apex/src/bt.rs` baris `230–232` (`VAL_FROM = "2025-07-01"`, `FIT_TO = VAL_FROM`, `OOS_FROM = VAL_FROM`), baris `725–729`, unit test `validation_boundary_matches_rules` baris `851–858` |
| **T-G2 #3**: Masukkan threshold CHOP `15 / 15 / 0.25` ke config default; risk rule lain tidak diubah | **LULUS** | `crates/market_reader/src/aggregate.rs` baris `135–145`, `config/screener.toml` baris `1–4`, `apps/apex/src/cfg.rs` baris `26`, `config/risk.toml` tidak berubah |
| **T-G2 #4**: Perbaiki DSR: SR per trial dari `R` per trade, buang trial `< 10` trade | **LULUS** | `apps/apex/src/bt.rs` baris `279–293` (`trial_sr`), unit test `trial_sr_drops_small_samples` baris `938–944`, `crates/backtest/src/gates.rs` baris `266–314` (`count_strategy_trials` syarat `>= 10`) |
| **T-G2 #5**: Walk-forward anchored 6-fold dengan refit CHOP per fold hanya di data train tiap fold | **LULUS** | `crates/backtest/src/gates.rs` baris `195–218` (`refit_threshold_from_train`), `apps/apex/src/bt.rs` baris `320–379` (`refit_chop_fold`) & baris `695–716` |
| **T-G2 #6**: Satu `target-msvc`, jalankan `cargo test --workspace --release` sekali; wajib lulus | **LULUS** | `134 passed, 0 failed` (`.\cargo-msvc.cmd test --workspace --release`) |
| **T-G2 #7**: Jalankan `apex validate --strategies S1,S2,S3,S4,S5` pada `2025-02-01` → `2026-05-01` tanpa holdout (2x: Universe Default & Universe 40) | **LULUS** | `reports/validate_default/validation_summary.json` & `reports/validate_u40/validation_summary.json` |
| **T-G2 #8**: Fast-forward `master` ke `integrate/v1` (lokal, tanpa `git push`) | **LULUS** | `master` == `integrate/v1` |
| **Aturan Keamanan**: Tidak kirim order ke Binance, `arm = false`, `mode = "paper"`, tidak baca `.env`/API key, holdout `2026-05-01..2026-10-31` terkunci | **PATUH 100%** | `config/engine.toml` (`mode = "paper"`, `arm = false`), `g11 = "LOCKED"` |

---

## 1. Riwayat Branch & Commit Hash

| Branch / Tahap | Commit Hash | Deskripsi |
|---|---|---|
| `master` (awal) | `56a3f03` | `fix(test): align market reader test expectations with relaxed chop thresholds` |
| `codex/strategies` | `5bf3982` | `feat(strategies,backtest,oms): add S1 OFCC watch, S3 magnet filter, and S5 kNN grader` |
| Merge `codex/strategies` | `37d80c1` | `merge(integrate/v1): codex/strategies into integrate/v1` |
| `hermes/validation` | `0ef3793` | `feat(validation): enforce G-02 inconclusive rule and honesty fixes for G-01..G-11` |
| Merge `hermes/validation` | `d55c9b4` | `merge(integrate/v1): hermes/validation into integrate/v1` |
| `codex/sample` | `8cef9b5` | `feat(sample): support 40-symbol universe and 5m S1/S3 entry research` |
| `integrate/v1` (T-G2 kode) | `594affb` | `feat(validation,integration): complete T-G2 merge, VAL=2025-07-01, CHOP 15/15/0.25, DSR R-per-trade>=10, and WF per-fold CHOP refit` |
| `integrate/v1` & `master` | `14a3e6d` (diperbarui ke HEAD) | Fast-forward `master` ke `integrate/v1` (lokal) |

---

## 2. Detail Resolusi Konflik & Integrasi Fitur

### 2.1 `crates/backtest/src/engine.rs`
Seluruh fitur dari ketiga branch dipertahankan secara utuh:
1. **Dari `codex/strategies` (`5bf3982`)**:
   - `Pending.ofcc_watch: Option<S1Watch>` (baris `169`, `556–596`, `749–754`): sinyal `S1_TFP` terlebih dahulu masuk fase observasi OFCC candle close (`Outcome::Waiting / Confirmed / Rejected / Expired`). Saat `Confirmed`, sistem memverifikasi harga close tidak menembus limit post-only (`crosses`), mengecek ulang volume gate bar terakhir (`current_volume_ok`), regime reader, dan risk gate sebelum mengaktifkan limit order.
   - `S5_RMP` kNN causal grader (`knn_training_start`, `knn_training_config`, `knn_sample`, `knn::grade` pada baris `384–414`, `477–483`, `651–654`, `767–772`): melatih seed kNN hanya dari replay IS pra-`cfg.start` dengan biaya baseline (`fee_mult = 1.0`, `funding_mult = 1.0`, `extra_slip = 0.0`), menolak overlap terhadap holdout terkunci (`2026-05-01..2026-10-31`), dan hanya menambahkan sampel ke memori setelah bar exit tertutup (`closed_ms < t_close`).
2. **Dari `hermes/validation` (`0ef3793`)**:
   - `BacktestConfig.chop: ChopThresholds` (baris `72`, `109`, `446`) diteruskan ke `RegimeTimeline::build(&mkt.btc_m15, &mkt.btc_h1, mkt.dom_h1.as_deref(), &alts, &close_times, cfg.cap_on_proxy, cfg.chop)`.
3. **Dari `codex/sample` (`8cef9b5`)**:
   - `BacktestConfig.entry_tf: Tf` (default `Tf::M15`; `Tf::M5` khusus riset `S1/S3` dengan `RegimeMode::Reader`), validasi bar `5m` bebas gap (`valid_five_minute_bars`), keputusan reader tetap pada penutupan bar `15m` (`closed_reader_decision`), syarat umur listing kontrak minimal 30 hari (`listed_long_enough` sesuai Rules §4.3), dan pengecekan `symbol_trending_at_open` serta `account_blocks_fill` pada saat fill.

### 2.2 `crates/market_reader/src/lib.rs` & `crates/market_reader/src/aggregate.rs`
- `pub use aggregate::{aggregate, AltSample, ChopThresholds, RawVote};` diekspor di `crates/market_reader/src/lib.rs`.
- `ChopThresholds::default()` di `crates/market_reader/src/aggregate.rs` (baris `141–145`) ditetapkan ke:
  ```rust
  Self { adx_max: 15.0, er_max: 0.25, trend_share_min: 15.0 }
  ```
- `config/screener.toml` (baris `1–4`) memuat:
  ```toml
  [chop]
  adx_max = 15.0
  er_max = 0.25
  trend_share_min = 15.0
  ```
- `apps/apex/src/cfg.rs` (`ScreenerFile.chop`, baris `26`) membaca `[chop]` dari `config/screener.toml` dan meneruskannya ke `BacktestConfig.chop` di `apps/apex/src/bt.rs` (`make_cfg`, baris `124`).

### 2.3 Konfigurasi Default Tetap Aman
- `config/engine.toml`: `mode = "paper"`, `arm = false`, `universe = []` (Universe 40 OFF di default).
- `config/strategies.toml`: `research_entry_5m = false` (Entry 5m OFF di default), `s5_knn_enabled = false`.
- `config/risk.toml`: Tidak ada perubahan pada parameter risiko (`risk_usd = 1.0`, `leverage = 25`, `rr = 2.0`, `sl_min_pct = 0.02`, `sl_max_pct = 0.04`, `max_positions = 2`, `cooldown_after_sl_sec = 3600`, `daily_sl_limit = 6`, `daily_loss_limit_usd = 6.0`).

---

## 3. Verifikasi Perbaikan Gate Runner (Tugas T-G2 #2, #4, #5)

1. **Definisi `VAL` = `2025-07-01` (`apps/apex/src/bt.rs`)**:
   - Seluruh referensi ke `2026-01-01` telah dihapus (`Select-String "2026-01-01"` pada `apps/` dan `crates/` menghasilkan `0` kecocokan).
   - `VAL_FROM = "2025-07-01"`, `FIT_TO = VAL_FROM`, `OOS_FROM = VAL_FROM` (baris `230–232`).
   - Segmen `IS` adalah `[2025-02-01, 2025-07-01)` dan segmen `VAL` adalah `[2025-07-01, 2026-05-01)` (baris `725–729`).
2. **Perbaikan DSR (`apps/apex/src/bt.rs` & `crates/backtest/src/gates.rs`)**:
   - `trial_sr(rs: &[f64]) -> Option<f64>` (baris `279–281`) membuang trial dengan `rs.len() < 10` (`None`) dan menghitung `sharpe_ratio(rs)` dari vektor `R` per trade (`t.r`).
   - `append_ledger_with` mencatat `strategy_trades` per strategi ke `trials_ledger.jsonl`, dan `count_strategy_trials` (`crates/backtest/src/gates.rs` baris `266–314`) hanya menghitung trial prior yang memiliki `>= 10` trade.
3. **Walk-Forward Anchored 6-Fold dengan Refit CHOP per Fold (`crates/backtest/src/gates.rs` & `apps/apex/src/bt.rs`)**:
   - Untuk setiap fold `f` (`anchored_folds(c.start, c.end, 6)`), `refit_chop_fold(&mkt, &c, f.train_start, f.train_end)` melakukan sweep `TREND_GRID`, `ADX_GRID`, dan `ER_GRID` **hanya pada window train `[f.train_start, f.train_end)`**, memilih threshold melalui `refit_threshold_from_train`, lalu menjalankan satu evaluasi OOS fold pada `[f.test_start, f.test_end)` menggunakan `refit_chop` tersebut.

---

## 4. Hasil `cargo test --workspace --release` (`target-msvc`)

Perintah: `.\cargo-msvc.cmd test --workspace --release`

| Crate / Target Test | Passed | Failed | Ignored | Durasi |
|---|---:|---:|---:|---:|
| `apex` (`apps/apex/src/main.rs`) | 8 | 0 | 0 | 0.00s |
| `apex-backtest` (`crates/backtest/src/lib.rs`) | 21 | 0 | 0 | 4.05s |
| `apex-core` (`crates/core/src/lib.rs`) | 23 | 0 | 0 | 0.02s |
| `apex-data` (`crates/data/src/lib.rs`) | 6 | 0 | 0 | 0.02s |
| `apex-indicators` (`crates/indicators/src/lib.rs`) | 6 | 0 | 0 | 0.00s |
| `apex-market-reader` (`crates/market_reader/src/lib.rs`) | 18 | 0 | 0 | 0.01s |
| `apex-oms` (`crates/oms/src/lib.rs`) | 7 | 0 | 0 | 0.00s |
| `apex-oms` (`crates/oms/tests/state_machine.rs`) | 20 | 0 | 0 | 0.13s |
| `apex-screener` (`crates/screener/src/lib.rs`) | 8 | 0 | 0 | 0.02s |
| `apex-strategies` (`crates/strategies/src/lib.rs`) | 17 | 0 | 0 | 1.67s |
| **TOTAL WORKSPACE** | **134** | **0** | **0** | **PASS (100%)** |

---

## 5. Hasil `apex validate --strategies S1,S2,S3,S4,S5` (`2025-02-01` → `2026-05-01`, Tanpa Holdout)

### 5.1 Run 1 — Universe Default (11 Altcoin + `BTCUSDT`)
- **Perintah**:
  ```powershell
  .\target-msvc\release\apex.exe validate --strategies S1,S2,S3,S4,S5 --from 2025-02-01 --to 2026-05-01 --data data --out reports\validate_default
  ```
- **Output JSON**: `C:\Users\FATHUR\Downloads\apex-scalper-rs\reports\validate_default\validation_summary.json`

#### Tabel Gate G-01..G-11 per Strategi (`Universe Default`)

| Strategi | IS Trades (min 100) | VAL Trades (min 60) | Full Trades (`2025-02..2026-05`) | Full E[R] / PF / Net USD | E[R] VAL (G-03 ≥ +0.10R) | PF VAL (G-04 ≥ 1.25) | Stress VAL E[R] / PF (G-05 >0 / ≥1.05) | WF Refit Positif / Terburuk (G-06 ≥4/6, ≥−0.15R) | MC p95 DD / P(<$40) / `n_year` (G-07 ≤15R, ≤5%) | DSR / N (`prior`+`sesi`+40) / `varSR` (G-08 ≥ 0.95) | Plateau G-09 (`Tetangga >0`, `Median` vs `50% E_VAL`) | Konsentrasi G-10 (`Simbol ≤40%` / `Bulan ≤35%`) | G-11 Holdout | Verdict Akhir |
|---|---:|---:|---:|---|---:|---:|---|---|---|---:|---|---|---|---|
| **S1_TFP** | `12` | `18` | `30` | `+0.355R` / `1.66` / `+$10.54` | `+0.129R` *(raw PASS)* | `1.20` *(raw FAIL)* | `+0.068R` / `1.10` *(raw PASS)* | `4/6` / `-1.006R` *(raw FAIL)* | `11.1R` / `0.0%` / `22` *(raw PASS)* | `0.040` (`N=107`: `7+60+40`, `varSR=0.048995`) *(raw FAIL)* | `1/2 (50%)`, med `+0.114` vs `+0.064` *(raw FAIL)* | Simbol `62.2%` / Bulan `54.2%` *(raw FAIL)* | `LOCKED` | **NO-GO** (`G-02 INCONCLUSIVE`) |
| **S2_LSR** | `1` | `0` | `1` | `+1.867R` / `inf` / `+$1.79` | `+0.000R` *(raw FAIL)* | `0.00` *(raw FAIL)* | `+0.000R` / `0.00` *(raw FAIL)* | `1/6` / `+0.000R` *(raw FAIL)* | `0.0R` / `0.0%` / `1` *(INCONCL)* | `0.000` (`N=47`: `7+0+40`, `varSR=0.010000`) *(raw FAIL)* | `0/2 (0%)`, med `+0.000` vs `+0.000` *(raw FAIL)* | Simbol `100.0%` / Bulan `100.0%` *(raw FAIL)* | `LOCKED` | **NO-GO** (`G-02 INCONCLUSIVE`) |
| **S3_SBR** | `1` | `2` | `3` | `-0.999R` / `0.00` / `-$2.99` | `-0.998R` *(raw FAIL)* | `0.00` *(raw FAIL)* | `-1.046R` / `0.00` *(raw FAIL)* | `0/6` / `-1.000R` *(raw FAIL)* | `2.0R` / `0.0%` / `2` *(INCONCL)* | `0.000` (`N=47`: `7+0+40`, `varSR=0.010000`) *(raw FAIL)* | `0/2 (0%)`, med `-0.998` vs `-0.499` *(raw FAIL)* | Simbol `0.0%` / Bulan `0.0%` (`net < 0`, *raw FAIL*) | `LOCKED` | **NO-GO** (`G-02 INCONCLUSIVE`) |
| **S4_WSL** | `0` | `1` | `1` | `-1.005R` / `0.00` / `-$1.00` | `-1.005R` *(raw FAIL)* | `0.00` *(raw FAIL)* | `-1.089R` / `0.00` *(raw FAIL)* | `0/6` / `-1.005R` *(raw FAIL)* | `1.0R` / `0.0%` / `1` *(INCONCL)* | `0.000` (`N=47`: `7+0+40`, `varSR=0.010000`) *(raw FAIL)* | `0/2 (0%)`, med `+0.000` vs `+0.000` *(raw FAIL)* | Simbol `0.0%` / Bulan `0.0%` (`net < 0`, *raw FAIL*) | `LOCKED` | **NO-GO** (`G-02 INCONCLUSIVE`) |
| **S5_RMP** | `0` | `0` | `0` | `+0.000R` / `0.00` / `+$0.00` | `+0.000R` *(raw FAIL)* | `0.00` *(raw FAIL)* | `+0.000R` / `0.00` *(raw FAIL)* | `0/6` / `+0.000R` *(raw FAIL)* | `0.0R` / `0.0%` / `1` *(INCONCL)* | `0.000` (`N=47`: `7+0+40`, `varSR=0.010000`) *(raw FAIL)* | `0/2 (0%)`, med `+0.000` vs `+0.000` *(raw FAIL)* | Simbol `0.0%` / Bulan `0.0%` (`net = 0`, *raw FAIL*) | `LOCKED` | **NO-GO** (`G-02 INCONCLUSIVE`) |

#### Detail Walk-Forward 6-Fold dengan Refit CHOP per Fold (`Universe Default`)

| Strategi | Fold 1 (`2025-04-06..06-10`) | Fold 2 (`2025-06-10..08-14`) | Fold 3 (`2025-08-14..10-18`) | Fold 4 (`2025-10-18..12-22`) | Fold 5 (`2025-12-22..2026-02-25`) | Fold 6 (`2026-02-25..05-01`) |
|---|---|---|---|---|---|---|
| **S1_TFP** | `+1.422R` (6 tr, refit `15/15/0.25`) | `+0.751R` (5 tr, refit `10/15/0.25`) | `-0.281R` (4 tr, refit `10/12/0.25`) | `+0.458R` (4 tr, refit `10/15/0.25`) | `+0.447R` (4 tr, refit `10/15/0.25`) | `-1.006R` (2 tr, refit `10/15/0.25`) |
| **S2_LSR** | `+1.867R` (1 tr, refit `15/15/0.25`) | `+0.000R` (0 tr, refit `10/15/0.25`) | `+0.000R` (0 tr, refit `10/12/0.25`) | `+0.000R` (0 tr, refit `10/15/0.25`) | `+0.000R` (0 tr, refit `10/15/0.25`) | `+0.000R` (0 tr, refit `10/15/0.25`) |
| **S3_SBR** | `-1.000R` (1 tr, refit `15/15/0.25`) | `+0.000R` (0 tr, refit `15/15/0.25`) | `+0.000R` (0 tr, refit `15/15/0.25`) | `+0.000R` (0 tr, refit `15/15/0.25`) | `-0.998R` (2 tr, refit `15/15/0.25`) | `+0.000R` (0 tr, refit `15/15/0.25`) |
| **S4_WSL** | `+0.000R` (0 tr, refit `15/15/0.25`) | `+0.000R` (0 tr, refit `15/15/0.25`) | `+0.000R` (0 tr, refit `15/15/0.25`) | `+0.000R` (0 tr, refit `15/15/0.25`) | `+0.000R` (0 tr, refit `15/15/0.25`) | `-1.005R` (1 tr, refit `15/15/0.25`) |
| **S5_RMP** | `+0.000R` (0 tr, refit `15/15/0.25`) | `+0.000R` (0 tr, refit `15/15/0.25`) | `+0.000R` (0 tr, refit `15/15/0.25`) | `+0.000R` (0 tr, refit `15/15/0.25`) | `+0.000R` (0 tr, refit `15/15/0.25`) | `+0.000R` (0 tr, refit `15/15/0.25`) |

---

### 5.2 Run 2 — Universe 40 (39 Altcoin + `BTCUSDT`)
- **Perintah**:
  ```powershell
  .\target-msvc\release\apex.exe --config reports\config_u40 validate --strategies S1,S2,S3,S4,S5 --from 2025-02-01 --to 2026-05-01 --data "C:\Users\FATHUR\Downloads\apex-wt-codex\research\tc3_data\market_clean" --out reports\validate_u40
  ```
- **Output JSON**: `C:\Users\FATHUR\Downloads\apex-scalper-rs\reports\validate_u40\validation_summary.json`

#### Tabel Gate G-01..G-11 per Strategi (`Universe 40`)

| Strategi | IS Trades (min 100) | VAL Trades (min 60) | Full Trades (`2025-02..2026-05`) | Full E[R] / PF / Net USD | E[R] VAL (G-03 ≥ +0.10R) | PF VAL (G-04 ≥ 1.25) | Stress VAL E[R] / PF (G-05 >0 / ≥1.05) | WF Refit Positif / Terburuk (G-06 ≥4/6, ≥−0.15R) | MC p95 DD / P(<$40) / `n_year` (G-07 ≤15R, ≤5%) | DSR / N (`prior`+`sesi`+40) / `varSR` (G-08 ≥ 0.95) | Plateau G-09 (`Tetangga >0`, `Median` vs `50% E_VAL`) | Konsentrasi G-10 (`Simbol ≤40%` / `Bulan ≤35%`) | G-11 Holdout | Verdict Akhir |
|---|---:|---:|---:|---|---:|---:|---|---|---|---:|---|---|---|---|
| **S1_TFP** | `29` | `37` | `66` | `+0.226R` / `1.39` / `+$14.74` | `+0.015R` *(raw FAIL)* | `1.02` *(raw FAIL)* | `-0.053R` / `0.93` *(raw FAIL)* | `4/6` / `-1.005R` *(raw FAIL)* | `18.1R` / `1.7%` / `44` *(raw FAIL: DD>15R)* | `0.001` (`N=109`: `7+62+40`, `varSR=0.042401`) *(raw FAIL)* | `0/2 (0%)`, med `-0.086` vs `+0.007` *(raw FAIL)* | Simbol `31.8%` *(ok)* / Bulan `63.3%` *(raw FAIL)* | `LOCKED` | **NO-GO** (`G-02 INCONCLUSIVE`) |
| **S2_LSR** | `1` | `0` | `1` | `+1.867R` / `inf` / `+$1.79` | `+0.000R` *(raw FAIL)* | `0.00` *(raw FAIL)* | `+0.000R` / `0.00` *(raw FAIL)* | `1/6` / `+0.000R` *(raw FAIL)* | `0.0R` / `0.0%` / `1` *(INCONCL)* | `0.000` (`N=47`: `7+0+40`, `varSR=0.010000`) *(raw FAIL)* | `0/2 (0%)`, med `+0.000` vs `+0.000` *(raw FAIL)* | Simbol `100.0%` / Bulan `100.0%` *(raw FAIL)* | `LOCKED` | **NO-GO** (`G-02 INCONCLUSIVE`) |
| **S3_SBR** | `3` | `3` | `6` | `-0.035R` / `0.98` / `-$0.09` | `-0.030R` *(raw FAIL)* | `1.01` *(raw FAIL)* | `-0.092R` / `0.92` *(raw FAIL)* | `1/6` / `-1.000R` *(raw FAIL)* | `4.0R` / `0.0%` / `4` *(INCONCL)* | `0.289` (`N=47`: `7+0+40`, `varSR=0.010000`) *(raw FAIL)* | `2/2 (100%)`, med `+0.443` vs `+0.221` *(raw PASS)* | Simbol `0.0%` / Bulan `0.0%` (`net < 0`, *raw FAIL*) | `LOCKED` | **NO-GO** (`G-02 INCONCLUSIVE`) |
| **S4_WSL** | `0` | `1` | `1` | `-1.005R` / `0.00` / `-$1.00` | `-1.005R` *(raw FAIL)* | `0.00` *(raw FAIL)* | `-1.089R` / `0.00` *(raw FAIL)* | `0/6` / `-1.005R` *(raw FAIL)* | `1.0R` / `0.0%` / `1` *(INCONCL)* | `0.000` (`N=47`: `7+0+40`, `varSR=0.010000`) *(raw FAIL)* | `0/2 (0%)`, med `+0.000` vs `+0.000` *(raw FAIL)* | Simbol `0.0%` / Bulan `0.0%` (`net < 0`, *raw FAIL*) | `LOCKED` | **NO-GO** (`G-02 INCONCLUSIVE`) |
| **S5_RMP** | `1` | `2` | `3` | `-1.001R` / `0.00` / `-$3.00` | `-1.002R` *(raw FAIL)* | `0.00` *(raw FAIL)* | `-1.053R` / `0.00` *(raw FAIL)* | `0/6` / `-1.002R` *(raw FAIL)* | `2.0R` / `0.0%` / `2` *(INCONCL)* | `0.000` (`N=47`: `7+0+40`, `varSR=0.010000`) *(raw FAIL)* | `0/2 (0%)`, med `-0.501` vs `-0.501` *(raw FAIL)* | Simbol `0.0%` / Bulan `0.0%` (`net < 0`, *raw FAIL*) | `LOCKED` | **NO-GO** (`G-02 INCONCLUSIVE`) |

#### Detail Walk-Forward 6-Fold dengan Refit CHOP per Fold (`Universe 40`)

| Strategi | Fold 1 (`2025-04-06..06-10`) | Fold 2 (`2025-06-10..08-14`) | Fold 3 (`2025-08-14..10-18`) | Fold 4 (`2025-10-18..12-22`) | Fold 5 (`2025-12-22..2026-02-25`) | Fold 6 (`2026-02-25..05-01`) |
|---|---|---|---|---|---|---|
| **S1_TFP** | `+0.933R` (15 tr, refit `15/15/0.25`) | `+1.107R` (11 tr, refit `15/15/0.25`) | `+0.152R` (5 tr, refit `15/12/0.25`) | `-0.688R` (9 tr, refit `15/15/0.25`) | `+0.456R` (8 tr, refit `15/15/0.25`) | `-1.005R` (6 tr, refit `15/15/0.25`) |
| **S2_LSR** | `+1.867R` (1 tr, refit `15/15/0.25`) | `+0.000R` (0 tr, refit `10/15/0.25`) | `+0.000R` (0 tr, refit `10/12/0.25`) | `+0.000R` (0 tr, refit `10/15/0.25`) | `+0.000R` (0 tr, refit `10/15/0.25`) | `+0.000R` (0 tr, refit `10/15/0.25`) |
| **S3_SBR** | `-0.039R` (3 tr, refit `15/15/0.25`) | `-0.977R` (1 tr, refit `15/15/0.25`) | `-1.000R` (1 tr, refit `15/15/0.25`) | `+0.000R` (0 tr, refit `15/15/0.25`) | `+1.886R` (1 tr, refit `15/15/0.25`) | `+0.000R` (0 tr, refit `15/15/0.25`) |
| **S4_WSL** | `+0.000R` (0 tr, refit `15/15/0.25`) | `+0.000R` (0 tr, refit `15/15/0.25`) | `+0.000R` (0 tr, refit `15/15/0.25`) | `+0.000R` (0 tr, refit `15/15/0.25`) | `+0.000R` (0 tr, refit `15/15/0.25`) | `-1.005R` (1 tr, refit `15/15/0.25`) |
| **S5_RMP** | `-1.000R` (1 tr, refit `15/15/0.25`) | `+0.000R` (0 tr, refit `15/15/0.25`) | `-1.002R` (2 tr, refit `15/15/0.25`) | `+0.000R` (0 tr, refit `15/15/0.25`) | `+0.000R` (0 tr, refit `15/15/0.25`) | `+0.000R` (0 tr, refit `15/15/0.25`) |

---

## 6. Kesimpulan GO / NO-GO

- **Engineering & Integrasi**: **GO / SELESAI** — Seluruh kode dari `codex/strategies`, `hermes/validation`, dan `codex/sample` (`8cef9b5`) telah terintegrasi ke `integrate/v1` dan di-fast-forward ke `master` lokal, dengan `134/134` test release lulus di `target-msvc`.
- **Validasi Strategi (`S1..S5`)**: **NO-GO** untuk kelima strategi pada kedua universe:
  1. **G-02 (`INCONCLUSIVE`)**: Sampel tertinggi (`S1_TFP` pada Universe 40: `IS = 29`, `VAL = 37`, total `66` trade) masih di bawah ambang `IS ≥ 100` dan `VAL ≥ 60`.
  2. **Degradasi Edge pada Universe 40**: Pada Universe Default, `S1_TFP` mencetak `VAL E[R] = +0.129R` (`PF = 1.20`, stress `+0.068R / PF 1.10`), tetapi ekspansi ke Universe 40 menurunkan `VAL E[R]` menjadi `+0.015R` (`PF = 1.02`, stress `-0.053R / PF 0.93`) dan meningkatkan `MC p95 DD` ke `18.1R`.
  3. **Walk-Forward & DSR**: Meski `S1_TFP` mencapai `4/6` fold positif setelah refit CHOP per fold di train, fold ke-6 (`2026-02-25..2026-05-01`) mengalami drawdown `-1.006R` (Default) / `-1.005R` (Universe 40), melanggar batas fold terburuk `≥ -0.15R`, dan `DSR` masih jauh di bawah `0.95` (`0.040` pada Default, `0.001` pada Universe 40).
  4. **Holdout (`G-11`)**: Tetap **LOCKED** (`2026-05-01..2026-10-31`) sesuai aturan.
