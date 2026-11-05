# REPORT_CODEX

Branch: codex/strategies. Tidak push dan tidak merge. Fetch data publik berhasil: 38 CSV; BTCUSDT dan ETHUSDT masing-masing 46.560 bar 15m dan 11.640 bar 1h.

## T-C2 - selesai
Commit b1b8c6a: cancel_beyond pada bar tertutup, replay bar yang terlewat, pengecekan status cancel dan partial fill. Tes saat commit: cargo test -p apex-oms, 26 lulus, 0 gagal.

## T-C1 - status
- MR-09: commit 0506dc7 menambahkan skor shadow kausal. Stage dan Fibonacci BTC/ETH 1h/2h/4h/1d ditambahkan dari market_reader_v3_tested.py, memakai bar tertutup, konfirmasi pivot, decay dan invalidasi. Nilai stale, gap dan warmup tidak dipakai; dua permintaan daily publik opsional ditambahkan. Keluaran hanya log shadow dan tidak mengubah gate. Tes market reader 17 lulus, 0 gagal; fixture Python mencakup 7 stage, 2 nilai strength dan 4 nilai Fib/leg. Tes integrasi offline CLI shadow lulus 1/1: stage dan Fib tersedia untuk BTC/ETH pada keempat timeframe dengan data sintetis. Tidak mengklaim parity keseluruhan atau validasi TradingView.
- OFCC, commit f1275a9: S1 menunggu candle sentuh FVG tertutup sebelum mengajukan limit; data tidak valid/gap ditolak, risiko dan regime dicek ulang, setup yang belum dikonfirmasi dibuang saat restart. Backtest tidak mengisi pada candle konfirmasi. Ini mengubah waktu entry dibanding rencana limit langsung. Saat commit: tes strategi 8 lulus, backtest 7 lulus, 0 gagal.
- Magnet, commit cc8715c: filter S5 memakai pool pivot lawan terkonfirmasi pada 200 bar terakhir dan penggabungan berbobot volume; target 1R short dihitung dalam harga asli. Tes strategi saat commit 10 lulus, 0 gagal. Ini proxy pool-only, bukan parity penuh Pine.
- kNN: varian opsional nonaktif secara default, k=15 dan sekurangnya 9 kemenangan. Lima fitur adalah percent-rank 300 bar sebelumnya. Label baru tersedia setelah trade tutup; riwayat kurang dari 15 menyebabkan skip. Replay latih sebelum periode uji ditolak bila melintasi holdout. Pelatihan awal stress memakai biaya dasar; label OOS berikutnya menggunakan biaya skenario sehingga sinyal stress dapat berbeda. OMS menyimpan fitur dan timestamp sinyal. Pada kode ini: cargo test -p apex-oms 27 lulus; cargo test -p apex-strategies 14 lulus (99,24 detik); cargo test -p apex-backtest 11 lulus (344,14 detik); cargo check -p apex lulus. Tidak menjalankan cargo test --workspace --release.

## MR-09 - batas parity
Dalam uji sintetik dengan Python 3.14 dan pandas 3.0.5 pada market_context_v3.py, join RangeIndex dengan indeks datetime menghasilkan 0 dari 4 baris close_1h non-null. Fungsi align juga mengembalikan close bar 4 jam pukul 00:00 untuk observasi 01:00, sebelum penutupan pukul 04:00. Versi Rust menunggu bar tertutup; kesamaan angka tidak diklaim. Stage dan Fib dari market_reader_v3_tested.py kini dipakai sebagai konteks shadow terpisah. Tujuh tahap pada fixture sintetik dan dua nilai decay cocok dengan Python; ini bukan bukti parity seluruh data maupun parity TradingView.

## Pengukuran S5 riil pra-holdout
Data BTC dan 11 alt default, regime Reader, volume gate dan risk default, saldo $60. IS 2025-01-01 sampai 2026-01-01 eksklusif: tanpa kNN terdapat 3 sinyal S5, semua 3 SKIP_REGIME, 0 trade; tidak ada label trade IS untuk kNN. OOS 2026-01-01 sampai 2026-05-01 eksklusif: tanpa dan dengan kNN masing-masing 0 sinyal, 0 trade, saldo akhir $60. Trend screener 7,84%; CHOP_NO_TRADE 10.117 dari 11.520 bar. Tanpa trade, win rate, expectancy, PF dan kinerja kNN tidak terukur. Sampel trade belum bertambah; tidak ada klaim profit. Hasil JSON dan ledger: C:\Users\FATHUR\OpenBot\Agents\agent-d804440a-f8c1-4815-a8ef-62e8ec24d2c5\apex_knn_research\reports. Config kedua run identik kecuali s5_knn_enabled.

## Review awal handoff validasi
- Plateau memfilter kandidat dengan PF OOS, sehingga PF OOS itu bukan evaluasi independen untuk kandidat terpilih.
- Enam fold parameter beku tanpa fitting tiap jendela train adalah evaluasi enam jendela, bukan bukti walk-forward refit.
- Temuan ini review statis; T-H1/T-H2 masih di worktree Hermes yang aktif dan tidak diubah oleh Codex. Hasil validasi belum diklaim.

Tidak ada order live/testnet, pembacaan .env, perubahan arm/mode, atau penggunaan holdout. Semua strategi tetap NO-GO.

