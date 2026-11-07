# Panduan & Daftar Centang Kesiapan Go-Live (Apex-Trade)

> Dokumen ini adalah gerbang kendali resmi sebelum mesin beralih dari fase riset menuju eksekusi nyata pada Binance USDⓈ-M Futures.
> **Aturan Utama:** Agen AI dilarang menyalakan live atau mengirim order. Seluruh otorisasi live wajib ditandatangani oleh manusia pemilik sistem.

---

## 1. Jalur Kesiapan Bertahap (G0 → G1 → G2 → G3 → G4)

| Gerbang | Deskripsi & Prasyarat | Penanggung Jawab | Bukti yang Wajib Dilampirkan | Status Saat Ini |
|---|---|---|---|---|
| **G0 — Validasi Riset (GO)** | Strategi memenuhi 10 gerbang validasi deterministik (G-02 s/d G-11) sesuai `docs/plan/Rules.md`. | Agen & Manusia | Ringkasan `apex validate --strategies <ID>` dengan status `GO`. | **NO-GO** (Semua strategi saat ini riset) |
| **G1 — Uji Forward Demo** | Berjalan di Binance Futures Demo (`--stage demo`) minimal **14 hari** berturut-turut dan mengumpulkan minimal **30 trade**. | Manusia (eksekusi) & Agen (analisis) | Laporan `apex forward-gate --stage demo --since <YYYY-MM-DD>` berstatus **PASS** (nol `SL_MISSING`, nol `RULE_VIOLATION`). | Belum dimulai |
| **G2 — Shadow Trading (Mainnet Data)** | Berjalan di data nyata WebSocket Binance (`--mode paper`, `--stage shadow`) minimal **7 hari** untuk memvalidasi paritas sinyal, rekonsiliasi, dan latensi clock. | Manusia (eksekusi) & Agen (analisis) | Laporan `apex forward-gate --stage shadow --since <YYYY-MM-DD>` berstatus **PASS**; integritas audit tanpa anomali. | Belum dimulai |
| **G3 — Live Ukuran Terkecil (Micro Live)** | Eksekusi riil dengan ukuran minimum (\$1 risk per trade, modal \$60, margin 25x cross). Kriteria berhenti harian (-\$6 / 6 SL) dan lantai ekuitas (\$40) terkunci keras. | Manusia | Hasil verifikasi `apex preflight --target live` berstatus **PASS** (15/15 pemeriksaan hijau) sebelum proses pertama dinyalakan. | Terkunci |
| **G4 — Skalasi Ukuran Posisi** | Keputusan menaikkan batas risiko hanya boleh diambil bila terdapat bukti empiris dari trade nyata yang terkumpul. | Manusia | Laporan `apex review --since <YYYY-MM-DD> --out reports/review.md` dengan $n \ge 100$, E[R] positif pada selang kepercayaan 95% block-bootstrap per minggu ISO. | Terkunci |

---

## 2. Prasyarat Akun & Kredensial Binance (Khusus Manusia)

Pemeriksaan ini melibatkan kepatuhan hukum, yurisdiksi, dan keamanan akun yang tidak dapat dan tidak boleh diverifikasi secara otomatis oleh AI:

- [ ] **Kelayakan Akun:** Akun Binance memiliki akses resmi ke Binance USDⓈ-M Futures sesuai hukum dan yurisdiksi tempat tinggal pengguna. (Manusia)
- [ ] **Kunci API Khusus:** Membuat API Key terpisah untuk masing-masing lingkungan (Key Demo untuk `demo-fapi.binance.com`, Key Live untuk `fapi.binance.com`). Dilarang mencampur key. (Manusia)
- [ ] **Izin API Minimum (Least Privilege):**
  - [x] *Enable Reading* = Aktif
  - [x] *Enable Futures* = Aktif
  - [x] *Enable Withdrawals* = **NONAKTIF (WAJIB MATI)**
- [ ] **IP Whitelist:** Kunci API Live dikunci ke alamat IP statis VPS Linux yang menjalankan engine (`ipRestrict: true`). (Manusia)
- [ ] **Konfigurasi Akun Futures di Binance:**
  - Mode Posisi: **One-Way Mode** (`dualSidePosition: false`, bukan Hedge Mode).
  - Mode Aset Margin: **Single-Asset Margin** (`multiAssetsMargin: false`, margin USDT).
  - Margin Type: **CROSSED** pada seluruh simbol universe.
  - Leverage: Disetel ke 25x pada setiap simbol universe.
