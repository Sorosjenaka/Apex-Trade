# Roadmap: Apex Scalper RS

> v1.0 · 5 Nov 2026 (WIB). Estimasi: S < 4 jam, M 4–16 jam, L > 16 jam. Granularitas BUILD: 1 respons = 1 tugas (≤ 3 file); tugas besar dipecah saat BUILD.
> Urutan fase mengikuti GR-9: riset & validasi → testnet → shadow mainnet → ARM mainnet oleh user.

## Fase 0 — Fondasi (target 2 hari)
- [ ] T-0.1 Workspace Cargo + struktur folder [→ architecture §3], `rust-toolchain.toml` 1.87 (NFR-15) [S]
- [ ] T-0.2 `Cargo.toml` workspace + dependency architecture §1 (A-01); `web/package.json` Vite+React+TS (FR-033) [S]
- [ ] T-0.3 `.env.example` (placeholder saja) + `.gitignore` (`.env`, `data/`, `reports/*`, `target/`, `node_modules/`) (NFR-04) [S]
- [ ] T-0.4 README.md [→ README] (NFR-15; Rules §5) [S]
- [ ] T-0.5 `apps/engine`: axum bind 127.0.0.1:8787, `GET /api/health` → `{"status":"ok"}` + test integrasi (FR-001, NFR-03, NFR-05) [S]
- [ ] T-0.6 `config/risk.toml` + loader yang menolak start bila angka ≠ Rules §1 (FR-002; GR-1..GR-7) [S]
- [ ] T-0.7 CI: fmt, clippy -D warnings, test, gitleaks (NFR-04) [S]

**Selesai bila:** `curl -s 127.0.0.1:8787/api/health` → `{"status":"ok"}` (output ditempel di CHANGELOG), `cargo test` hijau, FR-001 & NFR-05 lulus.

## Fase 1 — Data, Fitur, Screener (target 1–1,5 minggu)
- [ ] T-1.1 `data::vision` downloader klines 5m/15m/1h + aggTrades + fundingRate, verifikasi CHECKSUM (FR-029, US-12) [M]
- [ ] T-1.2 Unduh universe A-14 + BTC, 2023-01-01 → 2026-10-31; laporan celah (FR-029, US-12) [M]
- [ ] T-1.3 Port `indicators.rs`, `pivots.rs`, `fibonacci.rs` + test lama lulus (FR-008, FR-010) [M]
- [ ] T-1.4 Fitur regime: ER, CHOP, SRV, Bilateral Filter, Decycler, MTF closed-bar, ATR ratio (FR-008, US-09) [L]
- [ ] T-1.5 Fitur delta (takerBuy), percent-rank, volume profile range-tertutup (FR-010) [M]
- [ ] T-1.6 Crate `screener` sesuai SCREENER.md §2–§4 + histeresis (FR-008, FR-009, US-09) [M]
- [ ] T-1.7 Prefix test kausalitas untuk semua fitur + screener (NFR-18) [M]
- [ ] T-1.8 `apex-research screen --date` → laporan % jam TREND per simbol (FR-008; SCREENER §7) [S]

**Selesai bila:** FR-008, FR-029 lulus; NFR-12 (unduh+fitur), NFR-18 100%.

## Fase 2 — Strategi, Replay, Validasi (target 2–3 minggu) → keputusan GO/NO-GO (BG-1)
- [ ] T-2.1 Kunci HOLDOUT: hash file 2026-05-01..2026-10-31 dicatat, akses diblok flag `--unlock-holdout` (Rules §4.3, US-14) [S]
- [ ] T-2.2 Port `scalp_portfolio.rs` → `crates/backtest` (universe dinamis, balance 60, TF 15m/1h) + test determinisme (FR-030, NFR-17, US-13) [L]
- [ ] T-2.3 `risk::sizing` + kasus U-01..U-08 (FR-011, FR-012, FR-013, US-04, US-05) [S]
- [ ] T-2.4 Gate cooldown global 60 menit setelah SL (FR-015, US-08) [S]
- [ ] T-2.5 Gate stop harian 6 SL / −$6 UTC + max 2 posisi (FR-014, FR-016, US-06, US-07) [S]
- [ ] T-2.6 Model biaya: fee maker/taker, slip, funding histori, fill "tembus 1 tick" (FR-030) [M]
- [ ] T-2.7 Prefix test untuk S1–S5 (NFR-18) [M]
- [ ] T-2.8 S3 (SBR): implementasi + paritas vs oracle Python apex Strategi 1 (FR-010, US-13) [L]
- [ ] T-2.9 S1, S2, S4, S5: implementasi; tes khusus label kNN S5 hanya dari trade tertutup (FR-010, NFR-18) [L]
- [ ] T-2.10 `trials_ledger` + impor 40 trial `apex_legacy` (FR-032, US-15) [S]
- [ ] T-2.11 `crates/validate`: walk-forward, Monte Carlo, DSR, plateau, konsentrasi, stress (FR-031, US-14) [L]
- [ ] T-2.12 Grid IS per strategi (≤ 60 trial) → pilih 1 parameter → VAL → laporan G-01..G-10 (FR-031) [L]
- [ ] T-2.13 HOLDOUT sekali untuk strategi yang lolos G-01..G-10 → G-11 → verdict GO/NO-GO ke `config/strategies.toml` (FR-031, BG-1) [M]

