# P-3: Ledger Trial Global Terisolasi dan Identitas Holdout Statis

## 1. Konteks
Dalam validasi kuantitatif ketat (`docs/plan/Rules.md` §3 & G-01..G-11), Deflated Sharpe Ratio (DSR) dan Haircut Sharpe mensyaratkan pencatatan akurat terhadap jumlah percobaan ($N_{\text{trial}}$) yang pernah dijalankan oleh peneliti.
Saat ini:
- Pencatatan trial masih bersifat parsial pada berkas lokal per run (`reports/validation_summary.json`).
- Percobaan variasi parameter sweep manual yang gagal seringkali tidak tercatat secara global, membuka celah p-hacking bawah sadar.
- Tanggal interval holdout dapat dimasukkan secara bebas melalui flag CLI (`--from`, `--to`, `--unlock-holdout`), dan terdapat inkonsistensi dokumentasi (antara `2026-05-01..2026-10-31` di AGENTS.md vs `2026-07-01..2026-11-04` di Rules.md/backtest).

## 2. Opsi
1. **Opsi A (Status Quo):** Menyerahkan tanggung jawab pencatatan trial kepada disiplin manual peneliti.
   - *Kelebihan:* Tidak ada hambatan I/O atau penguncian file saat eksperimen.
   - *Kekurangan:* Pelanggaran integritas saintifik; DSR menjadi over-optimistik karena $N_{\text{trial}}$ terhitung lebih rendah dari kenyataan.
2. **Opsi B (Rekomendasi — Ledger Global Append-Only dengan Reservasi Atomik):**
   - Buat berkas ledger persisten pada `research/trials_ledger.jsonl`.
   - Kunci unik trial:
     $$\text{TrialKey} = \text{BLAKE3}(\text{StrategyParams} \cup \text{DataManifest} \cup \text{Universe} \cup \text{GitCommit})$$
   - Reservasi atomik sebelum eksekusi dimulai (`OpenOptions::create_new(true)` + `fsync`).
   - Setiap variasi parameter sweep dihitung secara otomatis sebagai trial baru.
   - Menghitung $N_{\text{efektif}}$ melalui klastering korelasi (Optimal Number of Clusters / ONC) **tanpa melonggarkan ambang batas gate validasi G-01..G-11**.
   - Interval holdout dikunci sebagai konstanta bertanda tangan di `config/` dan tidak dapat di-override lewat argumen CLI umum.
3. **Opsi C (Database Server Terpusat):** Menggunakan basis data eksternal (PostgreSQL/SQLite). Berlebihan untuk single-developer/small-team repo dan menambah dependensi eksternal.

## 3. Rekomendasi
Pilih **Opsi B**. Format JSONL append-only menjamin transparansi riwayat riset, portabel di git, dan dapat diaudit secara independen.

## 4. Risiko
- Potensi bottleneck konkurensi bila beberapa proses riset dijalankan serentak pada mesin yang sama (dapat dimitigasi dengan retry backoff penguncian file).
- Ledger yang membengkak seiring ribuan sweep (dapat dikompresi atau diindeks terpisah).

## 5. Rencana Test
- Uji reservasi atomik ganda: dua proses yang mengeksekusi parameter identik secara bersamaan wajib mendeteksi duplikasi.
- Uji penghitungan $N_{\text{trial}}$: membuktikan bahwa $N$ meningkat pada setiap variasi run dan nilai DSR terkoreksi sesuai rumus resmi Bailey & López de Prado.

## 6. Rencana Rollback
Bila mekanisme ledger mengalami lock contention, nonaktifkan pemeriksaan melalui variabel environment debug `APEX_ALLOW_UNLOGGED_TRIALS=1` tanpa mengubah logika strategi.

## 7. Estimasi Usaha
- ±14–18 jam kerja engineer kuantitatif / Rust senior.

## 8. Dampak pada Branch Lain
- `hermes/validation`: Sangat tinggi (Hermes akan mengintegrasikan pembacaan ledger ke perhitungan DSR).
- `codex/strategies`: Sedang (Codex wajib menyertakan manifes parameter pada setiap eksperimen).
- `codex/sample`: Rendah.
