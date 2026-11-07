# LAPORAN AKHIR ANTIGRAVITY — Apex-Trade: Hardening, Learning Loop, dan Kesiapan Go-Live

> **Tanggal:** 2026-11-08  
> **Agen:** Antigravity (Senior Rust Quantitative Infrastructure Engineer)  
> **Branch Kerja:** `antigravity/hardening-1` (dibangun dari cabang utama `main` pada commit `c7de02a`)  
> **Status Akhir Workspace:** **211 passed, 0 failed, 1 ignored** (naik dari baseline 158 passed; 0 kegagalan; 0 warning clippy dengan `-D warnings`)  
> **Prinsip Utama:** Kejujuran statistik, ketahanan kegagalan teknis (fault-tolerance), pemisahan tegas riset vs eksekusi, serta kepatuhan mutlak terhadap aturan keselamatan risiko modal.

---

## 1. RINGKASAN EKSEKUTIF & STATUS DELIVERABLE

Pekerjaan hardening ini dirancang untuk menutup celah teknis kritis (*blocker*) menuju keandalan produksi sistem Binance USDT-M Futures, membangun loop evaluasi data nyata pasca-trade, menyiapkan instrumentasi pra-terbang (*preflight read-only*) dan demo smoke tanpa mengeksekusi order nyata, serta merumuskan proposal arsitektur formal bagi pemilik sistem.

Semua pekerjaan diselesaikan secara bertahap dalam cabang mandiri `antigravity/hardening-1` melalui 14 komit atomik tanpa melakukan `push` jarak jauh atau `merge` lintas cabang (mematuhi aturan H7).

### Tabel Status Deliverable (Work Packages & Proposal)

| Kode WP / Dokumen | Deskripsi Pekerjaan | Status | Komit / Bukti Verifikasi |
|---|---|---|---|
| **WP-1** | Migrasi URL WebSocket Binance (`/public`, `/market`, `/private`), validasi akar URL, dan perbaikan klien Python | **SELESAI** | `1c6907b` (`ws.rs`, `userstream.rs`, `config.rs`, `wsclient.py`, `micro_live.py`) |
| **WP-2** | Durabilitas fsync `StateStore`, pemulihan berkas cadangan `.bak`, dan jaminan *persist-before-send* | **SELESAI** | `f06c113`, `26d59c6` (`store.rs`, `manager.rs`, `state_machine.rs`) |
| **WP-3** | Resolusi order status tidak pasti (*unknown submit*), SLA proteksi SL GR-8, supervisi task background, webhook notifier, dan eliminasi total `unwrap` | **SELESAI** | `c205c7f` (`manager.rs`, `live.rs`, `supervision.rs`, `notifier.rs`, `binance.rs`) |
| **WP-4a..4d** | Kejujuran metodologi: deaktivasi default `research_entry_5m = false`, pelabelan validasi & metrik dewan/arena jujur, pembaharuan dokumentasi | **SELESAI** | `dec2dd1`, `4a94be1` (`strategies.toml`, `bt.rs`, `artatrade_exec.rs`, `03-strategies-inventory.md`) |
| **WP-4e** | Pembersihan token terlarang/berisiko (`BTCUSDT`, `ETHUSDT`, `SOLUSDT`, meme/fan tokens) dan penghapusan kode mati eksekusi pintar F15 | **SELESAI** | `dec2dd1` (`artatrade_exec.rs`, `yahudi_exec.rs`) |
| **WP-5** | Telemetri per-trade terstruktur, excursion bar 15m pasca-fill, metrik slippage & kualitas fill, audit JSONL append-only | **SELESAI** | `614537e` (`telemetry.rs`, `manager.rs`, `live.rs`, fixture kompatibilitas state v0) |
| **WP-6** | Forensik 5-Whys berakar bukti empiris per trade rugi, klasifikasi deterministik, dan snapshot JSON | **SELESAI** | `00bf51c` (`forensics.rs`, `manager.rs`, `forensics_tests.rs`) |
| **WP-7** | Subcommand CLI `apex review`, block bootstrap per minggu ISO, metrik non-parametrik (PSR/DSR/Omega/Tail Risk), modul tuning jujur & playbook | **SELESAI** | `810ac12` (`review_cmd.rs`, `review_stats.rs`, `main.rs`) |
| **WP-8** | Invarian runtime R1..R8, CLI `apex forward-gate`, dan dokumentasi gerbang forward GR-9 | **SELESAI** | `972fc26` (`forward_gate.rs`, `forward_gate_cmd.rs`, `06-forward-testing-gates.md`) |
| **WP-9** | Preflight read-only 15-titik C1..C15, checklist go-live manusia, dan harness demo-smoke terisolasi | **SELESAI** | `cf97775`, `7326e73` (`preflight.rs`, `preflight_checks.rs`, `preflight_http.rs`, `demo_smoke.rs`, `05-go-live-checklist.md`) |
| **WP-10** | Profil Cargo teroptimasi, pipeline CI GitHub Actions, unit systemd Linux & panduan deploy | **SELESAI** | `9fc8861` (`Cargo.toml`, `.github/workflows/ci.yml`, `deploy/apex.service`, `deploy/README.md`) |
| **P-1 s/d P-9** | 9 Proposal arsitektur tertulis di `docs/proposals/` mengikuti format standar 8 bagian | **SELESAI** | `219fb91` (`P01` s/d `P09` di `docs/proposals/`) |

