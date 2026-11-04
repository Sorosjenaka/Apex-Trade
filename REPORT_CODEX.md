# REPORT_CODEX

Branch: codex/strategies. Tidak push dan tidak merge. Fetch data publik berhasil: 38 CSV; BTCUSDT dan ETHUSDT masing-masing 46.560 bar 15m dan 11.640 bar 1h.

## T-C2 - selesai
Commit b1b8c6a: cancel_beyond pada bar tertutup, replay bar yang terlewat, pengecekan status cancel dan partial fill. Tes saat commit: cargo test -p apex-oms, 26 lulus, 0 gagal.

## T-C1 - status
- MR-09, commit 0506dc7: skor multi-timeframe shadow dan divergence terkonfirmasi. Tidak mengubah gate. Tes market reader 13 lulus, 0 gagal. Ini adaptasi kausal, bukan parity 1:1; stage dari skrip berbeda dan komponen Fib belum di-port. MR-09 masih parsial dan belum divalidasi.
- OFCC, commit f1275a9: S1 menunggu candle sentuh FVG tertutup sebelum mengajukan limit; data tidak valid/gap ditolak, risiko dan regime dicek ulang, setup yang belum dikonfirmasi dibuang saat restart. Backtest tidak mengisi pada candle konfirmasi. Ini mengubah waktu entry dibanding rencana limit langsung. Saat commit: tes strategi 8 lulus, backtest 7 lulus, 0 gagal.
- Magnet, commit cc8715c: filter S5 memakai pool pivot lawan terkonfirmasi pada 200 bar terakhir dan penggabungan berbobot volume; target 1R short dihitung dalam harga asli. Tes strategi saat commit 10 lulus, 0 gagal. Ini proxy pool-only, bukan parity penuh Pine.
- kNN: varian opsional nonaktif secara default, k=15 dan sekurangnya 9 kemenangan. Lima fitur adalah percent-rank 300 bar sebelumnya. Label baru tersedia setelah trade tutup; riwayat kurang dari 15 menyebabkan skip. Replay latih sebelum periode uji ditolak bila melintasi holdout. Pelatihan awal stress memakai biaya dasar; label OOS berikutnya menggunakan biaya skenario sehingga sinyal stress dapat berbeda. OMS menyimpan fitur dan timestamp sinyal. Pada kode ini: cargo test -p apex-oms 27 lulus; cargo test -p apex-strategies 14 lulus (99,24 detik); cargo test -p apex-backtest 11 lulus (344,14 detik); cargo check -p apex lulus. Tidak menjalankan cargo test --workspace --release.

## MR-09 - batas parity
Dalam uji sintetik dengan Python 3.14 dan pandas 3.0.5 pada market_context_v3.py, join RangeIndex dengan indeks datetime menghasilkan 0 dari 4 baris close_1h non-null. Fungsi align juga mengembalikan close bar 4 jam pukul 00:00 untuk observasi 01:00, sebelum penutupan pukul 04:00. Versi Rust menunggu bar tertutup; kesamaan angka tidak diklaim. Stage didefinisikan pada market_reader_v3_tested.py menggunakan regular/hidden divergence dan struktur berbeda dan belum di-port.

## Pengukuran S5 riil pra-holdout
Data BTC dan 11 alt default, regime Reader, volume gate dan risk default, saldo $60. IS 2025-01-01 sampai 2026-01-01 eksklusif: tanpa kNN terdapat 3 sinyal S5, semua 3 SKIP_REGIME, 0 trade; tidak ada label trade IS untuk kNN. OOS 2026-01-01 sampai 2026-05-01 eksklusif: tanpa dan dengan kNN masing-masing 0 sinyal, 0 trade, saldo akhir $60. Trend screener 7,84%; CHOP_NO_TRADE 10.117 dari 11.520 bar. Tanpa trade, win rate, expectancy, PF dan kinerja kNN tidak terukur. Sampel trade belum bertambah; tidak ada klaim profit. Hasil JSON dan ledger: C:\Users\FATHUR\OpenBot\Agents\agent-d804440a-f8c1-4815-a8ef-62e8ec24d2c5\apex_knn_research\reports. Config kedua run identik kecuali s5_knn_enabled.

## Review awal handoff validasi
- Plateau memfilter kandidat dengan PF OOS, sehingga PF OOS itu bukan evaluasi independen untuk kandidat terpilih.
- Enam fold parameter beku tanpa fitting tiap jendela train adalah evaluasi enam jendela, bukan bukti walk-forward refit.
- Temuan ini review statis; T-H1/T-H2 masih di worktree Hermes yang aktif dan tidak diubah oleh Codex. Hasil validasi belum diklaim.

Tidak ada order live/testnet, pembacaan .env, perubahan arm/mode, atau penggunaan holdout. Semua strategi tetap NO-GO.