**Selesai bila:** M-1 = 5 verdict tertulis. Bila 0 strategi GO → proyek berhenti di sini; laporan NO-GO adalah hasil sah, bukan kegagalan teknis.

## Fase 3 — Eksekusi Testnet (target 3 minggu termasuk 14 hari jalan)
- [ ] T-3.1 Klien REST signed + sinkron waktu + mode testnet/mainnet; verifikasi base URL `demo-fapi.binance.com` & WS `fstream.binancefuture.com` (FR-005, FR-034, A-20) [M]
- [ ] T-3.2 Muat filter + bracket berkala (FR-003, NFR-19) [S]
- [ ] T-3.3 Cek/set akun: one-way, CROSSED, 25x (FR-004, US-02) [M]
- [ ] T-3.4 WS kline + bookTicker + reconnect/backfill (FR-006, FR-007, NFR-07) [M]
- [ ] T-3.5 Order state machine + entry LIMIT GTX/MARKET + TTL (FR-017, US-03) [M]
- [ ] T-3.6 Algo SL/TP `closePosition` MARK_PRICE + konfirmasi ≤ 2 dtk (FR-018, NFR-02, US-03) [M]
- [ ] T-3.7 Gagal SL → MARKET reduceOnly + DISARM; test wiremock (FR-019, US-03) [M]
- [ ] T-3.8 User data stream ORDER/ALGO/ACCOUNT + keepalive (FR-020, US-10) [M]
- [ ] T-3.9 Reconciler start + 60 dtk; test kill -9 (FR-021, NFR-09, US-10) [L]
- [ ] T-3.10 ARM/DISARM/KILL + equity floor + audit (FR-022, FR-023, FR-024, FR-025, US-02, US-11) [M]
- [ ] T-3.11 API status/trades/screener/signals (FR-026, FR-027, FR-028, US-01) [S]
- [ ] T-3.12 Jalan testnet ≥ 14 hari / ≥ 30 trade; laporan slippage (PG-4, M-8, NFR-01, NFR-02) [L]
- [ ] T-3.13 Test kontrak harian endpoint algoOrder di testnet (FR-018, NFR-08; architecture §7) [S]
- [ ] T-3.14 Audit secret: grep log/DB/respons + gitleaks (NFR-04) [S]
- [ ] T-3.15 Soak 72 jam: uptime, RSS (NFR-08, NFR-10) [M]
- [ ] T-3.16 Dashboard Vite+React (FR-033, NFR-13, NFR-14, NFR-16) [M]

**Selesai bila:** semua FR P0 lulus test; PG-1..PG-4 terpenuhi di testnet; 0 `SL_MISSING`.

## Fase 4 — Mainnet bertahap
- [ ] T-4.1 Shadow mainnet ≥ 7 hari (`arm=false`, sinyal & sizing dicatat) — bandingkan dengan replay hari yang sama (NFR-18, PG-3) [M]
- [ ] T-4.2 User meng-ARM mainnet secara manual dengan wallet $60 (GR-9; US-02) [S]
- [ ] T-4.3 Review mingguan: M-2..M-7; review 100 trade untuk BG-2 (M-3) [S]
- [ ] T-4.4 FR-035 Notifikasi Telegram (US-01) [S]