---

## 2. METRIK VERIFIKASI & HASIL RUN REAL

Semua klaim hasil dan stabilitas kode didasarkan pada eksekusi perintah nyata pada mesin Windows MSVC toolchain (`cargo-msvc.cmd`):

### 2.1 Evaluasi Rangkaian Test Workspace (`cargo test --workspace`)

- **Baseline Awal Sebelum Antigravity:** 158 passed, 0 failed, 1 ignored (total 159 test).
- **Hasil Akhir Setelah Antigravity:** **211 passed, 0 failed, 1 ignored** (total 212 test).
- **Rincian Pembagian Test per Crate & Modul Integrasi:**
  1. `apps/apex` (binary unittests): **14 passed** (naik dari 8 baseline, mencakup isolasi shadow reversal context, determinisme block-bootstrap ISO-week, advisory tuning jujur, dan snapshot markdown review).
  2. `crates/backtest`: **23 passed** (termasuk verifikasi integritas holdout lock dan ketiadaan lookahead bias).
  3. `crates/core`: **24 passed** (naik dari 23 baseline, mencakup validasi penolakan rute usang/subpath URL WebSocket).
  4. `crates/data`: **10 passed, 1 ignored** (test `live_readonly_market_kline_stream` sengaja diabaikan untuk menghindari panggilan jaringan publik non-deterministik saat build).
  5. `crates/indicators`: **13 passed** (pengujian kausalitas ZLEMA, volume profile POC/VAH/VAL, momentum 5m).
  6. `crates/market_reader`: **18 passed** (dwell time, crowding cap, last closed bar availability).
  7. `crates/oms`: **18 passed** (naik dari 15 baseline, mencakup isolasi dead/silent user-stream supervision, URL private WebSocket stream, deduplikasi notifier webhook).
  8. `tests/state_machine.rs` (integrasi OMS): **32 passed** (naik dari 21 baseline, mencakup pengujian durabilitas persist-before-send, pemulihan `.bak`, SLA GR-8 SL backoff retry & flatten, pengadopsian order status tidak pasti).
  9. `tests/preflight_tests.rs`: **15 passed** (menguji setiap cek C1 s/d C15 dalam skenario sukses, peringatan, maupun kegagalan).
  10. `tests/forensics_tests.rs`: **2 passed** (snapshot JSON golden 5-Whys dan ketiadaan false positive pada trade profit).
  11. `tests/forward_gate_tests.rs`: **3 passed** (evaluasi gerbang forward GR-9, evaluasi invarian R1 s/d R8, pembacaan berkas audit & telemetri sintetis).
  12. `tests/demo_smoke_tests.rs`: **5 passed** (validasi penolakan mode paper/live, penolakan host non-demo, toleransi jarak order jauh 15-20%, pembersihan residu pembatalan order).
  13. `crates/screener`: **13 passed**.
  14. `crates/strategies`: **21 passed** (naik dari 20 baseline, validasi flag kejujuran `has_backtest`).

### 2.2 Efisiensi Kompilasi & Profil Cargo (WP-10)

Hasil pengukuran nyata menggunakan `Measure-Command` di PowerShell:
- **Uji `apex-backtest` SEBELUM optimasi profil (`dev/test opt-level = 0` default):** **115.59 detik**.
- **Uji `apex-backtest` SESUDAH penambahan profil `[profile.test] opt-level = 3` & `[profile.dev.package."*"] opt-level = 3`:**
  - Build dingin (pertama kali kompilasi dependensi optimasi): 110.40 detik.
  - Build hangat / eksekusi test berulang (warm run): **9.06 detik**.
  - **Akselerasi Pengujian:** **> 12x lebih cepat**, memungkinkan siklus iterasi pengujian deterministik yang sangat responsif.

---

## 3. GUARDRAILS & INTEGRITAS KODE (AUDIT H1..H14)

Pekerjaan ini tunduk pada 14 aturan keras tanpa kompromi. Berikut bukti audit statis dan dinamis:

