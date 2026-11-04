# AGENT.md — Otak Sesi untuk Apex Scalper RS

## Ringkasan proyek (≤ 10 baris)
- Engine trading **Rust** untuk Binance USDⓈ-M Futures, untuk 1 operator (soros way, WIB) dengan modal **$60 USDT**.
- Risk tetap **$1 neto per trade**, margin variabel = notional/25, SL struktural 2–4% (di luar rentang = skip), 25x CROSSED, max 2 posisi, stop harian 6 SL / −$6 (hari UTC), cooldown 60 menit global setelah SL.
- 5 strategi hipotesis (S1–S5) gabungan 28 skrip TradingView [→ STRATEGIES.md]; hanya trade di pasar trending [→ SCREENER.md].
- Tidak ada strategi yang boleh live sebelum lolos 11 gerbang [→ Rules §4.3] dan testnet.
- Basis kode: port modul Rust apex [→ APEX_MAPPING.md]. Angka kelayakan: [→ FEASIBILITY.md].

## Mode eksekusi aktif
**MODE-TERMINAL** (agen membangun dengan Cursor/terminal). Belum ada kode; posisi = blueprint selesai.

## Tech stack
Rust 1.87 · tokio · axum · reqwest · tokio-tungstenite · rust_decimal · sqlx (SQLite) · serde · tracing · secrecy · rayon · Vite+React+TS (dashboard). Detail [→ architecture §1].

## 10 Golden Rules (salinan Rules §1)
1. GR-1 Risk per trade = $1,00 neto (fee+slip termasuk); tanpa compounding.
2. GR-2 Margin TIDAK tetap: margin = notional/25; qty dibulatkan ke bawah; SKIP bila risk < $0,80 / notional < minNotional / qty < minQty; tanpa sizing acak.
3. GR-3 SL 2,00–4,00% dari entry; di luar = SKIP; SL tidak digeser.
4. GR-4 25x, CROSSED, one-way; mismatch = tidak boleh ARM.
5. GR-5 Maks 2 posisi (termasuk pending), 1 per simbol.
6. GR-6 Stop harian: 6 SL atau realized ≤ −$6 per hari UTC (reset 07:00 WIB).
7. GR-7 Cooldown 60 menit global setelah setiap SL.
8. GR-8 SL+TP di exchange via `/fapi/v1/algoOrder` ≤ 2 dtk; gagal 5 dtk → tutup + DISARM.
9. GR-9 ARM default false; hanya user yang ARM; testnet dulu; hanya strategi GO.
10. GR-10 Hanya entry di state TREND searah; equity < $40 → DISARM.

**Larangan agen (Rules §6):** jangan pernah membaca/mencetak `.env` atau secret; jangan kirim order mainnet dari test/riset; jangan ubah angka GR tanpa REVISI user.

## Struktur folder
[→ architecture §3]. Ringkas: `crates/{core,data,features,screener,strategies,risk,exec,backtest,validate}`, `apps/{engine,research-cli}`, `web/`, `config/`, `data/` (gitignored).

## Cara menjalankan & testing
[→ README]. Ringkas: `cargo test --workspace`; `cargo run -p apex-engine`; `curl -s 127.0.0.1:8787/api/health`.

## Peta ID cepat

| ID | Nama | Status |
|---|---|---|
| US-01..US-15 | lihat PRD §4.2 | todo |
| FR-001 | Health check | todo |
| FR-002 | Start aman | todo |
| FR-003 | Muat filter simbol | todo |
| FR-004 | Cek akun | todo |
| FR-005 | Sinkron waktu | todo |
| FR-006 | Ingest bar tertutup | todo |
| FR-007 | Deteksi celah data | todo |
| FR-008 | Screener | todo |
| FR-009 | Gate regime | todo |
| FR-010 | Sinyal strategi | todo |
| FR-011 | Sizing risk $1 | todo |
| FR-012 | Skip jarak SL | todo |
| FR-013 | Skip risk minimum | todo |
| FR-014 | Batas posisi | todo |
| FR-015 | Cooldown SL | todo |
| FR-016 | Stop harian | todo |
| FR-017 | Entry order | todo |
| FR-018 | SL/TP di exchange | todo |
| FR-019 | Gagal SL → tutup | todo |
| FR-020 | User data stream | todo |
| FR-021 | Rekonsiliasi | todo |
| FR-022 | ARM manual | todo |
| FR-023 | Kill switch | todo |
| FR-024 | Equity floor | todo |
| FR-025 | Audit log | todo |
| FR-026 | API status | todo |
| FR-027 | API trades & PnL harian | todo |
| FR-028 | API screener | todo |
| FR-029 | Downloader histori | todo |
| FR-030 | Replay portofolio | todo |
| FR-031 | Laporan validasi | todo |
| FR-032 | Trials ledger | todo |
| FR-033 | Dashboard baca-saja | todo |
| FR-034 | Mode jaringan | todo |
| FR-035 | Notifikasi Telegram (P2) | todo |
| NFR-01..NFR-19 | lihat PRD §6 | todo |
| S1..S5 | strategi | HIPOTESIS (belum divalidasi) |

## Posisi terakhir pengerjaan
- 5 Nov 2026 (WIB): blueprint v1.0 selesai (9 file + STRATEGIES/SCREENER/FEASIBILITY/APEX_MAPPING + pine/CATALOG.md).
- Langkah berikutnya: `BUILD 0` mulai T-0.1.
- Keputusan yang perlu dikonfirmasi user: A-02, A-03, A-07, A-12 (🔺).

## Matriks US → FR
| US | FR |
|---|---|
| US-01 | FR-001, FR-025, FR-026, FR-027, FR-033 |
| US-02 | FR-002, FR-004, FR-022, FR-034 |
| US-03 | FR-005, FR-017, FR-018, FR-019 |
| US-04 | FR-003, FR-011 |
| US-05 | FR-012, FR-013 |
| US-06 | FR-014 |
| US-07 | FR-016, FR-024 |
| US-08 | FR-015 |
| US-09 | FR-006, FR-007, FR-008, FR-009, FR-028 |
| US-10 | FR-020, FR-021 |
| US-11 | FR-023 |
| US-12 | FR-029 |
| US-13 | FR-010, FR-030 |
| US-14 | FR-031 |
| US-15 | FR-032 |

## ✅ Checklist Konsistensi (RULES §8)
1. ☑ Semua US terpetakan ke ≥ 1 FR (matriks di atas; 15/15).
2. ☑ FR P0+P1 (FR-001..FR-034) ada di PRD §7.1; P2 (FR-035) di §7.3; sisanya di §7.2.
3. ☑ NFR-01..NFR-19 punya angka target + cara verifikasi.
4. ☑ Nol deskripsi duplikat: rumus sizing hanya di Rules §4.1; ambang screener hanya di SCREENER.md; angka pasar hanya di FEASIBILITY.md; file lain merujuk.
5. ☑ BG-1..BG-3 dan PG-1..PG-4 punya metrik M-1..M-8 di PRD §3.3.
6. ☑ Setiap tugas ROADMAP merujuk FR/NFR/US-ID.
7. ☑ Pain point kedua segmen merujuk E-01..E-11 di PRD §2.3.
8. ☑ Requirement diuji biner (angka + satuan + kondisi); kata terlarang sudah dicek dengan grep.
