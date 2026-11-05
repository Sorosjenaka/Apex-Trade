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

## 10. T-C3 — perluasan sampel (ditutup tanpa adopsi)

Setelah izin eksplisit pengguna, master lokal di-fast-forward dari `07a768a` ke `221c94c`; branch `codex/sample` dibuat dari master. Lihat `REPORT_MERGE.md`. Draft Hermes yang belum di-commit tidak diubah atau dimasukkan. Semua strategi tetap NO-GO.

Aturan IS/VAL proyek (`Rules.md` §4.3) menetapkan VAL mulai 2025-07-01 UTC. Validator sebelumnya salah menggunakan split sweep eksploratif 2026-01-01; boundary pencacahan diperbaiki. Sweep yang menggunakan periode VAL untuk memilih kandidat tetap tidak independen: G-09 ditandai INCONCLUSIVE dan nilai sweep hanya diagnostik. Holdout tetap terkunci.

Audit fetch menemukan endpoint funding Binance sebelumnya menerima `endTime` inklusif tepat di awal holdout (2026-05-01 00:00 UTC). Fetch yang sedang berjalan dihentikan setelah beberapa simbol; permintaan funding tersebut mungkin sudah mengunduh satu titik di batas itu. Tidak ada backtest holdout dan simulasi pra-holdout menyaring funding sebelum waktu akhir. Kode fetch diperbaiki agar meminta `end - 1 ms` dan memfilter ulang hasil; `cargo test -p apex-data` lulus 6/6. Fetch diulang ke direktori baru.

### Opsi a — universe 40 pair (replay selesai; belum layak validasi penuh)

Ranking memakai jumlah quote volume harian pada Desember 2024, sebelum periode uji. Metadata exchange dan allowlist diambil dari snapshot saat ini; hasil rentan survivorship bias dan bukan universe point-in-time lengkap. Dari 239 kontrak kandidat, 239 memiliki 31 bar harian lengkap dan 40 teratas dipilih. BTC hanya konteks; ETH termasuk universe. Pair terakhir, COWUSDT, memiliki rata-rata volume Desember sekitar $280 juta/hari. Konfigurasi riset terpisah dapat dimatikan dengan kembali ke konfigurasi default, tanpa mengubah risk rule atau volume gate. Terdapat 42 seri termasuk BTC dan BTCDOM: pemeriksaan timestamp mendapati seluruh candle 15m/1h lengkap, funding terurut dalam rentang dan tanpa jeda lebih dari 8 jam + 1 menit; ini bukan verifikasi penuh checksum atau jadwal funding historis. Data dan pemilihan memiliki bias snapshot filter saat ini. Perluasan universe juga mengubah breadth yang dipakai Reader.

| Run terpisah | Sinyal | Trade | IS | VAL | WR | PF | E[R] | E VAL | Max DD |
|---|---:|---:|---:|---:|---:|---:|---:|---:|---:|
| S1 | 1.032 | 64 | 29 | 35 | 43,75% | 1,468 | +0,265 | +0,073 | $7,53 / 7,72R |
| S3 | 48 | 6 | 3 | 3 | 33,33% | 0,976 | −0,035 | −0,030 | $2,00 / 2,09R |
| Gabungan S1+S3 | 1.080 | 67 | 30 | 37 | 43,28% | 1,440 | +0,251 | +0,093 | $7,29 / 7,42R |

S1 menambah 18 trade VAL dari baseline (17 ke 35), tetapi masih kurang dari 60 dan E VAL turun dari +0,195 menjadi +0,073R, di bawah G-03 +0,10R. IS juga masih 29, di bawah 100. S3 E VAL negatif dengan hanya 3 trade; jangan pilih varian itu sebagai kandidat. S1: beberapa simbol masing-masing maksimum 4/64 trade (6,25%); April, Mei 2025 dan Januari 2026 masing-masing 8/64 (12,5%). DOGE menghasilkan $4,68 dari laba neto $16,73 (28,0%); Mei 2025 $9,32 dari $16,73 (55,7%), melampaui batas G-10 bulan 35% dengan definisi neto proyek. ETH hanya 1 trade, +$1,77; sampel tidak bermakna. Konsentrasi lengkap setiap simbol/bulan dan statistiknya di `research/results/universe.json`, baseline di `research/results/baseline.json`. Angka ini menggantikan hasil awal 65 trade karena replay ulang menerapkan pembatalan pending saat gate berubah dan batas usia listing, seperti pada varian 5m.

Rekomendasi sementara: **belum layak validasi penuh atau trading**. Hasil VAL pernah terpapar riset sebelumnya dan pilihan universe memakai snapshot kontrak kini; tidak independen. Seluruh gate tetap NO-GO. Hash backtest yang ada tidak mencakup universe dan data: baseline dan universe menghasilkan hash parameter sama, sehingga direktori dan manifest di atas harus dipakai untuk membedakannya. Ledger per direktori juga belum menjamin hitungan DSR global. Opsi b diukur di bawah. Opsi c dan d tidak pernah ditulis di catatan tugas, jadi tidak dibuat.