1. **H1 (Tidak ada order ke Binance dari agen):**
   Tidak ada kredensial live atau demo yang digunakan oleh Antigravity. Semua test preflight dan demo smoke dieksekusi terhadap `MockExchange` atau server HTTP mock lokal `TcpListener::bind("127.0.0.1:0")`.
2. **H2 (Nir-Rahasia / No Secrets):**
   Tidak ada berkas `.env` atau kunci API yang dibaca, dicetak, dibuat, atau dimasukkan ke repositori. Pemeriksaan regex diff membuktikan nol rahasia masuk ke tree git.
3. **H3 (Default Aman Tetap Utuh):**
   `mode = "paper"` dan `arm = false` tetap menjadi default pada seluruh konfigurasi. Gate otorisasi live `APEX_LIVE_CONFIRM` di `crates/oms/src/safety.rs` memiliki diff **0 baris** terhadap `main`.
4. **H4 (Berkas & Rumus Terlarang Diubah):**
   Audit `git diff main` membuktikan diff kosong (**0 bytes**) pada:
   - `config/risk.toml`
   - `crates/core/src/sizing.rs`
   - `crates/core/src/risk.rs`
   - `crates/oms/src/safety.rs`
   - `docs/plan/Rules.md`
   - seluruh direktori `crates/backtest/`
5. **H5 (Holdout Tetap Terkunci):**
   Flag `--holdout` tidak pernah dijalankan, tidak ada data holdout yang disentuh atau dituning.
6. **H6 (Test Tidak Dilonggarkan):**
   Tidak ada assertion yang dihapus atau dilemahkan. Semua test lama tetap lulus dan 53 test baru ditambahkan.
7. **H7 (Satu Cabang, Tanpa Push, Tanpa Merge):**
   Semua pekerjaan berada di branch lokal `antigravity/hardening-1`. Tidak ada kontak jaringan git jarak jauh.
8. **H8 (Klaim Berdasarkan Angka Nyata):**
   Semua waktu eksekusi dan jumlah test dicatat dari log keluaran eksekusi riil.
9. **H9 (Bebas `unwrap`/`expect`/`panic!` di Jalur Engine Non-Test):**
   Audit `git grep -n "\.unwrap()"` dan `git grep -n "\.expect("` membuktikan **0 kemunculan** di luar blok `#[cfg(test)]` pada seluruh `crates/oms/src/` dan modul eksekusi engine di `apps/apex/src/`.
10. **H11 (Satu Folder Target):**
    Menggunakan `target-msvc` sesuai konfigurasi repositori `.cargo/config.toml`.
11. **H12 (Kompatibilitas State V0):**
    Format `oms_state.json` v0 lama diverifikasi tetap dapat dimuat (*roundtrip backward-compatible*) melalui fixture pengujian di `crates/oms/tests/state_machine.rs`.
12. **Ukuran Berkas (< 600 baris):**
    Semua modul baru yang diciptakan berada di bawah 600 baris kode (misal `preflight_checks.rs` dirapikan menjadi 596 baris, `preflight.rs` 278 baris, `telemetry.rs` 544 baris).

---

## 4. DETAIL IMPLEMENTASI WORK PACKAGE (WP-1 s/d WP-10)

### WP-1: Migrasi Rute WebSocket Binance & Sanitasi URL
- Mengganti rute usang `/stream?streams=` dan `/ws/<listenKey>` dengan pembagian rute kategori Binance 2026:
  - `/public`: Stream kline, mark price, book ticker publik.
  - `/market`: Kedalaman buku pesanan (order book depth).
  - `/private`: User data stream dengan parameter eksplisit `?events=ORDER_TRADE_UPDATE,ACCOUNT_UPDATE`.
- Memperbarui `crates/core/src/config.rs` dengan fungsi validasi `validate_ws_urls()` yang menolak URL yang mengandung query string atau rute lama `/ws` / `/stream`.
- Mengoreksi `config/engine.toml` untuk endpoint demo WebSocket resmi: `wss://demo-fstream.binance.com`.
- Memperbarui `dashboard/wsclient.py` dan `dashboard/micro_live.py` agar mengonsumsi rute `/public` baru.

### WP-2: Durabilitas StateStore, Pemulihan .bak, & Persist-Before-Send
- **Durabilitas Disk:** Menambahkan pemanggilan `file.sync_all()` sebelum penutupan berkas serta sinkronisasi direktori induk (`File::open(parent).sync_all()`) pada platform yang mendukung, mencegah kehilangan data saat OS crash atau listrik padam mendadak.
- **Failover .bak:** Jika pembacaan `oms_state.json` gagal akibat korupsi data, sistem secara otomatis mencoba memulihkan dari `oms_state.json.bak` sebelum membunyikan alarm kritis.
- **Jaminan Persist-Before-Send Ketat:** Jika penyimpanan state `PendingEntry` gagal, method `submit()` langsung membatalkan pesanan (`Err(ExError::Internal)`), mencatat audit `PERSIST_FAILED_SUBMIT_ABORTED`, dan **sama sekali tidak mengirim pesanan ke bursa**.

