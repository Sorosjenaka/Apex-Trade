# Review integrasi - BELUM MERGE KE MASTER

Waktu review: 2026-11-05. Cabang kerja Codex: `codex/integration-review`, berasal dari `integrate/v1` commit d55c9b4. Master masih 07a768a. Dua commit gabungan di integrate/v1 sudah dibuat Hermes; belum ada merge ke master dan REPORT_MERGE.md belum tersedia di master. T-C3 belum dimulai karena user meminta menunggu merge ke master.

## Perlindungan validasi pada branch review
- G-01 tetap INCONCLUSIVE karena command belum memverifikasi checksum dan prefix replay; status `tested_pass` tidak boleh true.
- G-06 tetap INCONCLUSIVE: enam jendela parameter beku bukan walk-forward dengan refit per fold. Output mencatat hasil jendela sebagai diagnostik terpisah.
- Stress G-05 dinilai pada trade VAL, memerlukan >=60 trade VAL stress; ringkasan juga menyimpan metrik full secara terpisah.
- Sampel IS/VAL dihitung hanya bila signal, entry dan exit berada dalam segmen yang sama, berurutan dan exit bukan EndOfData. G-11 juga hanya menghitung trade yang seluruhnya berada pada jendela holdout; tidak ada holdout yang dijalankan.
- Durasi annualisasi G-07 mengikuti rentang VAL yang benar-benar diuji. Kegagalan menulis atau sinkronisasi ledger menyebabkan error sebelum hasil trial ditampilkan, bukan diam-diam dilewati. JSON tidak menandai G-09 lulus ketika sampel G-02 kurang.
- Tidak mengubah sizing, arm/mode, safety live, threshold CHOP, atau perhitungan N trial. Semua strategi tetap NO-GO.

## Temuan yang belum selesai
1. Guard sekali jalan holdout belum cukup kuat: hash dihitung dari seluruh konfigurasi termasuk start/end; mengganti jendela menghasilkan hash berbeda. Ledger bergantung pada direktori `--out`, dan dicatat setelah run, sehingga ganti direktori atau kegagalan sebelum pencatatan dapat melewati riwayat. Jangan gunakan unlock holdout; dibutuhkan desain identitas parameter stabil dan reservasi ledger bersama yang atomik sebelum G-11 boleh dijalankan.
2. Sweep plateau memilih kandidat menggunakan PF OOS; angka OOS itu bukan evaluasi independen kandidat. Tidak ada dasar mengadopsi threshold hanya dari hasil tersebut. G-06 refit masih belum diimplementasikan, dan G-01 checksum/prefix belum diverifikasi oleh command validate.
3. Worktree C:\Users\FATHUR\Downloads\apex-scalper-rs masih di integrate/v1 dengan 3 file modifikasi belum di-commit oleh Hermes. Perubahan itu mengurangi hitungan trial DSR dengan mengabaikan trial <10 dan menurunkan default CHOP; tidak dimasukkan atau dihapus karena user melarang melonggarkan validasi dan meminta no-trade sideways. Data kerja Hermes dibiarkan utuh.
4. T-C3 harus menunggu master benar-benar diperbarui dan laporan merge final tersedia. User sebelumnya melarang Codex melakukan merge; instruksi mengambil alih pekerjaan Hermes belum menghapus larangan itu secara eksplisit.

## Tes
- `cargo test -p apex`: 5 lulus, 0 gagal.
- `cargo test -p apex-oms -p apex-market-reader`: OMS 27 lulus dan market reader 18 lulus, 0 gagal.
- `cargo test -p apex-strategies`: 14 lulus, 0 gagal.
- `cargo test -p apex-backtest`: 15 lulus, 0 gagal (588,14 detik), termasuk anti-look-ahead dan batas risiko.
- Tidak ada order, akses .env/API key, perubahan arm/mode, penggunaan holdout, push atau merge oleh Codex.
