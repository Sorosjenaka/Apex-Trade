# P-5: Strategi Penanganan Panic dan Ketahanan State Engine

## 1. Konteks
Saat ini, penanganan panic pada aplikasi `apex` belum dikonfigurasi secara eksplisit di profil Cargo (menggunakan default Rust `panic = "unwind"`).
- Pada `crates/oms/src/lib.rs:12` dan modul live, kami telah menerapkan `#![cfg_attr(not(test), deny(clippy::unwrap_used, clippy::expect_used, clippy::panic))]` untuk memastikan tidak ada pemanggilan panic eksplisit.
- Namun demikian, pada task asynchronous `tokio::spawn` (seperti user-stream dan supervisor stream publik di `crates/oms/src/supervision.rs`), panic tak terduga (misalnya out-of-memory atau bug internal library) berisiko menyebabkan *silent task death* di mana task thread berhenti tanpa mematikan proses utama, atau memicu unwinding di tengah penulisan I/O state.
- Perbedaan antara `panic = "abort"` dan `panic = "unwind"` memiliki implikasi kritis terhadap integritas durabilitas berkas state `oms_state.json`.

## 2. Opsi
1. **Opsi A (Status Quo — Panic Unwind Standar):**
   - *Kelebihan:* Task tokio yang mengalami panic dapat ditangkap menggunakan `std::panic::catch_unwind` atau `JoinHandle`.
   - *Kekurangan:* State memori yang ditinggalkan saat unwinding dapat berada dalam kondisi korup (*poisoned lock*), dan runtime tokio tidak selalu mematikan seluruh binary, menyisakan proses zombie.
2. **Opsi B (Rekomendasi — `panic = "abort"` + Supervisi Systemd + Boot Reconcile):**
   - Tetapkan `panic = "abort"` pada profil `[profile.release]` di `Cargo.toml`.
   - Bila panic terjadi di bagian mana pun dari engine, proses langsung berhenti seketika (*fail-fast*).
   - Manajer proses OS (`systemd` via `Restart=always` di `deploy/apex.service`) akan mendeteksi kegagalan proses dan meluncurkan ulang binary `apex` dalam 5 detik.
   - Prosedur boot `reconcile` OMS (`crates/oms/src/manager.rs:450-510`) membaca `oms_state.json` atau memulihkan dari `oms_state.json.bak` untuk mengadopsi posisi aktif di bursa Binance dan memastikan proteksi SL tetap terpasang.
3. **Opsi C (Isolasi Multi-Process IPC):** Memisahkan OMS dan feed data ke dalam dua binary terpisah yang berkomunikasi lewat Unix domain socket. Terlalu rumit dan menambah latensi antar-proses.

## 3. Rekomendasi
Pilih **Opsi B**. Dalam sistem eksekusi finansial berlatensi rendah, *fail-fast* dengan pemulihan rekonsiliasi deterministik jauh lebih aman daripada melanjutkan proses yang status memorinya tidak lagi dapat dijamin (*corrupted invariants*).

## 4. Risiko
- Restart proses membutuhkan waktu re-koneksi WebSocket (±1–2 detik) di mana pembacaan sinyal baru tertunda sejenak.
- Mengharuskan berkas state selalu berada dalam kondisi durable (`sync_all` sebelum ACK bursa), yang telah diselesaikan pada WP-2.

## 5. Rencana Test
- Uji injeksi panic sintetis pada worker thread: buktikan bahwa proses langsung keluar dengan status non-zero tanpa merusak integritas `oms_state.json`.
- Uji bootstrap pemulihan: jalankan engine pasca-abort, verifikasi seluruh trade aktif terbaca dan SL diverifikasi ke bursa.

## 6. Rencana Rollback
Kembalikan baris `panic = "abort"` pada `Cargo.toml` ke default `unwind` melalui satu commit revert.

## 7. Estimasi Usaha
- ±6–8 jam kerja engineer Rust senior (pengujian systemd failure recovery dan verifikasi fsync durability).

## 8. Dampak pada Branch Lain
- `hermes/validation`: Tidak ada (Hermes bekerja pada backtest murni).
- `codex/strategies`: Rendah.
- `codex/sample`: Tidak ada.