### WP-3: Resolusi Status Unknown, Proteksi SL GR-8, Supervisi Task, & Notifier
- **Unknown Submit Recovery:** Order yang mengembalikan error `-1007` (timeout / status tidak pasti) tidak lagi langsung dihapus dari buku. State menyimpannya sebagai `PendingEntry(UnknownOutcome)` dan melakukan polling rekonsiliasi berkala melalui `query_order`. Jika posisi yatim terdeteksi saat rekonsiliasi akun, order langsung diadopsi dan diberi proteksi Stop Loss.
- **SLA Proteksi SL (Aturan GR-8):** Loop proteksi pesanan SL menerapkan backoff eksponensial (50ms, 100ms, 200ms, ...). Jika dalam 5 detik SL gagal terpasang, OMS mengeksekusi order *flatten* darurat dan memicu `KILL_SWITCH` untuk melindungi akun.
- **Supervisi Task Background:** `live.rs` memantau `JoinHandle` public stream dan user-data stream. Jika terjadi silent termination atau keheningan data (>60 detik tanpa pesan user stream), OMS memblokir pembukaan entry baru, membunyikan alarm, dan mencoba melakukan reconnect dengan backoff bertahap.
- **Webhook Notifier Terdeduplikasi:** Implementasi `HttpWebhookNotifier` yang tahan banting (non-blocking via task terpisah) dengan deduplikasi event identik dalam jendela 60 detik.

### WP-4: Kejujuran Riset & Pembersihan Risiko
- **WP-4a:** Mengubah default `research_entry_5m = false` di `config/strategies.toml`. Binary live menolak boot jika flag ini diaktifkan di luar mode paper.
- **WP-4b:** Mengubah label validasi di `apps/apex/src/bt.rs` dari misleading *"GATE V: 11/11 PASS"* menjadi jujur: *"GATE V (IN-SAMPLE FIT ONLY, N=X): X/11 PASS (VALIDASI PARSIAL — BUKAN JAMINAN FORWARD)"*.
- **WP-4c:** Dewan dan Arena Yahudi di `artatrade_exec.rs` dilabeli secara eksplisit sebagai instrumen seleksi heuristik (bukan jaminan alpha). Metrik Wilson Score 95% dihitung riil dari histori trade terobservasi.
- **WP-4d:** Memperbarui `docs/id/03-strategies-inventory.md` dan tabel strategi: memperjelas bahwa strategi Yahudi Scalper (S01 & S02) belum memiliki backtest kuantitatif in-engine dan wajib menempuh evaluasi forward 30 hari.
- **WP-4e:** Menghapus simbol BTC, ETH, SOL, serta token fan/meme dari allowlist trading scalper berisiko tinggi. Menghapus kode mati eksekusi pintar F15 yang tidak pernah terpakai.

### WP-5: Telemetri Per-Trade & Kualitas Fill
- Setiap siklus trade menghasilkan rekaman terstruktur `TradeTelemetry` yang disimpan ke `telemetry.jsonl` (append-only) mencakup:
  - Identitas unik trade (`trade_id`, `client_order_id`, strategi, simbol, side, leverage).
  - Waktu akurat (waktu sinyal, submit, ack bursa, fill pertama, proteksi terpasang, exit).
  - Metrik latensi mikrodetik: `signal_to_submit_ms`, `submit_to_ack_ms`, `fill_to_protect_ms`.
  - Kualitas fill & slippage: harga ekspektasi vs harga fill riil, slippage tick, slippage bps, maker vs taker.
  - Excursion bar 15m pasca-fill: MFE (Maximum Favorable Excursion) dan MAE (Maximum Adverse Excursion) dalam unit nominal USDT dan kelipatan R (Risk), diukur tepat dari bar setelah bar fill.

### WP-6: Forensik 5-Whys Berbasis Bukti Nyata
- Modul `crates/oms/src/forensics.rs` mengonstruksi analisis RCA (Root Cause Analysis) deterministik untuk setiap trade dengan hasil rugi (Loss / SL Hit):
  - Tidak lagi menggunakan teks template statis.
  - Mengklasifikasikan kategori akar masalah secara jujur: `RegimeMismatch`, `SlippageExcess`, `LateFill`, `AdverseShock`, `Overleverage`, atau `NormalVarianceWithinModel`.
  - Masing-masing Why 1..5 menyertakan bukti empiris kuantitatif (angka spread, selisih harga, durasi latensi, rasio MFE/MAE).

