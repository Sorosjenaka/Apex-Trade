# REPORT_CODEX

Branch: codex/strategies. Fetch publik selesai: 38 CSV; BTCUSDT dan ETHUSDT masing-masing 46.560 bar 15m dan 11.640 bar 1h.

## T-C2 — cancel_beyond
Implementasi pada bar tertutup, replay bar yang terlewat, rekonsiliasi status cancel serta pelestarian posisi partial fill. Tes `cargo test -p apex-oms`: 26 lulus (7 unit, 19 integrasi), 0 gagal. Tidak menjalankan test workspace sebelum commit ini sesuai instruksi terbaru user untuk tes per crate dan commit T-C2 sekarang.

## T-C1 — belum selesai
MR-09, OFCC fill check, magnet zone dan kNN belum di-port. Sumber MR-09: `C:\Users\FATHUR\Downloads\apex-trading-system\scripts\market_context_v3.py`.

Tidak ada order dikirim, holdout dijalankan, atau perubahan arm/mode. Tidak ada klaim profit.
