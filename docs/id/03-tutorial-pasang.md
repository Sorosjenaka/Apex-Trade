# Tutorial Pemasangan

Panduan langkah demi langkah untuk mengompilasi, menguji, dan menjalankan **Apex Trade** di mesin lokal maupun VPS Anda. Pertahankan konfigurasi bawaan **mode simulasi (`PAPER`)** sampai Anda memahami seluruh alur kerja dan metrik validasinya.

## 1. Prasyarat

- **Rust Toolchain ≥ 1.85** (`rustup` — pada Windows gunakan target `x86_64-pc-windows-msvc` bersama *Visual Studio C++ Build Tools*).
- **Python ≥ 3.10** (hanya pustaka standar Python untuk menjalankan server terminal web `dashboard/server.py`).
- **Git**.
- *(Opsional untuk Testnet/Live)* Akun **Binance USDT-M Futures** dengan API Key berizin *Read + Futures Trading* (**tanpa** izin *Withdraw*). Tidak dibutuhkan untuk mode `PAPER`.

## 2. Kloning Repositori

```powershell
git clone https://github.com/Sorosjenaka/Apex-Trade.git
cd Apex-Trade
```

## 3. Periksa Konfigurasi Aman

Secara bawaan, berkas `config/engine.toml` sudah terkunci dalam mode aman:

```toml
mode = "paper"   # simulasi lokal menggunakan data pasar publik nyata
arm  = false     # kunci pengaman eksekusi nyata tetap OFF
```

Untuk melihat daftar variabel lingkungan opsional (tanpa perlu mengubah apa pun pada mode `paper`), periksa berkas `.env.example`.

> **PENTING:** Mesin Rust **tidak** membaca berkas `.env` secara otomatis agar kunci rahasia tidak pernah tersedot secara tidak sengaja dari direktori kerja. Kunci API (bila kelak diperlukan untuk `testnet`/`live`) hanya diberikan melalui variabel lingkungan sesi proses.

## 4. Kompilasi & Jalankan Pengujian Unit

TLS menggunakan `rustls` murni sehingga tidak memerlukan instalasi OpenSSL eksternal:

```powershell
cargo build --release --bin apex
cargo test --workspace
```

Binary hasil optimasi akan tersedia di `target-msvc\release\apex.exe` (sesuai konfigurasi `.cargo/config.toml`).

## 5. Menjalankan Mesin Simulasi (`PAPER`) & Dashboard Web

### Cara A — Dua Terminal Manual

1. **Terminal 1 — Mesin Inti Scalping (`PAPER`)**:

   ```powershell
   .\target-msvc\release\apex.exe run --mode paper
   ```

2. **Terminal 2 — Dashboard & Terminal Web (`:8787`)**:

   ```powershell
   python dashboard\server.py
   ```

### Cara B — Otomatis via Watchdog (Direkomendasikan di Windows)

Jalankan penjaga proses yang otomatis menyalakan mesin `paper` serta dashboard port `8787` di latar belakang dan menghidupkannya ulang bila berhenti:

```powershell
powershell -ExecutionPolicy Bypass -File scripts\apex_watchdog.ps1
```

## 6. Verifikasi Instalasi

1. Buka peramban ke **`http://127.0.0.1:8787`**.
2. Periksa status mesin dari CLI:

   ```powershell
   .\target-msvc\release\apex.exe status
   .\target-msvc\release\apex.exe screen --top 15
   ```

3. Cek endpoint telemetri JSON konsol:
   - `http://127.0.0.1:8787/api/markets/scan` — kandidat pemindaian 7-gate.
   - `http://127.0.0.1:8787/api/live/mesh` — status 21 agen & riwayat sidang dewan.
   - `http://127.0.0.1:8787/api/universe/audit` — hasil penyaringan kelas aset kripto.

## 7. Mengunduh Data Historis & Menjalankan Backtest / Validasi

```powershell
# Unduh klines publik (tanpa API key)
.\target-msvc\release\apex.exe fetch --from 2025-06-01 --to 2026-04-30 --data data

# Jalankan backtest per strategi & skenario stres biaya
.\target-msvc\release\apex.exe backtest --data data --per-strategy
.\target-msvc\release\apex.exe backtest --data data --strategies S1 --stress
```

## 8. Prosedur Beralih ke Testnet / Live (Hanya Bila Sudah Siap)

1. Uji terlebih dahulu di **Binance Futures Testnet** (`--mode testnet`) sebelum mempertimbangkan modal sungguhan.
2. Setel kredensial hanya pada environment sesi terminal:

   ```powershell
   $env:BINANCE_API_KEY = "ISI_API_KEY_ANDA"
   $env:BINANCE_API_SECRET = "ISI_SECRET_ANDA"
   ```

3. Ubah `arm = true` di `config/engine.toml` secara sadar, pastikan strategi telah berstatus `GO` di `config/strategies.toml`, dan untuk mode `live` wajib menambahkan:

   ```powershell
   $env:APEX_LIVE_CONFIRM = "I_UNDERSTAND_LIVE_ORDERS"
   .\target-msvc\release\apex.exe run --mode live
   ```

4. Untuk menghentikan entri baru seketika kapan saja (*kill switch*):

   ```powershell
   .\target-msvc\release\apex.exe kill --reason "manual_stop"
   ```

## Pemecahan Masalah Singkat

| Gejala | Solusi |
|---|---|
| Port `8787` sudah terpakai proses lain | Jalankan `scripts\apex_watchdog.ps1` (otomatis membersihkan proses asing di port `8787`) atau ubah `$env:APEX_DASH_PORT` |
| `cargo build` gagal menulis `apex.exe` (*Access is denied*) | Hentikan proses `apex.exe` yang sedang aktif sebelum melakukan kompilasi ulang |
| HTTP `429` / `418` dari Binance publik | Biarkan *Rate-Limit Guard* melakukan *backoff* otomatis beberapa puluh detik; cache lokal dashboard tetap menyajikan data terakhir yang sehat |