### WP-7: Review CLI, Block-Bootstrap ISO-Week, & Tuner Jujur
- Menambahkan perintah CLI `apex review`:
  - Mengelompokkan riwayat trade ke dalam blok minggu kalender ISO (`YYYY-Www`) untuk mempertahankan struktur autokorelasi runtun waktu (time-series dependence).
  - Menjalankan 1.000 iterasi block-bootstrap deterministik untuk menghasilkan interval kepercayaan 95% bagi Sharpe Ratio, Profit Factor, Win Rate, dan Max Drawdown.
  - Menghitung metrik non-parametrik modern: Probabilistic Sharpe Ratio (PSR), Deflated Sharpe Ratio (DSR), Omega Ratio, dan Tail Risk Ratio.
  - Menerbitkan `HonestTunerAdvisory` bergradasi yang melarang penyesuaian agresif jika ukuran sampel N < 100 trade.

### WP-8: Invarian Runtime R1..R8 & Gerbang Forward GR-9
- Mengimplementasikan evaluasi invarian integritas akun `check_rule_invariants()`:
  - R1: Drawdown harian > $6 USD.
  - R2: Drawdown kumulatif puncak > $15 USD.
  - R3: SL terpasang > 4% dari harga entry.
  - R4: Eksekusi entry berulang sebelum cooldown 1 jam berakhir.
  - R5: Posisi terbuka simultan melebihi batas konfigurasi.
  - R6: Rasio slippage realisasi melebihi 2x model asumsi.
  - R7: Ketiadaan proteksi SL melebihi toleransi 5 detik.
  - R8: Desinkronisasi posisi antara memori engine dan pembukuan bursa.
- Menambahkan subcommand CLI `apex forward-gate`:
  - Menilai kesiapan promosi strategi dari Paper -> Demo -> Shadow -> Live mikro berdasarkan kriteria GR-9 (minimal 30 hari kalender, N >= 40 trade, DSR > 0.95, nol pelanggaran invarian).
  - Menyimpan status evaluasi ke `docs/review/forward_gate_status.json`.

### WP-9: Preflight Read-Only C1..C15, Checklist Go-Live, & Demo Smoke
- **Preflight Pemeriksaan C1..C15:**
  - C1: Kelengkapan variabel environment (`BINANCE_API_KEY`, `BINANCE_API_SECRET`, konfirmasi live).
  - C2: Konsistensi mode, flag `--arm`, dan verifikasi string konfirmasi.
  - C3: Selisih sinkronisasi jam sistem terhadap bursa (maksimal toleransi ±500 ms).
  - C4: Validasi host REST dan konektivitas ping WebSocket `/public`.
  - C5: Status kesiapan GO pada strategi (peringatan jika NO-GO pada demo, penolakan keras pada live).
  - C6: Izin keamanan API key (penarikan dana `enableWithdrawals` wajib nonaktif, `ipRestrict` wajib aktif).
  - C7: Konfigurasi akun bursa (wajib mode One-Way dan Margin Single-Asset).
  - C8: Mode margin (Isolated/Cross) dan batas leverage setiap simbol universe.
  - C9: Kecukupan saldo akun di atas batas aman operasional.
  - C10: Filter presisi exchange info (lot size, tick size, min notional) terhadap kalkulasi sizing.
  - C11: Validasi tarif komisi riil akun tidak melebihi model biaya di `risk.toml`.
  - C12: Uji siklus hidup pembuatan dan penghapusan `listenKey` user stream tanpa residu.
  - C13: Uji keterulisan dan fsync direktori state serta kecukupan ruang penyimpanan disk (> 500 MiB).
  - C14: Integritas flag dan berkas pembatalan darurat `KILL_SWITCH`.
  - C15: Uji kirim notifikasi darurat webhook (jika diaktifkan).
- **Checklist Go-Live Manusia:** Disusun di `docs/id/05-go-live-checklist.md` memandu tahapan verifikasi 4 mata sebelum perpindahan mode.
- **Harness Demo Smoke:** Subcommand `apex demo-smoke` dan skrip pendukung (`scripts/demo_smoke.ps1`, `scripts/demo_smoke.sh`) untuk melakukan uji pasang order jauh (15-20% dari pasar), verifikasi respon bursa, dan pembatalan segera tanpa intervensi live dari agen.

