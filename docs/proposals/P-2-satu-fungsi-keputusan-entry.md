# P-2: Satu Fungsi Keputusan Entry Murni untuk Backtest dan Live

## 1. Konteks
Saat ini terdapat disparitas struktural antara jalur simulasi masa lalu dan eksekusi berjalan:
- `crates/backtest/src/engine.rs` mengevaluasi sinyal entri S1..S5 secara internal dengan logika pembulatan bar tertutup dan filter regim.
- `apps/apex/src/live.rs` (±L1200-1450) memiliki alur mandiri yang mencakup evaluasi sinyal, penilaian dewan, filter volume, dan rate-guard.
- Strategi S01 (Sweep Scalp) dan S02 (Sniper Reversal) hanya terhubung di jalur live (`crates/oms/src/yahudi_exec.rs`) dan sama sekali **belum memiliki jalur simulasi backtest**, sehingga tidak bisa divalidasi dengan pipeline G-02..G-11.
- Terdapat risiko look-ahead bias jika fitur kontekstual (makro, kNN regim) menggunakan bar yang belum sepenuhnya tertutup pada timestamp keputusan.

## 2. Opsi
1. **Opsi A (Status Quo):** Mempertahankan dua jalur terpisah.
   - *Kelebihan:* Tidak ada pekerjaan refactoring pada engine backtest yang sudah stabil.
   - *Kekurangan:* Risiko divergensi logika (apa yang diuji di backtest berbeda dari apa yang dieksekusi di live); S01/S02 tetap tidak dapat dibacktest.
2. **Opsi B (Rekomendasi — Fungsi Murni `decide_entry()` Bersama):**
   - Mendefinisikan struktur input murni `DecisionContext` yang hanya berisi data historis yang sudah selesai (`closed_bars`, `regime_snapshot`, `macro_bias`, `account_risk_state`).
   - Membuat fungsi murni:
     ```rust
     pub fn decide_entry(ctx: &DecisionContext, strat: StrategyId) -> DecisionOutcome;
     ```
   - Baik `apex-backtest` maupun `live.rs` wajib memanggil fungsi ini sebagai otoritas tunggal keputusan sinyal.
   - Mengimplementasikan evaluasi S01 dan S02 ke dalam `decide_entry()` sehingga kedua strategi ini dapat dibacktest secara komprehensif.
3. **Opsi C (Trait Strategy Monomorfik Dinamis):** Mengabstraksi setiap strategi ke dalam trait dengan dynamic dispatch (`Box<dyn Strategy>`). Menambah overhead alokasi memori yang tidak diperlukan pada mesin scalping.

## 3. Rekomendasi
Pilih **Opsi B**. Logika keputusan sinyal dipindahkan ke crate `apex_strategies` sebagai fungsi murni deterministik tanpa efek samping I/O.

## 4. Risiko
- Potensi kebocoran masa depan (look-ahead bias) jika input ke `decide_entry` menyertakan candle yang sedang berjalan.
- Penambahan beban memori bila snapshot konteks disalin secara tidak efisien pada setiap bar.

## 5. Rencana Test
- **Uji Paritas (Parity Test):** Memasukkan serangkaian candle sintetis yang sama ke `decide_entry` melalui jalur unit test backtest dan harness live, lalu memverifikasi bahwa keputusan (Arah, Harga Rencana, Alasan Skip) bernilai identik 100%.
- **Uji Anti-Look-Ahead:** Memverifikasi bahwa data pada timestamp $T$ tidak dapat mengakses bar dengan `open_time \ge T`.

## 6. Rencana Rollback
Bila ditemukan perbedaan perilaku pada sinyal S1 di live, revert commit penyatuan dan kembalikan pemanggilan langsung ke logika lokal `live.rs`.

## 7. Estimasi Usaha
- ±16–20 jam kerja engineer Rust senior (pembuatan fungsi murni, penulisan adapter backtest untuk S01/S02, dan verifikasi paritas).

## 8. Dampak pada Branch Lain
- `hermes/validation`: Sangat tinggi (memungkinkan Hermes menjalankan validasi 10 gerbang pada S01/S02).
- `codex/strategies`: Tinggi (Codex harus mengarahkan strategi baru ke `decide_entry`).
- `codex/sample`: Rendah.
