# REPORT_CODEX

Branch: codex/strategies. Fetch publik selesai: 38 CSV; BTCUSDT dan ETHUSDT masing-masing 46.560 bar 15m dan 11.640 bar 1h.

## T-C2 — selesai
Commit `b1b8c6a`: cancel_beyond pada bar tertutup, replay bar terlewat, rekonsiliasi cancel dan penanganan partial fill. `cargo test -p apex-oms`: 26 lulus (7 unit, 19 integrasi), 0 gagal. Tidak menjalankan test workspace sebelum commit sesuai instruksi terbaru user.

## T-C1 — dalam pengerjaan
MR-09: skor shadow multi-timeframe dan deteksi divergence terkonfirmasi ditambahkan; log tidak mengubah gate. Tes `cargo test -p apex-market-reader`: 13 lulus, 0 gagal; `cargo check -p apex` lulus. Tes prefix, mutasi future, dan data tidak sinkron lulus. Ini adaptasi kausal dari sumber, bukan klaim parity: perbandingan numerik dengan Python belum dilakukan; `stage` dalam dokumen belum ada di sumber yang diperiksa dan belum diimplementasikan. Tidak layak disebut port lengkap.
Tes strategi awal 4 lulus. kNN belum di-port.

Review handoff Grok: T-H1/T-H2 belum selesai dan belum commit di worktree Hermes; tidak disentuh karena Hermes masih bekerja. Tidak ada order dikirim, holdout dijalankan, atau perubahan arm/mode. Tidak ada klaim profit.

## Review awal handoff validasi (belum final)
- Sweep `select_plateau` memakai PF OOS untuk menyaring kandidat grid. Karena OOS ikut memilih kandidat, PF OOS tersebut tidak boleh dilaporkan sebagai evaluasi independen atas kandidat terpilih; butuh pemisahan periode validasi lain atau label sebagai data seleksi.
- Enam fold menjalankan parameter beku tanpa fitting di masing-masing jendela train; ini evaluasi per enam jendela berurutan, bukan bukti walk-forward re-fit. Belum ada hasil run atau klaim gate lulus.
- Perubahan T-H1/T-H2 masih dalam worktree Hermes yang aktif; temuan ini berbasis review statis dan belum dicoba di sana.

## OFCC — sedang diuji
S1 ditunda sampai candle sentuh FVG tertutup; data flow tidak valid atau gap menyebabkan reject. Setelah konfirmasi, risk/regime diperiksa lagi sebelum pengajuan limit; watch yang belum dikonfirmasi hilang saat restart (fail closed). Backtest tidak mengisi pada candle konfirmasi. Empat tes terarah OFCC lulus. `cargo check -p apex` lulus; tes penuh per crate strategi dan backtest masih berjalan. Belum commit.

## Magnet ? adaptasi pool pivot
Filter S5 memakai pool pivot lawan terkonfirmasi, penggabungan berbobot volume dan masa aktif 200 bar; target 1R short dihitung dalam harga asli sebelum transformasi cermin. `cargo test -p apex-strategies`: 10 lulus, 0 gagal (80,73 detik), termasuk konfirmasi pivot, bobot volume, expiry, 1R short, prefix dan mutasi future. Ini proxy pool-only, bukan port penuh scoring/zone Pine dan belum diuji parity dengan TradingView.