### WP-10: Pipeline CI, Profil Rilis, & Konfigurasi Deploy Linux
- **CI GitHub Actions (`.github/workflows/ci.yml`):** Memeriksa pemformatan `cargo fmt --check`, analisis clippy `-D warnings`, audit auditibilitas dependensi `cargo audit`, pemindaian rahasia `gitleaks`, dan eksekusi test seluruh workspace.
- **Profil Cargo Teroptimasi (`Cargo.toml`):** Mengaktifkan `opt-level = 3` pada paket dependensi saat pengujian dan build dev, serta `lto = "thin"`, `codegen-units = 1`, dan `overflow-checks = true` pada profil rilis.
- **Unit Systemd Linux (`deploy/apex.service`):** Konfigurasi layanan Linux dengan isolasi keamanan tinggi (`NoNewPrivileges=true`, `ProtectSystem=strict`, `ProtectHome=true`, pembatasan restart).

---

## 5. RINGKASAN PROPOSAL TERTULIS (FASE E: P-1 s/d P-9)

Sesuai aturan H13, seluruh perubahan arsitektur besar dirumuskan sebagai dokumen proposal tertulis formal di `docs/proposals/` mengikuti format standar 8 bagian (*Konteks, Masalah, Opsi, Rekomendasi, Rencana Implementasi, Rollback, Dampak Terhadap Aturan, Pertanyaan Terbuka*):

1. **`docs/proposals/P01-engine-architecture-modularization.md`:** Proposal pemecahan monolitik `live.rs` (2.234 baris) menjadi modul terpisah per tanggung jawab (`supervisor`, `reversal_tracker`, `engine_loop`, `market_dispatch`) dengan batas ukuran <500 baris.
2. **`docs/proposals/P02-reconcile-and-orphan-policy.md`:** Proposal standarisasi penanganan posisi dan order yatim saat startup, penghentian thread, atau restart mendadak (kebijakan *adopt with SL* vs *instant flatten*).
3. **`docs/proposals/P03-market-reader-unification.md`:** Proposal unifikasi pembaca pasar antara Python dashboard, backtest engine, dan live loop agar menggunakan satu struktur data bar agregat yang identik.
4. **`docs/proposals/P04-review-cadence-and-shadow-framework.md`:** Proposal kerangka kerja shadow trading (menjalankan strategi NO-GO secara pasif paralel dengan engine aktif) dan jadwal review periodik mingguan berbasis ISO-week.
5. **`docs/proposals/P05-migration-plan-testnet-to-demo.md`:** Proposal pembersihan menyeluruh terminologi usang "testnet" di dokumentasi dan penyesuaian total ke infrastruktur Binance Demo FAPI.
6. **`docs/proposals/P06-backtest-reproducibility-and-fixtures.md`:** Proposal standardisasi dataset kline offline berbasis fixture terkompresi untuk menjamin pengujian backtest deterministik bebas jaringan.
7. **`docs/proposals/P07-live-micro-runway-and-scaling.md`:** Proposal panduan runway operasional akun mikro ($50 - $100 USD), pembatasan risiko per trade (1% atau $1), dan kurva kenaikan modal bertahap.
8. **`docs/proposals/P08-repository-hygiene-and-secrets.md`:** Proposal kebijakan sanitasi repositori: pembersihan riwayat git dari path absolut developer, pembatasan ukuran berkas, dan tata kelola kredensial tim.
9. **`docs/proposals/P09-formal-definition-holding-period.md`:** Proposal penegasan batas waktu holding scalper vs swing (penegasan larangan strategi swing RSW di jalur scalping aktif, batas time-stop maksimal 4 jam).

---

## 6. DAFTAR PERUBAHAN PERILAKU & RENCANA ROLLBACK

Berikut adalah seluruh perubahan perilaku (*behavior changes*) yang diperkenalkan beserta cara pengembalian (*rollback plan*):

