# Ikhtisar dan Fitur

## Apa itu Apex Trade?

**Apex Trade** adalah mesin perdagangan otomatis berbasis **Rust** untuk kontrak perpetual **Binance USDT-M Futures** yang dirancang khusus untuk **mode Scalping (`SCALP`)**. Sistem ini tidak menggunakan strategi *swing* yang menahan posisi berhari-hari, melainkan berfokus pada eksekusi presisi berdurasi pendek dengan batas risiko tetap dan pengawasan berlapis.

Prinsip utamanya adalah **keamanan bawaan (*safe-by-default*)**: repositori dikemas dengan `mode = "paper"` (simulasi penuh memakai harga nyata) dan saklar eksekusi `arm = false` (terkunci), sehingga tidak ada order sungguhan yang pernah keluar ke bursa sebelum operator mengaktifkannya sendiri melalui tiga kunci pengaman terpisah.

> **Status Riset:** Seluruh strategi di repositori ini (`S1`–`S5`, `S01_SWEEP`, `S02_SNIPER`) saat ini berstatus **NO-GO** — belum ada *edge* yang tervalidasi melampaui gate G-01..G-11.

## Daftar Fitur Utama

### 1. Scanner Pasar Realtime 7-Gate & Asset Class Guard
- Menyaring seluruh daftar simbol USDT-M secara *default-deny* untuk memastikan hanya aset kripto murni yang dipantau (menolak komoditas, indeks saham, forex, dan *fan token*).
- Mengevaluasi setiap kandidat melewati 7 gerbang: volume 24 jam, rezim pasar, arah tren *Zero-Lag EMA*, skor komposit, kelayakan rencana entri, *Choppiness Index*, dan rentang volatilitas ATR.
- Dilengkapi **Binance Rate-Limit Guard** yang memantau bobot header HTTP dan menerapkan jeda *backoff* otomatis bila mendekati batas bursa, serta koneksi WebSocket terpisah pada rute resmi `/public`, `/market`, dan `/private` (host demo: `wss://demo-fstream.binance.com`).

### 2. Mesin Sinyal Scalping Multi-Timeframe
- Mengombinasikan pembacaan makro `1h` dan struktur setup `15m` (`research_entry_5m = false` secara bawaan) tanpa *look-ahead bias* (hanya memproses kline yang telah resmi tertutup).
- Menyediakan strategi `S1_TFP`–`S5_RMP` (memiliki jalur evaluasi paritas di simulator *backtest*) serta strategi eksperimen jalur live/paper `S01_SWEEP` (*Liquidity Sweep Reclaim*) dan `S02_SNIPER` (*Liquidity Sniper*) yang ditandai `validated: false` pada log audit karena belum memiliki paritas *backtest*.

### 3. Gerbang Risiko Deterministik (\$1 per Trade)
- Ukuran kuantitas dihitung mundur dari anggaran risiko tetap **\$1.00** dibagi jarak invalidasi *stop-loss*, lalu dibulatkan mengikuti `stepSize` dan `tickSize` simbol menggunakan `rust_decimal`.
- **Pita SL 2%–4%**: setup dengan jarak invalidasi terlalu sempit (rawan tersapu *noise*) atau terlalu lebar otomatis dilewati.
- **Volatility Shield**: menahan pembukaan posisi baru ketika terjadi lonjakan volatilitas ekstrem atau kondisi pasar menyamping (`CHOP_NO_TRADE`).

### 4. Dewan Aturan Deterministik 3-Peran & Arena Advisory
- **Dewan Pra-Entry Deterministik (`enforced = true`):** kumpulan aturan deterministik (`if/else`), bukan agen otonom:
  - **Vega (*Macro Scout*)** memeriksa matriks 6-kondisi `BTC.D × BTC` sebelum mengizinkan posisi pada *altcoin*.
  - **Arka (*Setup Judge*)** mengevaluasi kualitas konfluensi skor, probabilitas analog *k-NN*, dan keselarasan orderflow.
  - **Wardha (*Risk Officer*)** memvalidasi ketahanan jarak *stop-loss* terhadap ATR dan memilih varian proteksi struktural terbaik.
  - Bila konsensus dewan gagal tercapai atau muncul satu suara `NO_GO` berkeyakinan tinggi pada dewan, kandidat **ditolak**.
- **Arena Arah (*Direction Battle*, `enforced = false`)** menghitung skor tesis `LONG` lawan `SHORT` sebagai **telemetri *advisory* saja** (dicatat ke audit & dashboard, tidak memveto order).

### 5. Manajemen Posisi & OMS Otomatis
- Menyimpan state ke disk sebelum mengirim order (`write-ahead state`) dengan `clientOrderId` unik agar bebas dari duplikasi order.
- Memasang proteksi *Stop-Loss* dan *Take-Profit* melalui *Conditional Algo Order* (`MARK_PRICE`).
- **Anti-Orphan Reconciler** memeriksa kesesuaian antara buku lokal dan bursa secara berkala; **Trend-Flip Confirmation Guard** menutup posisi bila tren berbalik terkonfirmasi pada beberapa bar berturut-turut.

### 6. Terminal & Dashboard Web Gelap (`:8787`)
- Menyajikan grafik kline bersih tanpa teks penolakan yang mengganggu, ringkasan ekuitas, posisi aktif, kurva kinerja, serta endpoint telemetri JSON untuk konsol mikrostruktur dan dewan agen.

### 7. Siklus Belajar, Forensik 5-Whys & Validasi CPCV
- Setiap posisi tertutup dianalisis otomatis melalui **5-Whys Forensics**, perhitungan **Wilson 95% Confidence Lower Bound**, serta usulan penyesuaian parameter terbatas (*bounded tuning*).
- Tersedia mesin *backtest* dan validasi **CPCV (*Combinatorial Purged Cross-Validation*)** untuk menguji ketahanan strategi terhadap *overfitting*.

## Alur Singkat dari Hulu ke Hilir

1. **Saring** — ambil semesta pasangan kripto likuid dan cek kesehatan *rate-limit*.
2. **Baca Konteks** — petakan rezim global (`BTC.D × BTC`) dan mikrostruktur simbol (CVD, profil volume 48-bin, peta likuidasi).
3. **Bangkitkan Sinyal** — bentuk kandidat scalping (`S1_TFP`, `S01_SWEEP`, `S02_SNIPER`) lengkap dengan level entri, SL, dan TP.
4. **Uji Risiko & Sidang Dewan** — hitung ukuran posisi \$1 dan jalankan pemungutan suara 3-agen secara *fail-closed*.
5. **Eksekusi & Awasi** — jalankan di `MockExchange` (`PAPER`) atau bursa (`LIVE` bila di-`arm`), pasang pelindung SL/TP, dan pantau *trend-flip*.
6. **Evaluasi** — catat audit penuh dan jalankan forensik pasca-perdagangan.

Lanjutkan ke [02-alur-kerja-workflow.md](02-alur-kerja-workflow.md) untuk melihat diagram alur lengkapnya.
