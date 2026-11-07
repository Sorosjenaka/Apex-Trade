# P-9: Rencana Riset Event Study, Perluasan Data Historis, dan Protokol Pra-Registrasi

## 1. Konteks
Tantangan saintifik terbesar pada strategi scalping frekuensi rendah (±4 trade per bulan per instrumen) adalah **keterbatasan ukuran sampel**:
- Dataset saat ini hanya mencakup periode 2025-01-01 s/d 2026-05-01 (In-Sample) dan 2026-05-01 s/d 2026-10-31 (Holdout, terkunci).
- Pada periode holdout 6 bulan, total trade yang dihasilkan S1 diperkirakan hanya sekitar $\approx 24$ transaksi. Angka ini secara struktural **gagal memenuhi syarat minimum gerbang G-02/G-11** yang mensyaratkan minimal $N \ge 40$ trade untuk signifikansi statistik.
- Tanpa data historis yang lebih panjang (misalnya 2019/2020 s/d sekarang), mustahil membuktikan apakah sinyal S1 memiliki *true edge* atau sekadar anomali rezim pasar bull 2025.
- Melonggarkan ambang $N \ge 40$ pada gerbang validasi dilarang keras oleh aturan Golden Rules (H4).

## 2. Opsi
1. **Opsi A (Status Quo):** Tetap pada dataset pendek dan membiarkan strategi gagal pada gerbang G-02/G-11 karena sampel kurang.
   - *Kelebihan:* Tidak membutuhkan pengunduhan arsip data besar (puluhan GB).
   - *Kekurangan:* Riset jalan di tempat; tidak ada keputusan konklusif yang dapat ditarik secara matematis.
2. **Opsi B (Rekomendasi — Rencana Riset Pra-Registrasi & Perluasan Arsip Historis 2020–2026):**
   - **Template Protokol Pra-Registrasi:** Peneliti wajib mendokumentasikan hipotesis tunggal, membekukan parameter TOML, dan melakukan commit git *sebelum* menjalankan simulasi pada data baru.
   - **Studi Peristiwa (Event Study):** Evaluasi daya prediksi mentah (*raw predictive power*) dari sinyal entry S1 secara terisolasi tanpa pengaruh trailing-stop, sizing, atau dewan:
     - Ukur return rata-rata $k$-bar ke depan ($k \in [1, 3, 5, 15]$ bar 15m), neto fee taker/maker, dengan selang kepercayaan 95% block-bootstrap per minggu ISO.
   - **Ablasi Per Gate:** Uji kontribusi masing-masing filter (Chop, Volume, CVD, Regime) untuk melihat apakah filter tersebut benar-benar memangkas false-positive atau hanya membuang sinyal yang menguntungkan.
   - **Perluasan Data Historis Publik:** Unduh data arsip resmi publik Binance Data Portal (`data.binance.vision`) untuk pasangan BTCUSDT, ETHUSDT, dan perp mayor lainnya sejak awal listing (2019/2020 s/d 2024).
   - **Tata Kelola Holdout:** Holdout 2026-05-01 s/d 2026-10-31 **tetap terkunci keras**. Data 2020–2024 digunakan untuk memperbesar In-Sample (IS) dan Validation (VAL), sehingga syarat $N \ge 40$ pada holdout dapat terpenuhi secara alami melalui evaluasi multi-simbol pada universe likuid yang terstandarisasi.
3. **Opsi C (Melonggarkan Syarat $N$):** Menurunkan ambang $N \ge 40$ menjadi $N \ge 20$. Ditolak mentah-mentah karena melanggar Golden Rules dan meningkatkan risiko curve-fitting secara drastis.

## 3. Rekomendasi
Pilih **Opsi B**. Pendekatan ini menyelesaikan masalah kekurangan sampel melalui kerja keras pengumpulan data historis objektif, bukan dengan melemahkan standar pembuktian saintifik.

## 4. Risiko
- Perubahan mikrostruktur pasar kripto (regulasi, dominasi institusi) antara tahun 2020 vs 2026 yang dapat memengaruhi relevansi data lama (*non-stationarity*).
- Bias kesintasan (*survivorship bias*) pada koin altcoin yang baru meluncur setelah 2021 (dapat dimitigasi dengan membatasi histori panjang hanya pada BTC dan ETH).

## 5. Rencana Test
- Verifikasi checksum SHA256 berkas arsip kline dari `data.binance.vision`.
- Memverifikasi determinisme pembacaan data historis (tidak ada timestamp duplikat, tidak ada celah candle yang terlewat tanpa backfill).

## 6. Rencana Rollback
Data arsip disimpan di direktori terpisah `data/historical_bulk/` dan tidak mengubah jalur pembacaan default `data/`.

## 7. Estimasi Usaha
- ±20–30 jam kerja riset kuantitatif (pembuatan downloader bulk, parsing arsip ZIP, dan eksekusi event study).

## 8. Dampak pada Branch Lain
- `hermes/validation`: Sangat tinggi (Hermes akan memperoleh sampel data yang cukup untuk memvalidasi G-02/G-11).
- `codex/strategies`: Tinggi.
- `codex/sample`: Sangat tinggi.