### Opsi b — entry 5m (replay selesai; tes workspace lulus)

Opsi fetch publik `--only-5m` ditambahkan untuk mengambil candle yang kurang tanpa menimpa data 15m/1h/funding; uji `cargo test -p apex` lulus 7/7. Opsi fetch ini sendiri tidak mengubah timeframe strategi.

Eksperimen `research_entry_5m = true` hanya berlaku untuk backtest S1/S3 dengan Reader; default tetap 15m. Entry, TTL dan konfirmasi OFCC memakai candle 5m yang sudah tutup; keputusan Reader hanya diperbarui setiap close 15m, sementara screener tetap 15m/1h. 24 jam volume memakai 288 candle 5m. Bar 5m yang hilang/tidak valid menyebabkan error, dan pasangan harus melewati 30 hari sejak listing; jika metadata tidak ada, bar pertama dipakai sebagai estimasi konservatif. Pending dibatalkan bila regime, sisi, universe, batas posisi, cooldown, stop harian, atau trend simbol pada waktu yang tersedia tidak lagi mengizinkan fill. Pengecekan yang sama juga diterapkan saat replay ulang 15m untuk perbandingan adil. Tes prefix/mutasi masa depan untuk S1/S3 dan tes 1h tertutup lulus; ini bukan bukti G-01 lengkap.

| Run terpisah | Sinyal | Trade | IS | VAL | WR | PF | E[R] | E VAL | Max DD |
|---|---:|---:|---:|---:|---:|---:|---:|---:|---:|
| S1 5m | 440 | 44 | 23 | 21 | 38,64% | 1,197 | +0,118 | −0,316 | $8,49 / 8,57R |
| S3 5m | 5 | 3 | 2 | 1 | 66,67% | 3,677 | +0,921 | +1,879 | $1,00 / 1,00R |
| Gabungan 5m | 445 | 46 | 24 | 22 | 41,30% | 1,335 | +0,195 | −0,216 | $6,66 / 6,69R |

S1: SUI 9/44 trade (20,45%); April dan Mei 2025 masing-masing 6/44 (13,64%). ENA menyumbang $3,75 dari neto $5,29; Februari 2025 menyumbang $6,59 (lebih dari 100% neto karena bulan lain rugi). ETH terpisah: 5 trade, neto −$2,02, E −0,415R. S3 hanya satu trade VAL: angka positif tidak bermakna. Distribusi lengkap ada di `research/results/five_minute.json`. **5m tidak direkomendasikan untuk validasi penuh:** E VAL S1 dan gabungan negatif, dan semua sampel jauh di bawah G-02.

### Baseline untuk perbandingan (replay Reader, universe default, 2025-01-01–2026-05-01 eksklusif)

| Run terpisah | Sinyal | Trade | IS | VAL | WR | PF | E[R] | E VAL | Max DD |
|---|---:|---:|---:|---:|---:|---:|---:|---:|---:|
| S1 | 297 | 29 | 12 | 17 | 48,28% | 1,776 | +0,402 | +0,195 | $2,99 / 3,01R |
| S3 | 11 | 3 | 1 | 2 | 0% | 0 | −0,999 | −0,998 | $2,99 / 3,00R |
| Gabungan S1+S3 | 308 | 31 | 12 | 19 | 45,16% | 1,566 | +0,311 | +0,069 | $4,99 / 5,01R |

Replay memakai biaya model dan saldo awal $60; IS berakhir 2025-07-01 dan VAL sesudahnya. Sinyal dan hasil per strategi dari run masing-masing, sedangkan gabungan berbagi slot dan saldo. S1: konsentrasi trade tertinggi DOGE 5/29 (17,2%), bulan November 2025 dan Januari 2026 masing-masing 4/29 (13,8%); laba neto DOGE $6,55 dari total $11,54, April 2025 $5,71 dari total $11,54. S3: masing-masing LINK, SUI, XRP 1/3; Februari 2026 2/3, total neto negatif. Distribusi lengkap simbol/bulan dapat dihitung ulang dengan `research/tc3_metrics.py` pada JSON replay. Sampel IS/VAL jauh di bawah G-02 dan VAL pernah dipakai riset sebelumnya; angka ini tidak membuktikan profit konsisten.

`cargo test --workspace --offline` pada `codex/sample` lulus 2026-11-05: apex 7, backtest 21 (306,93 d), core 23, data 6, indicators 6, market_reader 18, oms 7, state_machine 20, screener 8, strategies 17 (81,97 d). Jumlah 133 lulus, 0 gagal. Doc-test setiap crate 0. Tidak ada order, holdout, push, atau merge. `research_entry_5m` tetap `false`. Kedua opsi tetap NO-GO.
