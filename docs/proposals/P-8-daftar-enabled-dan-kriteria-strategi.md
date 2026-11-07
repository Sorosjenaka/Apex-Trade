# P-8: Pengetatan Daftar `enabled` dan Kriteria Penambahan Strategi Baru

## 1. Konteks
Konfigurasi saat ini pada `config/strategies.toml` memuat:
```toml
enabled = ["S1", "S01", "S02"]
```
Fakta teknis yang terungkap:
- S1 adalah satu-satunya strategi yang memiliki pipeline backtest lengkap dan riwayat validasi formal di `crates/backtest`.
- S01 (Sweep Scalp) dan S02 (Sniper Reversal) adalah strategi eksperimental yang diintegrasikan langsung ke runtime eksekusi tanpa backtest engine (lihat P-2) dan berstatus **NO-GO**.
- Mengaktifkan strategi yang belum tervalidasi pada daftar `enabled` menciptakan beban kognitif bagi operator dan memperluas permukaan error pada pemantauan paper trading.
- Di sisi lain, menonaktifkan S01 dan S02 secara prematur dari mode paper akan menghentikan pengumpulan data telemetri forward untuk evaluasi kedua strategi tersebut.

## 2. Opsi
1. **Opsi A (Status Quo):** Membiarkan `enabled = ["S1", "S01", "S02"]` pada mode paper.
   - *Kelebihan:* Data forward S01 dan S02 tetap terkumpul di `audit/trades_telemetry.jsonl` untuk dipelajari.
   - *Kekurangan:* Sinyal bersaing pada slot alokasi risiko yang sama; pembacaan review menjadi bias karena bercampurnya strategi yang memiliki backtest dengan yang tidak.
2. **Opsi B (Rekomendasi — Pengetatan Bertahap Berbasis Gate Status):**
   - Batasi `enabled = ["S1"]` secara default untuk pengujian stabilitas fase 1 (G1 demo).
   - Buat aturan tata kelola baku: **Strategi baru dilarang dimasukkan ke daftar `enabled`** kecuali memenuhi dua prasyarat mutlak:
     1. Telah memiliki harness simulasi event-driven di `crates/backtest` (melalui penyatuan `decide_entry`, lihat P-2).
     2. Telah lolos 10 gerbang validasi deterministik (G-02 s/d G-11) dengan status resmi `GO`.
   - Untuk S01 dan S02, sediakan mode flag khusus `--experimental-strategies` atau pisahkan ke file konfigurasi terpisah `strategies.experimental.toml` selama masa pengumpulan data riset.
3. **Opsi C (Hapus Permanen S01 dan S02):** Menghapus kode S01/S02 sepenuhnya. Berisiko membuang potensi riset sebelum sempat diuji di backtest.

## 3. Rekomendasi
Pilih **Opsi B**. Fokuskan seluruh sumber daya komputasi dan pengawasan pada satu strategi utama (S1) sampai lolos seluruh gerbang forward G1/G2, sebelum memperluas universe strategi.

## 4. Risiko
- Pengumpulan data empiris untuk evaluasi performa live S01/S02 tertunda hingga jalur backtest selesai dibangun.

## 5. Rencana Test
- Uji isolasi strategi: buktikan bahwa saat `enabled = ["S1"]`, engine hanya mengevaluasi sinyal S1 dan tidak mengalokasikan slot posisi untuk kode lain.
- Uji penolakan boot: engine menolak start pada mode live bila terdapat strategi non-GO di dalam daftar `enabled`.

## 6. Rencana Rollback
Daftar `enabled` adalah nilai konfigurasi TOML murni di `config/strategies.toml` yang dapat diubah kembali kapan saja tanpa kompilasi ulang.

## 7. Estimasi Usaha
- ±2–4 jam kerja konfigurasi dan dokumentasi.

## 8. Dampak pada Branch Lain
- `hermes/validation`: Sangat positif (fokus validasi tidak terpecah).
- `codex/strategies`: Sedang (Codex perlu mengimplementasikan backtest sebelum mengaktifkan strategi baru).
- `codex/sample`: Rendah.
