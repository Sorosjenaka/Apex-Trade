# TASKS.md - papan tugas Apex Scalper RS

| ID | Pemilik | Branch | Tugas | Status |
|---|---|---|---|---|
| T-C1 | Codex | codex/strategies | Port Pine lebih lengkap: S5 kNN grader, magnet zone filter, order-flow candle classifier (OFCC) fill check, MR-09 reversal context. Tambah sampel trade tanpa melonggarkan risk rule. | DONE (port selesai; sampel terukur 2026-11-05: S1 29 trade PF 1.78 E +0.402R, S5 0 — magnet+CHOP blok; penambahan sampel butuh knob T-H2 di hermes/validation + merge T-G1, lihat REPORT_CODEX §9) |
| T-C2 | Codex | codex/strategies | OMS: cancel_beyond (batalkan pending entry kalau harga lari), test di mock. | DONE (commit b1b8c6a, cargo test -p apex-oms 26 lulus) |
| T-C3 | Codex | codex/sample | Perluasan sampel tanpa melonggarkan risk: universe 40 pair dan entry 5m. | DONE tanpa adopsi (universe S1 VAL 35 E +0,073R; 5m S1 VAL 21 E −0,316R; cargo test --workspace 133 lulus, 0 gagal). Semua strategi NO-GO. |
| T-H1 | Hermes | hermes/validation | Walk-forward 6 fold, Monte Carlo 10k, stress biaya 2x, DSR dengan jumlah trial, uji plateau parameter, otomatiskan gate G-09 dan G-11. Data 2025-01..2026-04 saja. | DONE (runner bt.rs G-02..G-11 sesuai Rules.md; hasil S1/S5 NO-GO/INCONCLUSIVE — REPORT_HERMES) |
| T-H2 | Hermes | hermes/validation | Tuning anti-overfit: cari regime CHOP threshold yang tidak memblok 87% bar tanpa menurunkan PF di OOS. Laporkan plateau, bukan titik terbaik. | DONE (trend_share_min 15 / adx_max 15 / er_max 0.25 — blok FIT 78.5% <87%, PF OOS 1.38 vs 1.14 tidak turun; adopsi config = T-G1) |
| T-G1 | Grok Bot | master | Review, merge, jaga gate validasi. | IN_PROGRESS |