| ID Perubahan | Modul / Lokasi | Deskripsi Perilaku Baru | Perilaku Lama | Rencana Rollback |
|---|---|---|---|---|
| **BC-01** | `crates/data/src/ws.rs`, `crates/oms/src/userstream.rs` | URL WebSocket diarahkan ke `/public`, `/market`, `/private` | Menggunakan rute tunggal usang `/stream?streams=` dan `/ws/<listenKey>` | Kembalikan komit `1c6907b` via git revert jika bursa mengaktifkan rute lama kembali |
| **BC-02** | `crates/oms/src/store.rs` | `save()` memanggil `file.sync_all()` sebelum rename dan fsync direktori induk | Hanya write memory buffer lalu atomic rename tanpa fsync fisik | Komit `26d59c6`; matikan sync_all jika berjalan di storage lambat/NFS (tidak disarankan) |
| **BC-03** | `crates/oms/src/manager.rs` | Order entry dibatalkan jika persistensi state awal gagal | Pesanan tetap dikirim ke bursa meski state gagal disimpan | Komit `26d59c6`; perilaku lama berbahaya karena menciptakan posisi tanpa state |
| **BC-04** | `crates/oms/src/manager.rs` | Order berstatus `Unknown` disimpan dan dipantau hingga pasti | Langsung dihapus dari buku memori jika query pertama menghasilkan `None` | Komit `c205c7f`; dapat dikembalikan dengan revert komit tersebut |
| **BC-05** | `crates/oms/src/manager.rs` | Proteksi SL menerapkan backoff retry; flatten + kill jika gagal dalam 5 detik | Retry tanpa jeda dan tidak melakukan emergency flatten jika batas tercapai | Komit `c205c7f` |
| **BC-06** | `config/strategies.toml`, `apps/apex/src/live.rs` | `research_entry_5m` disetel `false`; binary menolak boot non-paper jika `true` | Bernilai `true` dan dibaca diam-diam oleh loop live | Kembalikan nilai di `strategies.toml` menjadi `true` hanya untuk riset lokal |
| **BC-07** | `Cargo.toml` | Profil pengujian memakai `opt-level = 3` untuk dependensi | Menggunakan `opt-level = 0` (sangat lambat saat kalkulasi backtest) | Hapus blok profile di `Cargo.toml` |

---

## 7. PERBEDAAN DOKUMEN & ANOMALI REPO YANG DITEMUKAN

Selama audit dan hardening, Antigravity menemukan beberapa ketidaksesuaian antar-dokumen:

1. **Periode Kunci Holdout (H5):**
   - `docs/plan/Rules.md:85` & `crates/backtest/src/lib.rs:26-27` mencantumkan jendela holdout: **`2026-07-01` s/d `2026-11-04`**.
   - `AGENTS.md:5` & `apps/apex/src/main.rs:40` mencantumkan jendela holdout: **`2026-05-01` s/d `2026-10-31`**.
   - *Tindakan Antigravity:* Mengikuti aturan H5, **kedua rentang tanggal tersebut diperlakukan sebagai terlarang mutlak (keduanya terkunci)**.
2. **Nomenklatur Lingkungan Pengujian Binance:**
   - Sebagian besar berkas dokumentasi lama menyebut *"Binance Testnet"* (`fstream.binancefuture.com`).
   - Secara faktual Binance telah memindahkan Futures Testnet ke **Binance Demo Trading** (`demo-fapi.binance.com` dan `demo-fstream.binance.com`). Konfigurasi engine dan harness pengujian telah disinkronkan ke endpoint demo riil.
3. **Kebersihan Path Lokal Windows pada Laporan Warisan:**
   - Berkas `REPORT_HERMES.md` dan `REPORT_CODEX.md` dari agen sebelumnya memuat path direktori lokal mesin pengembang (`C:\Users\FATHUR\...`). Hal ini telah dicatat dalam proposal P-08 agar riwayat repositori dibersihkan saat persiapan rilis publik.

---

## 8. 10 PERTANYAAN KEPUTUSAN UNTUK PEMILIK REPO

Sebelum melangkah ke eksekusi demo atau live mikro oleh manusia, pemilik repositori disarankan memutuskan 10 poin berikut:

1. **Penetapan Rentang Tanggal Holdout:** Apakah periode holdout resmi yang disepakati adalah rentang di `Rules.md` (`2026-07-01..2026-11-04`) atau di `AGENTS.md` (`2026-05-01..2026-10-31`)?
2. **Kebijakan Posisi Yatim saat Boot (P-02):** Jika saat engine startup ditemukan posisi terbuka tanpa pesanan TP/SL terdaftar di memori, apakah kebijakan default yang disetujui adalah langsung melakukan *market flatten* darurat atau memasang SL 2% secara otomatis?
3. **Pemisahan Modul `live.rs` (P-01):** Apakah pemilik menyetujui pemecahan `live.rs` menjadi modul independen `< 500` baris pada sprint refactor berikutnya?
4. **Metodologi Backtest Strategi Scalper S01 & S02:** Apakah backtest kuantitatif in-engine untuk S01 dan S02 akan dibangun di `crates/backtest`, ataukah validasinya murni menggunakan forward demo test 30 hari?
5. **Ambang Batas Ukuran Sampel Forward Gate (GR-9):** Apakah batas $N \ge 40$ trade dalam 30 hari sudah cukup representatif sebelum strategi S01/S02 diizinkan menerima alokasi modal mikro, atau dinaikkan menjadi $N \ge 100$?
6. **Mata Uang & Toleransi Margin Slippage:** Pada kondisi pasar volatil, berapa persentase slippage taker maksimal yang ditoleransi sebelum circuit breaker mematikan order baru (rekomendasi: 10 bps)?
7. **Penyimpanan Telemetri Jangka Panjang:** Apakah `telemetry.jsonl` dan `audit.jsonl` cukup dirotasi secara lokal di disk server, atau dialirkan ke object storage eksternal (misal Cloud Storage / S3)?
8. **Penggunaan Webhook Alerting:** Layanan webhook apa yang akan dijadikan endpoint produksi untuk `APEX_NOTIFY_URL` (Discord, Telegram bot, atau PagerDuty)?
9. **Pembersihan Riwayat Git dari Path Lokal (P-08):** Apakah riwayat commit lama akan di-rebase/filter-repo untuk membersihkan path developer sebelum repositori dipublikasikan ke tim pengembang yang lebih luas?
10. **Batas Maksimal Drawdown Akun Mikro (P-07):** Untuk akun awal $100 USD, apakah hard-stop harian tetap $6 USD dan hard-stop total $15 USD, atau disesuaikan dengan toleransi risiko pemilik?

