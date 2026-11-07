# P-4: Migrasi Trade Tertutup ke Berkas JSONL Append-Only

## 1. Konteks
Saat ini, berkas `oms_state.json` menyimpan seluruh riwayat perdagangan:
- `book.trades`: posisi yang sedang aktif berjalan.
- `book.closed`: daftar seluruh trade yang sudah ditutup sejak awal deployment (`crates/oms/src/manager.rs:70`).
- Seiring bertambahnya bulan dan ribuan transaksi, berkas `oms_state.json` akan terus membesar tanpa batas. Karena `StateStore::save()` melakukan serialisasi dan `fsync` seluruh struktur state pada setiap pembaruan order/fill, ukuran state yang membengkak akan memperlambat I/O jalur kritis eksekusi.
- Pada `apps/apex/src/live.rs` (±L760), sinkronisasi trade tertutup mengandalkan indeks memori `seen_closed: usize`.
- Strategi S5 (Regime kNN) mengandalkan fitur trade tertutup untuk pembaruan bobot memori.

## 2. Opsi
1. **Opsi A (Status Quo):** Membiarkan `book.closed` di dalam `oms_state.json`.
   - *Kelebihan:* Struktur data tunggal, tidak ada migrasi skema berkas.
   - *Kekurangan:* Degradasi performa I/O secara gradual; penulisan atomik `File::sync_all()` memakan waktu lebih lama setiap trade ditutup.
2. **Opsi B (Rekomendasi — Pemisahan ke `trades_closed.jsonl` Append-Only):**
   - Batasi `book.closed` di dalam `oms_state.json` hanya pada jendela $N$ trade terakhir (misalnya 50 trade terakhir untuk kebutuhan pemulihan langsung).
   - Setiap trade yang selesai ditutup langsung di-append ke berkas `audit/trades_closed.jsonl` (format 1 baris JSON per trade + `flush()`).
   - S5 kNN dan modul review membaca riwayat masa lalu dari `trades_closed.jsonl` melalui streaming iterator, bukan memuat array raksasa ke memori RAM.
3. **Opsi C (Rotasi Berkas Mingguan):** Memecah file state per minggu `oms_state_YYYY_Www.json`. Terlalu kompleks untuk pemulihan restart mesin.

## 3. Rekomendasi
Pilih **Opsi B**. Pendekatan ini mempertahankan performa O(1) pada penulisan state kritis OMS sambil menjaga seluruh riwayat transaksi tetap durable dan mudah diolah kembali.

## 4. Risiko
- Inkonsistensi data bila proses terbunuh di antara penulisan append JSONL dan penulisan state utama.
- Dampak pada korelasi kNN S5 bila indeks offset pembacaan trade tertutup tergeser.

## 5. Rencana Test
- Uji migrasi non-destruktif: memuat berkas `oms_state.json` lama yang memiliki 100 trade tertutup, memverifikasi seluruhnya tertransmigrasi ke `trades_closed.jsonl` tanpa duplikasi.
- Uji konsistensi `seen_closed` pada `live.rs` dengan skenario restart berulang.

## 6. Rencana Rollback
`oms_state.json` mempertahankan flag kompatibilitas mundur. Jika di-revert, trade dapat dibaca kembali dari JSONL dan digabungkan kembali ke array JSON lama.

## 7. Estimasi Usaha
- ±10–14 jam kerja engineer Rust senior.

## 8. Dampak pada Branch Lain
- `hermes/validation`: Rendah.
- `codex/strategies`: Sedang (khusus S5 kNN memory loader).
- `codex/sample`: Rendah.
