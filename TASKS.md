# TASKS.md - papan tugas Apex Scalper RS

| ID | Pemilik | Branch | Tugas | Status |
|---|---|---|---|---|
| T-C1 | Codex | codex/strategies | Port Pine lebih lengkap: S5 kNN grader, magnet zone filter, order-flow candle classifier (OFCC) fill check, MR-09 reversal context. Tambah sampel trade tanpa melonggarkan risk rule. | IN_PROGRESS |
| T-C2 | Codex | codex/strategies | OMS: cancel_beyond (batalkan pending entry kalau harga lari), test di mock. | IN_PROGRESS |
| T-H1 | Hermes | hermes/validation | Walk-forward 6 fold, Monte Carlo 10k, stress biaya 2x, DSR dengan jumlah trial, uji plateau parameter, otomatiskan gate G-09 dan G-11. Data 2025-01..2026-04 saja. | IN_PROGRESS |
| T-H2 | Hermes | hermes/validation | Tuning anti-overfit: cari regime CHOP threshold yang tidak memblok 87% bar tanpa menurunkan PF di OOS. Laporkan plateau, bukan titik terbaik. | IN_PROGRESS |
| T-G1 | Grok Bot | master | Review, merge, jaga gate validasi. | IN_PROGRESS |
