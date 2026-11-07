# P-1: Pemecahan Modul Monolitik dan Penataan Penamaan Domain

## 1. Konteks
Berdasarkan tinjauan arsitektur kode saat ini, berkas `apps/apex/src/live.rs` telah membengkak hingga ±2.276 baris kode dan merangkap beragam tanggung jawab yang berbeda:
- Inisialisasi loop trading, feed WebSocket publik, dan stream privat akun (`live.rs:180-350`).
- Logika konsensus dewan sinyal dan arena pertarungan (`live.rs:600-850`).
- Evaluasi filter volume, rate-guard, dan pemeriksaan cooldown (`live.rs:900-1150`).
- Penelusuran trace S1, perhitungan excursion bar 15m, dan sinkronisasi pasca-trade (`live.rs:1050-1350`).
- Logika rekonsiliasi posisi yatim dan mirror state (`live.rs:1600-1950`).

Selain itu, `crates/oms/src/artatrade_exec.rs` (±1.280 baris) dan `crates/oms/src/yahudi_exec.rs` (±340 baris) masih menggunakan nama merek warisan (`artatrade_*`, `yahudi_*`) yang mengaburkan fungsi teknis sebenarnya (seperti `council`, `progressive_policy`, `probation_guard`, `cooldown_guard`).

## 2. Opsi
1. **Opsi A (Status Quo):** Mempertahankan struktur berkas monolitik apa adanya.
   - *Kelebihan:* Tidak ada pergeseran baris atau risiko konflik git segera.
   - *Kekurangan:* Sulit diaudit, risiko tinggi regresi saat bugfix, coupling sangat ketat, melanggar batas tanggung jawab tunggal (SRP).
2. **Opsi B (Rekomendasi — Pemecahan Bertahap dengan Facade Re-export):**
   - Pecah `apps/apex/src/live.rs` menjadi modul sub-direktori:
     - `apps/apex/src/live/mod.rs` (entry point runner & loop)
     - `apps/apex/src/live/boot.rs` (inits, auth, warm-up kline)
     - `apps/apex/src/live/entry.rs` (evaluasi sinyal, dewan, sizing)
     - `apps/apex/src/live/post_trade.rs` (excursion, telemetri, sync closed)
     - `apps/apex/src/live/reconcile.rs` (mirroring & orphan adoption)
   - Pecah `crates/oms/src/artatrade_exec.rs` dan `yahudi_exec.rs` menjadi modul fungsional:
     - `crates/oms/src/council.rs` (aturan deterministik dewan)
     - `crates/oms/src/rate_guard.rs` (pembatas frekuensi entry)
     - `crates/oms/src/trend_guard.rs` (konfirmasi trend-flip & cooldown)
     - `crates/oms/src/progressive.rs` (peringkat progresif advisory)
   - Gunakan `pub use` di `lib.rs` / `main.rs` sehingga seluruh impor publik tetap kompatibel 100%.
3. **Opsi C (Pemisahan Crate Penuh):** Memisahkan ke crate independen baru (`apex-council`, `apex-exchange`, `apex-risk`). Terlalu agresif untuk tahap sekarang dan memperpanjang waktu build.

## 3. Rekomendasi
Pilih **Opsi B**. Pemecahan dilakukan dalam serangkaian commit terisolasi dengan penambahan golden test pada setiap langkah untuk membuktikan invarian eksekusi tidak bergeser satu bit pun.

## 4. Risiko
- Potensi konflik merge dengan branch kerja lain yang mengedit `live.rs`.
- Kesalahan re-export visibilitas `pub(crate)` yang dapat memicu error kompilasi.

## 5. Rencana Test
- Menjalankan seluruh test suite unit dan integration (`cargo test -p apex -p apex-oms`).
- Golden test snapshot: output JSON `s1_trace`, keputusan dewan, dan status sinkronisasi sebelum dan sesudah refactoring wajib identik.

## 6. Rencana Rollback
Karena arsitektur mempertahankan facade `pub use`, rollback dapat dilakukan dengan `git revert` pada commit pemecahan modul tanpa mempengaruhi format berkas state di disk.

## 7. Estimasi Usaha
- ±12–16 jam kerja engineer Rust senior (pemecahan modul, pengujian golden snapshot, pembersihan warning).

## 8. Dampak pada Branch Lain
- `hermes/validation`: Rendah (Hermes fokus pada `crates/backtest`).
- `codex/strategies`: Sedang (jika Codex memodifikasi antarmuka sinyal di `live.rs`, pemecahan ini membutuhkan rebase yang terkoordinasi).
- `codex/sample`: Rendah (hanya dataset sampling).
