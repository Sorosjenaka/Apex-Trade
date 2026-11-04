# Desain: Apex Scalper RS

> v1.0 · 5 Nov 2026 (WIB). Dashboard **baca-saja** (FR-033) + 3 aksi kontrol (ARM, DISARM, KILL). Bentuk data: [→ Schema §2, §3].

## 1. Prinsip Desain

1. **Status dulu, angka kedua** — ARM, mode, dan stop harian terbaca dalam ≤ 5 dtk tanpa klik (US-01, NFR-13).
2. **Aksi berbahaya butuh niat eksplisit** — ARM butuh mengetik frasa `ARM testnet`/`ARM mainnet`. KILL selalu terlihat dan cukup 1 klik + 1 konfirmasi (US-02, US-11, NFR-14).
3. **Alasan selalu terlihat** — setiap sinyal yang tidak dieksekusi menampilkan kode SKIP-nya (US-05, US-06, US-07, US-08, US-09).
4. **Waktu dalam WIB, data dalam UTC** — UI menampilkan WIB dengan label "WIB"; batas hari trading ditandai "reset 07:00 WIB" (US-07).
5. **Tanpa angka palsu** — field kosong tampil "—", bukan 0 (US-01).

## 2. Layout Utama

```
┌───────────────────────────────────────────────────────────────────────────────┐
│ APEX SCALPER RS   [TESTNET]   ● DISARMED        Equity $60.00   05 Nov 14:02 WIB │
│                                                 [ARM…]  [DISARM]  [■ KILL]      │
├───────────────┬───────────────┬───────────────┬───────────────────────────────┤
│ Hari ini (UTC)│ SL 0/6        │ PnL −$0.00/−$6│ Cooldown: — │ Posisi 0/2        │
├───────────────┴───────────────┴───────────────┴───────────────────────────────┤
│ POSISI TERBUKA                                                                  │
│ Simbol  Sisi  Entry    SL (%)        TP       Qty    Margin  Risk   uPnL  Prot.  │
│ SOLUSDT LONG  182.40   178.10 (2.36) 191.00  0.26   $1.90   $0.98  +0.21 ✔ SL✔TP │
├─────────────────────────────────────────────────────────────────────────────────┤
│ SCREENER (update 14:00 WIB)                     filter: [TREND saja ▾]          │
│ Simbol  State       Votes ADX  ER   CHOP SRV   ATR%1h Spread Funding  Alasan     │
│ SOLUSDT TREND_UP    +4    31.2 0.41 38.5 0.52  0.92   1.05   0.010%   —          │
│ ETHUSDT RANGING      0    17.9 0.12 61.0 0.04  0.61   0.04   0.009%   CHOP≥55    │
├─────────────────────────────────────────────────────────────────────────────────┤
│ SINYAL TERAKHIR                         │ TRADE HARI INI                        │
│ 13:45 S1 SOLUSDT LONG ORDERED           │ 09:12 S3 SUIUSDT SHORT SL  −1.02R      │
│ 13:30 S3 ENAUSDT SHORT SKIP_COOLDOWN    │ 11:40 S1 XRPUSDT LONG  TP  +1.89R      │
└─────────────────────────────────────────┴───────────────────────────────────────┘
```

Modal ARM: judul "Aktifkan trading (TESTNET)", input teks, tombol "Aktifkan" nonaktif sampai input = frasa tepat. Di bawah input: daftar syarat FR-022 dengan centang (akun OK, stop harian tidak aktif, equity ≥ $40).

## 3. Design System

| Token | Hex | Pemakaian |
|---|---|---|
| bg | `#0E1116` | latar |
| panel | `#161B22` | kartu |
| text | `#E6EDF3` | teks utama |
| muted | `#8B949E` | label |
| long / profit | `#2EA043` | LONG, PnL +, TREND_UP |
| short / loss | `#F85149` | SHORT, PnL −, TREND_DOWN, KILL |
| warn | `#D29922` | cooldown, RANGING, SKIP |
| armed | `#A371F7` | badge ARMED |
| testnet | `#388BFD` | badge TESTNET |
| mainnet | `#F0883E` | badge MAINNET |

- **Tipografi:** Inter 14 px (UI), JetBrains Mono 13 px (angka, harga, id). Angka rata kanan.
- **Komponen:**
  - Kartu (radius 8 px, padding 16 px).
  - Badge state (pill, 12 px uppercase).
  - Tombol primer (ARM, ungu) dan tombol bahaya (KILL, merah, selalu di kanan atas).
  - Modal konfirmasi.
  - Tabel padat (baris 32 px).
  - Toast (atas-kanan, 5 dtk).

## 4. Behavior (FR P0 yang punya permukaan UI/API)

| FR | Loading | Error | Kosong | Sukses |
|---|---|---|---|---|
| FR-001 | — | banner merah "Engine tidak merespons (127.0.0.1:8787)" setelah 2 polling gagal | — | titik hijau di header |
| FR-022 ARM | tombol spinner, input terkunci | toast merah dengan `error.message` (mis. "Leverage ETHUSDT 20 ≠ 25") | — | badge berubah ARMED ungu; toast "ARMED testnet 14:02 WIB" |
| FR-023 KILL | tombol spinner ≤ 3 dtk | toast merah + tetap tampil DISARMED bila `arm=false` terbaca | — | toast "Dimatikan, 2 entry dibatalkan" |
| FR-026 status | skeleton kartu | banner kuning "Data status > 15 dtk" | "Tidak ada posisi" | data tampil |
| FR-016 stop harian | — | — | — | kartu SL berubah merah "STOP HARIAN — reset 07:00 WIB" |
| FR-015 cooldown | — | — | "Cooldown: —" | "Cooldown s/d 15:02 WIB (mm:ss)" kuning |
| FR-019 SL_MISSING | — | banner merah tetap sampai di-dismiss: "Posisi X ditutup: SL gagal; engine DISARM" | — | — |
| FR-021 recon | — | banner kuning "RECON_DIFF: …" | — | — |
| FR-010/FR-009 sinyal | skeleton | — | "Belum ada sinyal hari ini" | baris dengan badge decision |

## 5. Responsif

- Desktop ≥ 1024 px: layout §2.
- HP 360–767 px (NFR-16): header + tombol KILL sticky di atas; kartu ringkasan 2×2; tabel menjadi daftar kartu per posisi/simbol; screener hanya menampilkan TREND_*. Tidak ada scroll horizontal pada 360 px.
