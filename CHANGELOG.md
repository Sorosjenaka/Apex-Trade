# Changelog

## 0.1.0 — 2026-11-05
Rilis awal (ARM=false, mode=paper, semua strategi NO-GO).

### Ditambahkan
- Workspace Rust 8 crate + CLI `apex` (fetch, reader, screen, backtest, validate, record, run, kill, status).
- Sizing berbasis risk $1 dengan pembulatan stepSize/tickSize, gate risk 0,80–1,00, SL 2–4 %, 25× cross, maks 2 posisi, stop harian 6 SL/−$6 (reset 00:00 UTC), cooldown 1 jam.
- Market Reader (regime global, histeresis), screener per simbol, allowlist crypto default-deny, volume gate V-01..V-10.
- Strategi S1 TFP, S2 LSR, S3 SBR, S4 WSL, S5 RMP (closed-bar).
- Backtest event-driven baru (bukan backtest.rs lama yang look-ahead): fill limit butuh tembus ≥1 tick, SL dicek dulu, gap → fill di open, slip adverse, funding, likuidasi cross, tes no-look-ahead.
- Statistik: PF, expectancy R, max DD, Monte Carlo, PSR/DSR, walk-forward 6 fold, stress, konsentrasi; ledger trial.
- OMS Binance: HMAC, recvWindow, time sync, algo SL/TP closePosition MARK_PRICE, user stream, rekonsiliasi, kill switch, backoff, state persisten, audit log, MockExchange.

### Disederhanakan / belum
- Tanpa SQLite & axum (JSON/JSONL).
- S5 tanpa kNN; tanpa filter "magnet"; tanpa cek fill OFCC; MR-09 belum di-port.
- Regime backtest = proxy BTCDOMUSDT; filter exchange = snapshot 2026-11-04; volume gate backtest parsial (V-01/V-02).
- Live: pending entry tidak dibatalkan saat harga lari (cancel_beyond) — hanya TTL.
- Gate G-09 (plateau) dan G-11 (holdout) tidak otomatis. (dipasang di 0.1.1)

## 0.1.1 — 2026-11-05
### Diubah
- `apex validate`: walk-forward anchored 6 fold dijalankan ulang per segmen (bukan potongan satu equity curve), Monte Carlo 10.000 path ukuran 1 tahun, stress G-05 (fee ×2, slip +0,05% termasuk fill limit, funding ×2), DSR dengan N = trial ledger strategi itu + trial sesi + 40 `apex_legacy`.
- G-09 otomatis pada grid CHOP (regime `trend_share_min` dan screener `chop_range`). Plateau = run yang blok FIT-nya di bawah 87% dan PF OOS-nya tidak turun; ambang yang dilaporkan adalah median run, bukan titik terbaik.
- Sweep CHOP diperluas: knob regime `adx_max` dan `er_max` ikut di-grid, plus satu run verifikasi kombinasi ketiga knob memakai median plateau masing-masing.
- G-11 otomatis hanya bila `--unlock-holdout` dan jendela menyentuh 2026-05-01..2026-10-31. Tanpa itu statusnya LOCKED, bukan PASS. Holdout tidak dibuka oleh sweep.