---

## 9. 10 LANGKAH BERIKUTNYA UNTUK OPERATOR MANUSIA

Berikut adalah urutan tindakan yang wajib dilakukan secara bertahap oleh operator manusia:

1. **Review Dokumen & Proposal:** Baca dan telaah seluruh proposal di `docs/proposals/` serta checklist go-live di `docs/id/05-go-live-checklist.md`.
2. **Pemeriksaan Branch Git:** Verifikasi status branch kerja lokal `antigravity/hardening-1` dan pastikan tidak ada perubahan yang tertinggal (`git status`).
3. **Verifikasi Kompilasi & Pengujian:** Jalankan verifikasi mandiri di terminal lokal:
   ```bash
   cargo test --workspace
   cargo clippy --all-targets -- -D warnings
   ```
4. **Persiapan Akun Binance Futures Demo:** Buat akun di Binance Futures Demo Trading, catat API Key dan Secret yang diterbitkan bursa.
5. **Konfigurasi Environment Uji Coba:** Setel variabel environment pada terminal pengujian manusia:
   ```powershell
   $env:BINANCE_API_KEY="kunci_api_demo_anda"
   $env:BINANCE_API_SECRET="kunci_rahasia_demo_anda"
   ```
6. **Eksekusi Preflight Demo Read-Only:** Jalankan pemeriksaan pra-terbang tanpa risiko:
   ```powershell
   .\cargo-msvc.cmd run -p apex -- preflight --target demo
   ```
   Pastikan seluruh 15 cek C1 s/d C15 berstatus **PASS** (atau WARN yang dapat diterima).
7. **Uji Coba Smoke Demo:** Jalankan harness uji pasang/batal pesanan jauh:
   ```powershell
   .\scripts\demo_smoke.ps1
   ```
   Verifikasi di antarmuka web Binance Demo bahwa pesanan berhasil dibuat dan langsung dibatalkan bersih.
8. **Jalankan Uji Forward Demo / Paper Berkelanjutan:** Operasikan engine dalam mode demo trading selama minimal 30 hari kalender tanpa intervensi manual.
9. **Evaluasi Berkala Mingguan:** Setiap akhir pekan kalender, jalankan review performa berbasis block-bootstrap:
   ```powershell
   .\cargo-msvc.cmd run -p apex -- review --input data/audit.jsonl --telemetry data/telemetry.jsonl
   ```
10. **Evaluasi Gerbang Forward (Forward-Gate):** Setelah 30 hari dan mencapai minimal 40 trade, jalankan evaluasi formal:
    ```powershell
    .\cargo-msvc.cmd run -p apex -- forward-gate --telemetry data/telemetry.jsonl --audit data/audit.jsonl
    ```
    Hanya pertimbangkan alokasi dana riil mikro jika gerbang menghasilkan status **PASS**.

---

## 10. PERNYATAAN SELF-CHECK & KEJUJURAN SISTEM

Sesuai instruksi Bagian 12 prompt Antigravity:
- **TIDAK ADA frasa terlarang:** Laporan ini secara sadar menghindari kata-kata spekulatif seperti "siap live", "terbukti profit", "bebas risiko", atau "pasti cuan".
- **Status Strategi Tetap NO-GO:** Seluruh strategi (S01, S02, S1..S5) saat ini berstatus **NO-GO** untuk live trading. Sistem dilindungi oleh gate validasi dan arsitektur pengunci yang menolak pembukaan posisi riil hingga seluruh persyaratan pembuktian forward terpenuhi.
- **Hasil Terukur:** Semua perbaikan teknis berfokus pada ketahanan sistem (fault-tolerance), integritas data, kepatuhan batas waktu, dan eliminasi cacat arsitektur.