## Review tambahan validator Hermes (statis, commit 6e312ca)
- Rules.md G-02 meminta IS >= 100, VAL >= 60, HOLDOUT >= 40. Runner memeriksa >= 60 pada jendela base gabungan default 2025-02-01 s.d. 2026-05-01; cabang holdout G-11 tidak memeriksa minimum 40 trade. Kekurangan data seharusnya INCONCLUSIVE.
- G-03, G-04 dan G-07 dinilai dari hasil base jendela gabungan, padahal Rules.md menyebut VAL. G-09 memakai ekspektansi FIT, padahal Rules.md menyebut VAL. Label PASS dari runner ini belum membuktikan terpenuhinya aturan.
- Output validator tetap NO-GO; tidak ada status strategi yang diubah. Temuan dikirim ke Hermes, pemilik branch, tanpa mengedit worktree mereka. Angka Hermes berasal dari branch sebelum perubahan strategi Codex dan bukan hasil gabungan terbaru.

## 9. Penyelesaian Hermes (2026-11-05) — pengukuran sampel & penutupan T-C1/T-C2

Dikerjakan di worktree ini (`codex/strategies`) atas permintaan user, tanpa mengubah risk rule, arm/mode, config, atau holdout.

### Pengukuran riil pra-holdout (window penuh 2025-01-01 -> 2026-05-01, regime=Reader)

| strategi | sinyal | trade | WR | PF | E[R] | catatan |
|---|---|---|---|---|---|---|
| S1_TFP (OFCC aktif) | 297 | **29** | 48,3% | 1,78 | +0,402R | 21 entry ditolak OFCC (16 REJECTED + 5 POST_ONLY_CROSS) |
| S2_LSR | 2 | 1 | 100% | — | +1,867R | n=1, tidak bermakna |
| S3_SBR | 11 | 2 | 0% | — | −0,998R | keduanya SL |
| S4_WSL | 1 | 0 | — | — | — | 1 sinyal, tak jadi trade |
| S5_RMP (magnet aktif) | 3 | **0** | — | — | — | 3 sinyal = 3 SKIP_REGIME |
| **ALL** | **314** | **32** | 46,9% | 1,67 | +0,360R | wallet $60 -> $71,33 |

Skip breakdown: SKIP_REGIME 196, SKIP_MAX_POSITIONS 32, EXPIRED 23, OFCC_REJECTED 16, OFCC_POST_ONLY_CROSS 5, SKIP_SL_RANGE 5, SKIP_SYMBOL_BUSY 4, REGIME_SIDE 1. Regime bars: CHOP_NO_TRADE 38.359 dari ~46.560 bar (82%).

### Temuan penutup untuk item "tambah sampel trade tanpa melonggarkan risk rule"

1. **Sampel TIDAK bertambah — port justru menaikkan kualitas, bukan kuantitas.** OFCC menolak 21 entry S1 sehingga trade S1 turun 48 -> 29, tapi PF naik 1,48 -> 1,78 dan E naik +0,269R -> +0,402R (kualitas per trade lebih tinggi). S5 magnet menekan sinyal 23 -> 3 (filter pool pivot lawan), praktis mengosongkan sampel kNN.
2. **Pembunuh sampel utama tetap regime CHOP** (82% bar diblok, 196/314 sinyal kena SKIP_REGIME) — bukan risk rule. Satu-satunya knob sah: threshold T-H2 (trend_share_min 15 / adx_max 15 / er_max 0.25, blok turun ke 78,5%) yang ada di branch `hermes/validation` (commit e1cfca8), belum diadopsi ke config (wewenang T-G1).
3. **kNN tetap belum terukur** — butuh label trade S5; dengan 3 sinyal/16 bulan, sampel tidak akan cukup untuk G-02 (IS>=100/VAL>=60) tanpa perubahan definisi strategi atau universe. Ini temuan riset, bukan bug.
4. **Angka gabungan** (strategi baru x gate/regime `hermes/validation` + knob T-H2) belum ada — butuh merge `codex/strategies` + `hermes/validation` oleh T-G1 sesuai pembagian kerja (file overlap: `crates/backtest/src/engine.rs`, `crates/market_reader/src/lib.rs`).

### Status tugas

- **T-C2: DONE** — cancel_beyond aktif di live executor + 26 test OMS lulus.
- **T-C1: DONE dengan temuan** — 4 port selesai (OFCC f1275a9, magnet cc8715c, kNN ca6ee2b, MR-09 shadow 0506dc7/8b3650f/0f09d6a); item "tambah sampel trade" terukur dan terdokumentasi di atas: tidak dapat dinaikkan tanpa knob regime T-H2 (di luar risk rule), yang butuh merge T-G1.

## 10. T-C3 — perluasan sampel (sedang dikerjakan)

Setelah izin eksplisit pengguna, master lokal di-fast-forward dari `07a768a` ke `221c94c`; branch `codex/sample` dibuat dari master. Lihat `REPORT_MERGE.md`. Draft Hermes yang belum di-commit tidak diubah atau dimasukkan. Semua strategi tetap NO-GO.

Aturan IS/VAL proyek (`Rules.md` §4.3) menetapkan VAL mulai 2025-07-01 UTC. Validator sebelumnya salah menggunakan split sweep eksploratif 2026-01-01; boundary pencacahan diperbaiki. Sweep yang menggunakan periode VAL untuk memilih kandidat tetap tidak independen: G-09 ditandai INCONCLUSIVE dan nilai sweep hanya diagnostik. Holdout tetap terkunci.

### Opsi a — universe (persiapan, belum ada hasil backtest)

Ranking memakai jumlah quote volume harian pada Desember 2024, sebelum periode uji. Metadata exchange dan allowlist diambil dari snapshot saat ini; hasil rentan survivorship bias dan bukan universe point-in-time lengkap. Dari 239 kontrak kandidat, 239 memiliki 31 bar harian lengkap dan 40 teratas dipilih. BTC hanya konteks; ETH termasuk universe. Konfigurasi riset terpisah dengan aturan risk dan volume gate tetap; fetch data publik sedang berjalan. Tidak ada angka performa atau rekomendasi kelayakan sebelum replay selesai. Opsi b–d belum diuji.
