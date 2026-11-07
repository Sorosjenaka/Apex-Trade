# P-7: Penataan Struktur Dokumen, Artefak Laporan, dan Konsistensi Gitignore

## 1. Konteks
Saat ini struktur repositori memiliki beberapa tumpang tindih tata kelola dokumentasi dan berkas pelacak:
1. **Laporan Agen di Root Repositori:**
   - Terdapat berkas laporan dari berbagai agen pada root repo: `REPORT_HERMES.md`, `REPORT_CODEX.md`, `REPORT_ANTIGRAVITY.md`, `UNDERSTANDING.md`. Berkas-berkas ini membuat root repositori menjadi padat.
2. **Kontradiksi pada `.gitignore`:**
   - Berkas `.gitignore` mengecualikan direktori `/reports/` dan `/research/`, namun beberapa file di dalam folder tersebut telah ter-commit ke git (`reports/validation_summary.json`). Hal ini membingungkan alur kerja git status.
3. **Pemisahan Folder Dokumen:**
   - Terdapat tiga folder terpisah: `docs/plan/` (spesifikasi awal), `docs/public/` (ringkasan alur kerja), dan `docs/id/` (dokumentasi bahasa Indonesia baru untuk kesiapan operasional).
4. **Kebocoran Path Lokal Mesin Asal:**
   - Laporan warisan `REPORT_HERMES.md` dan `REPORT_CODEX.md` mengandung path absolut Windows (`C:\Users\FATHUR\...`) yang melanggar prinsip kebersihan repositori publik/tim.

## 2. Opsi
1. **Opsi A (Status Quo):** Membiarkan file laporan dan folder dokumentasi di lokasinya sekarang.
   - *Kelebihan:* Tidak mengganggu rujukan link eksternal yang mungkin dibuat oleh pemilik repositori.
   - *Kekurangan:* Repositori terlihat tidak terstruktur; kontradiksi gitignore menyulitkan CI; path lokal membocorkan informasi lingkungan lokal.
2. **Opsi B (Rekomendasi — Konsolidasi Terkoordinasi ke `docs/reports/`):**
   - Buat direktori `docs/reports/` dan pindahkan seluruh `REPORT_*.md` ke folder tersebut dengan persetujuan eksplisit pemilik repo (menghormati H7/H13).
   - Perbarui `.gitignore` agar mengizinkan tracking laporan resmi (`!reports/*.json`, `!docs/reports/*.md`) sambil tetap mengabaikan output sementara.
   - Satukan hierarki dokumentasi:
     - `docs/specs/` (menggantikan `docs/plan/`)
     - `docs/ops/` (menggantikan `docs/id/` dan panduan deploy)
     - `docs/reports/` (seluruh laporan audit dan validasi berkala)
   - Lakukan pembersihan path absolut lokal pada berkas laporan warisan melalui skrip sanitasi terverifikasi.
3. **Opsi C (Hapus Laporan Lama):** Menghapus seluruh file laporan agen dan hanya menyisakan dokumen operasional. Menghilangkan rekam jejak audit sejarah perubahan sistem.

## 3. Rekomendasi
Pilih **Opsi B**. Dilakukan setelah fase hardening selesai melalui koordinasi dengan pemilik repositori.

## 4. Risiko
- Link markdown lama antar-dokumen dapat patah bila path relatif tidak diperbarui secara cermat.

## 5. Rencana Test
- Menjalankan linter link markdown untuk memastikan tidak ada broken link internal (`find . -name "*.md" | xargs ...`).
- Memverifikasi bahwa `git status` pasca-perubahan `.gitignore` tidak memunculkan berkas untracked tak terduga.

## 6. Rencana Rollback
Gunakan `git mv` kembali ke lokasi semula bila pemilik repo menginginkan laporan tetap berada di root.

## 7. Estimasi Usaha
- ±3–5 jam kerja dokumentasi dan sanitasi path.

## 8. Dampak pada Branch Lain
- `hermes/validation`: Sedang (perlu pembaruan lokasi `REPORT_HERMES.md`).
- `codex/strategies`: Sedang (perlu pembaruan lokasi `REPORT_CODEX.md`).
- `codex/sample`: Rendah.