- [ ] **Saldo Dompet USDⓈ-M:** Saldo USDT tersedia di dompet Futures $\ge \$60.00$ (di atas lantai ekuitas \$40.00 dan penyangga margin).

---

## 3. Infrastruktur & Operasional VPS Linux (Khusus Manusia)

- [ ] **Spesifikasi & Host:** VPS Linux 64-bit dengan IP statis, akses SSH berbasis kunci publik, firewall egress hanya mengizinkan port 443 outbound ke domain resmi Binance dan webhook notifikasi.
- [ ] **Sinkronisasi Waktu (NTP):** Paket `chrony` aktif dan tersinkronisasi presisi tinggi (`chronyc tracking` offset $\le 10\text{ ms}$).
- [ ] **Supervisi Layanan (systemd):** Unit file `deploy/apex.service` terpasang di `/etc/systemd/system/apex.service` dengan hak akses terbatas pengguna `apex`.
- [ ] **Kanal Notifikasi:** Variabel environment `APEX_NOTIFY_URL` disetel pada `/etc/apex/apex.env` (izin `0600`) dan telah terverifikasi menerima pesan melalui `apex preflight --target demo --notify-test`.
- [ ] **Pencadangan State:** Skrip backup cron berjalan setiap jam mencadangkan direktori `/var/lib/apex/state/` ke lokasi terisolasi.
- [ ] **Rotasi Log:** `journald` atau logrotate dikonfigurasi agar berkas audit/log tidak memenuhi partisi disk (kapasitas kosong disk $\ge 1\text{ GiB}$).

---

## 4. Runbook Operasional & Kedaruratan (Emergency Procedures)

### 4.1 Menghentikan Mesin Seketika (Emergency Kill Switch)
```bash
# Opsi 1: Aktifkan file kill switch (engine yang sedang berjalan akan memblokir entry baru)
touch /var/lib/apex/state/KILL

# Opsi 2: Menghentikan engine dan memerintahkan perataan seluruh posisi terbuka
/opt/apex/bin/apex --config /etc/apex/config kill --flatten --reason "Darurat pasar/anomali sistem"

# Opsi 3: Mematikan service secara paksa
sudo systemctl stop apex
```

### 4.2 Menutup Posisi Secara Manual Melalui Aplikasi Binance
Bila koneksi VPS putus total atau terjadi kegagalan perangkat keras:
1. Buka aplikasi Binance resmi (Mobile atau Web).
2. Masuk ke menu **Futures USDⓈ-M** → tab **Positions**.
3. Klik tombol **Close All Positions** atau **Market Close** pada setiap posisi aktif.
4. Periksa tab **Open Orders** dan batalkan seluruh order `STOP_MARKET` / `LIMIT` sisa.

### 4.3 Tindakan Penanganan Kasus `UnknownSubmit` & `SL_MISSING`
- **`UnknownSubmit`:** Bila log mencatat `UNKNOWN_SUBMIT_RECORDED`, engine OMS secara otomatis memantau status order selama siklus rekonsiliasi. Jangan mematikan engine sebelum state terkonfirmasi; jika engine terhenti mendadak, periksa status order di antarmuka web Binance sebelum menyalakan ulang.
- **`SL_MISSING`:** Jika telemetri atau alert notifikasi memicu event `SL_MISSING` (posisi ada di bursa tanpa stop loss terpasang $> 2\text{ detik}$), engine akan mencoba memasang ulang hingga 3 kali; bila tetap gagal, engine mengeksekusi perataan posisi (`flatten`) dan menyalakan kill switch otomatis sesuai Golden Rule GR-8.

---

## 5. Lembar Pengesahan Manusia (Final Sign-Off)

> Centang semua butir di bawah ini sesaat sebelum mengubah `arm = true` pada `config/engine.toml`:

- [ ] Seluruh butir Bagian 1, 2, dan 3 telah berstatus **HIJAU**.
- [ ] Perintah `apex preflight --target live` telah dijalankan di server produksi dan menghasilkan **OVERALL: PASS**.
- [ ] Uji asap `apex demo-smoke` telah berhasil dijalankan di lingkungan Demo Binance.
- [ ] Nomor telepon darurat dan akses aplikasi Binance siap di tangan operator.
- [ ] Variabel `APEX_LIVE_CONFIRM=I_UNDERSTAND_LIVE_ORDERS` telah dipasang pada environment produksi.

**Nama Operator / Pemilik:** ___________________________  
**Tanggal Otorisasi:** ___________________________  
**Tanda Tangan:** ___________________________  
