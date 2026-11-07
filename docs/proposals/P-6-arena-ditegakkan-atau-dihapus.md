# P-6: Penegakan Aturan atau Penghapusan Modul Arena Sinyal

## 1. Konteks
Pada arsitektur awal sistem, terdapat dua layer evaluasi keputusan sebelum sinyal dieksekusi:
1. **Dewan Sinyal (`SignalCouncil`):** Aturan deterministik berbasis filter regim, spread, CVD, dan volume gate.
2. **Arena Pertarungan (`SignalArena`):** Logika heuristik perbandingan antar-kandidat strategi yang pada implementasi awalnya mengklaim melakukan "hard veto".

Namun, temuan hasil audit statis (Fase B, WP-4) membuktikan bahwa:
- Keputusan penolakan oleh Arena selama ini **tidak pernah ditegakkan** di jalur kritis (`crates/oms/src/artatrade_exec.rs:720`, sekarang berlabel `enforced: false`, `advisory: true`).
- Nilai ambang batas yang dilewatkan ke arena sebelumnya memakai literal konstan mentah (`72.0`, fallback `0.55`, `0.008`) tanpa bukti statistik kausalitas terhadap ekspektasi profit ($E[R]$).
- Mengizinkan komponen yang belum terbukti untuk memveto order berisiko mengurangi frekuensi perdagangan (*sample throughput*) tanpa memberikan perlindungan risiko yang nyata.

## 2. Opsi
1. **Opsi A (Status Quo):** Membiarkan Arena tetap berjalan dalam status *advisory* (hanya mencatat telemetri dan log audit tanpa memveto order).
   - *Kelebihan:* Tidak merusak struktur audit JSON yang sudah ada; data hipotetis arena tetap terkumpul untuk analisis di `apex review`.
   - *Kekurangan:* Menambah baris kode heuristik yang tidak aktif mengeksekusi proteksi.
2. **Opsi B (Rekomendasi — Protokol Ablasi Pra-Registrasi sebelum Penegakan Veto):**
   - Tetapkan aturan tata kelola: Arena **dilarang memveto order** sampai ada pengujian ablasi resmi (*ablation study*) pada data out-of-sample (minimal 100 trade) yang membuktikan bahwa:
     $$\Delta E[R] = E[R]_{\text{dengan arena}} - E[R]_{\text{tanpa arena}} > 0$$
     secara signifikan pada interval kepercayaan 95% block-bootstrap.
   - Bila bukti empiris tersebut tidak terpenuhi setelah 6 bulan pengumpulan data shadow/demo, modul Arena **wajib dihapus sepenuhnya** dari codebase untuk memangkas kompleksitas kode.
3. **Opsi C (Hapus Seketika):** Menghapus seluruh struct `SignalArena` dan log auditnya sekarang. Berisiko merusak kompatibilitas dashboard Python yang membaca field `arena` di JSON state.

## 3. Rekomendasi
Pilih **Opsi B**. Pertahankan Arena sebagai komponen advisory murni dengan klausul evaluasi berbasis data: buktikan efektivitasnya melalui uji ablasi terukur, atau hapus tuntas.

## 4. Risiko
- Operator atau pengembang baru mungkin salah mengira bahwa Arena aktif melindungi akun jika tidak membaca dokumen dan flag `enforced: false`.

## 5. Rencana Test
- Uji konsistensi audit: buktikan bahwa field `enforced: false` dan `inputs_defaulted` selalu tercatat benar di audit JSONL.
- Uji independensi: buktikan bahwa penolakan oleh Arena tidak pernah mengubah `action` menjadi `Reject` pada keputusan akhir dewan.

## 6. Rencana Rollback
Jika dewan dan pemilik repo memutuskan menghapus arena, commit penghapusan struct dapat diterapkan secara langsung tanpa mengganggu alur order.

## 7. Estimasi Usaha
- ±4–6 jam kerja untuk penyusunan pipeline pengujian ablasi pada modul `apex review`.

## 8. Dampak pada Branch Lain
- `hermes/validation`: Rendah.
- `codex/strategies`: Rendah.
- `codex/sample`: Rendah.
