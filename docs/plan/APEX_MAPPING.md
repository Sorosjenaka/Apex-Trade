# APEX_MAPPING — Dari apex-trading-system ke Apex Scalper RS

> Versi 1.0 · 5 Nov 2026 (WIB).
> Dasar: source apex (diekstrak ke `/workspace/apex/src`) dan review statis `REVIEW-apex-trading-system.md` (temuan C1–C8, H1–H9, M1–M10, L1–L7).
> Keputusan per file: **PAKAI** (salin dengan perubahan kecil), **PORT** (logika dipindah ke crate baru dengan perubahan berarti), **REFERENSI** (dibaca untuk parity/oracle, tidak dikompilasi), **BUANG** (tidak masuk proyek baru).
> Prinsip: tidak ada kode Python di jalur live. Python hanya boleh di `research/` sebagai oracle pembanding.

## 1. rust-engine (Cargo: serde, serde_json)

| File (baris) | Isi | Keputusan | Tujuan di proyek baru | Perubahan wajib |
|---|---|---|---|---|
| `src/scalp_portfolio.rs` (1240) | Replay portofolio: Config (balance 40, risk 1, RR 2, lev 25, max_pos 2, daily_sl_limit 6, cooldown 3,6e6 ms, pending TTL 1h, fee 0,02/0,05%, slip 0,02%), SL-first, gap pakai open lebih buruk, funding, minQty/minNotional | **PORT** (aset paling berharga) | `crates/backtest/src/portfolio.rs` | (1) `SYMBOLS` hardcoded BTC/ETH → universe dari config; (2) initial_balance 40 → **60**; (3) cooldown "antar fill" → **60 menit setelah exit SL, global** (A-03); (4) stop harian: 6 SL **atau** realized ≤ −$6 (A-02); (5) skip risk aktual < $0,80; (6) bar 15m/1h selain 5m; (7) keluarkan `trades/hari` & holding time; (8) ganti `is_multiple_of` agar build di Rust ≥ 1.80 atau kunci toolchain 1.87 di `rust-toolchain.toml` |
| `tests/scalp_portfolio.rs` (488) | Test replay | **PORT** | `crates/backtest/tests/` | Tambah test T-2.3 (sizing variabel), T-2.4 (cooldown SL), T-2.5 (stop harian) |
| `tests/scalp_causality.rs` (59) | Prefix test anti look-ahead | **PAKAI** | `crates/backtest/tests/causality.rs` | Generalisasi ke semua generator S1–S5 + screener |
| `src/validation.rs` (423) | Net-EV gate, limit fill ratio, slippage dinamis, stress preset, compare pre-cut (look-ahead), excursions MAE/MFE, CPCV groups | **PORT** | `crates/validate/` | Tambah: walk-forward 6 fold, Monte Carlo bootstrap, Deflated Sharpe (N dari `trials_ledger`), uji plateau, gate konsentrasi simbol |
| `tests/validation_gates.rs` (165) | Test gate | **PAKAI** | `crates/validate/tests/` | — |
| `src/indicators.rs` (215) | EMA, RMA, RSI, TR, ATR, MACD, AO, ADX Wilder, slope | **PAKAI** | `crates/features/src/basic.rs` | Tambah ER, CHOP, SRV, Bilateral Filter, Decycler, DistRank, percent-rank, delta |
| `src/pivots.rs` (164) | Pivot terkonfirmasi + divergence | **PAKAI** | `crates/features/src/pivots.rs` | Pastikan field `ready_time` = close bar t+len |
| `src/smc_generators.rs` (713) | fvg_mitigation, absorption_delta, volume_profile, trend_stack, distrank_knn, liquidity_sweep, rolling_delta | **PORT** | `crates/strategies/` (S1, S2, S5) | Audit kausalitas tiap fungsi (prefix test). `distrank_knn` wajib memakai label yang sudah selesai (T-2.9) |
| `src/fibonacci.rs` (85) | Fib aktif | **PAKAI** | `crates/features/src/fib.rs` (S5) | — |
| `src/data.rs` (275) | Load CSV klines | **PORT** | `crates/data/src/csv.rs` | Tambah kolom takerBuyBaseVolume; format Parquet opsional |
| `src/domain.rs` (129), `signal_types.rs` (124), `smc_ctl.rs` (111) | Tipe Candle/Timeframe, sinyal CTL | **PORT** sebagian | `crates/core/src/types.rs` | Hanya tipe; strategi CTL lama tidak dipakai |
| `src/risk.rs` (68) | `calculate_user_risk`: SL 2–6%, risk = 1,67% equity bila ≥ $120 | **BUANG / tulis ulang** | `crates/risk/src/sizing.rs` | Melanggar GR-1 (risk harus tetap $1), GR-3 (maks 4%, bukan 6%), dan tidak memasukkan fee/slip/stepSize. Ditulis ulang sesuai FEASIBILITY §1 |
| `src/backtest.rs` (417) | Backtest lama | **BUANG** | — | Look-ahead H1 (baris 199–207), posisi terbuka di akhir tidak dihitung (H2) |
| `src/council.rs`, `macro_reader.rs`, `macro_history.rs`, `reader.rs` | Voting agen, makro | **BUANG** | — | Tidak relevan; hanya menambah trial |
| `src/main.rs`, `bin/scalp-replay.rs` | CLI | **REFERENSI** | `apps/research-cli` | CLI baru `apex-research replay|validate|screen` |

## 2. research/tv7_scalp_20261104 (Python)

| File | Keputusan | Catatan |
|---|---|---|
| `RESEARCH_RULES.md` | **PAKAI** sebagai dasar Rules.md §4 | Disiplin IS/OOS, rumus sizing netto, larangan memotong SL |
| `features.py`, `signals.py`, `data_io.py`, `prepare.py` | **REFERENSI (oracle)** | Output Rust S3 harus identik dengan "Strategi 1 Structure Pressure" Python pada dataset sama (T-2.8) |
| `validation_stats.py`, `run_research.py`, `run_spec.json` | **REFERENSI** | run_spec v2 → `config/research.toml` |
| `charts.py` (674) | **BUANG** | Grafik dibuat ulang di dashboard |
| `source_manifest.json` | **PAKAI** | Digabung dengan `pine/CATALOG.md` (sha256) |

## 3. Python live stack (core/, agents/, analytics/, data/, config/)

| File | Keputusan | Alasan (temuan review) | Pengganti |
|---|---|---|---|
| `core/execution.py` (425) | **BUANG** (pelajaran saja) | C1 SL/TP hanya di memori, C2 FILLED tanpa RESULT, C3 posisi dihapus saat close gagal, C6 tanpa step/tick/minNotional, H5 PnL ×lev dua kali | `crates/exec` (state machine order + algoOrder) |
| `core/orchestrator.py` (1009) | **BUANG** | C5 tanpa reconcile/ARM, H6 data basi, M5, M7 bar belum close | `apps/engine` (tokio) |
| `core/protections.py` (198) | **PORT** konsep | Cooldown, SL guard, DD breaker berguna; H9 equity hardcode $10.000 | `crates/risk/src/guards.rs` (equity dari exchange) |
| `core/fusion.py`, `agents/*` (14 agen + LLM), `core/llm_manager.py`, `core/memory.py` | **BUANG** | C7 LLM Drex bisa membuka trade. Proyek baru: **tidak ada LLM di jalur keputusan** | — |
| `analytics/*` (learning_loop, feedback, debate…) | **BUANG** | H8 learning loop mengubah parameter risk saat live | Perubahan parameter hanya via rilis + validasi |
| `data/aggtrades.py` (214) | **PORT** logika | Downloader data.binance.vision aggTrades sudah benar arah | `crates/data/src/vision.rs` |
| `data/binance_data.py`, `data/ccxt_client.py`, `data/vike_client.py` | **BUANG** | Sinkron & blocking (H7); CCXT/Vike tidak dibutuhkan | `crates/exec/src/rest.rs` (reqwest) |
| `config/settings.py` | **BUANG** | L2 konfigurasi ganda, M1 BINANCE_TESTNET tak dibaca | `config/*.toml` + `.env` |
| `indicators/compute.py` (504) | **REFERENSI** | Oracle nilai indikator | `crates/features` |
| `dashboard.py`, `docker-compose.yml` | **BUANG** | M9 port 0.0.0.0, QuestDB tanpa auth | Dashboard Vite+React baca-saja, bind 127.0.0.1 |
| `run.py`, `setup.sh`, `init_pkgs.sh`, `requirements*.txt` | **BUANG** | — | `cargo` + `pnpm` |
| `.env.example` | **REFERENSI** | Nama variabel | `.env.example` baru (Design/README) |

## 4. scripts/ (65 file pencarian strategi) dan docs/

| Item | Keputusan | Catatan penting |
|---|---|---|
| `scripts/strategy_search_v3…v14`, `search_v9…v14`, tv* | **ARSIP** (tidak dipindah) | **Masuk ke hitungan trial.** ≈40 versi pencarian di data BTC/ETH 2025–2026 dicatat sebagai trial historis di `trials_ledger` (`source='apex_legacy'`, minimal N=40) agar Deflated Sharpe tidak optimistis |
| `docs/FINAL_*.json`, `search_*.json`, `COMBINED_60USD.json` | **ARSIP** | C8: tidak valid sebagai bukti (PF 0,39 setelah stress) |
| `docs/SPEC_STRATEGI_SMC_CTL.md`, `HYBRID_ARCHITECTURE.md`, `PROMPT_*` | **REFERENSI** | Ide saja |
| `tests/test_tv7_*.py` | **REFERENSI** | Kasus uji dipindah ke test Rust |

## 5. Ringkasan

- **Dipertahankan (PAKAI/PORT):** scalp_portfolio + test, validation + test, scalp_causality, indicators, pivots, smc_generators, fibonacci, data, protections (konsep), aggtrades (logika), RESEARCH_RULES.
- **Ditulis ulang total:** sizing (`risk.rs` melanggar GR-1/GR-3), eksekusi live, rekonsiliasi, ARM, risk guard.
- **Dibuang:** semua LLM/agen/analytics, backtest.rs (look-ahead), stack Python live, docker/QuestDB.
- **Data 2026 BTC/ETH sudah terkontaminasi** oleh pencarian lama. Holdout baru dijelaskan di Rules.md §4.2.
