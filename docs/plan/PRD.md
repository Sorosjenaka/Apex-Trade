# Apex Scalper RS

> PRD v1.0 · 5 Nov 2026 (WIB) · Blueprint mengikuti RULES.md v1.2 · Bahasa Indonesia
> Dokumen pendamping: architecture.md, Schema.md, Design.md, Rules.md, ROADMAP.md, AGENT.md, CHANGELOG.md, README.md. Lampiran riset: STRATEGIES.md, SCREENER.md, FEASIBILITY.md, APEX_MAPPING.md, `../pine/CATALOG.md`.

## 1. Ringkasan Eksekutif

1. **Produk:** Apex Scalper RS adalah mesin trading otomatis berbasis **Rust** untuk Binance USDⓈ-M Futures, ditambah pipeline riset/validasi kuantitatif dalam bahasa yang sama.
2. **Pengguna:** satu trader-operator ritel dengan modal awal **$60 USDT** yang juga meneliti strategi sendiri [→ PRD §4.1].
3. **Masalah:** sistem apex yang ada tidak memasang SL di exchange, sizing-nya melanggar aturan risiko, dan bukti backtest-nya tidak valid [→ PRD §2].
4. **Solusi dan pembeda:** aturan risiko tetap (risk $1, margin variabel, SL 2–4%, 25x CROSS, max 2 posisi, stop harian, cooldown) ditegakkan di kode dan di exchange. Lima strategi hipotesis hasil gabungan 28 skrip TradingView hanya boleh live setelah lolos gerbang validasi profesional, dan hanya pada pasar yang diklasifikasi trending oleh screener.
5. **Kenapa sekarang:** sejak 2025-12-09 Binance memindahkan order kondisional ke endpoint Algo Order, sehingga eksekusi lama harus ditulis ulang. Modul Rust apex yang layak pakai (`scalp_portfolio.rs`, `validation.rs`) sudah tersedia untuk di-port [→ APEX_MAPPING.md].

## 2. Problem Statement

### 2.1 Pernyataan Masalah Inti
Trader ritel bermodal $60 kesulitan menjalankan scalping Binance USDⓈ-M dengan risiko tetap $1 per trade ketika mengandalkan sistem apex yang ada. Akar penyebabnya: SL/TP hanya disimpan di memori bot, sizing mengabaikan filter simbol dan jarak SL, dan hasil backtest terkontaminasi look-ahead serta pemakaian ulang data OOS. Akibatnya kesiapan live diperkirakan hanya 10–15%, dan backtest terbaik turun ke profit factor 0,39 setelah stress biaya.

### 2.2 Siapa yang Terdampak
- Trader-Operator Modal Kecil
- Peneliti Strategi (peran yang sama, mode riset)

### 2.3 Bukti & Dampak

| ID | Bukti | Kualitatif/Kuantitatif | Sumber |
|---|---|---|---|
| E-01 | SL/TP tidak dipasang di exchange; hanya dicek tiap 60 detik dari memori. Bot crash = posisi tanpa stop | Kuantitatif (60 dtk) | REVIEW apex C1 (`execution.py:217-219, 277-329`) |
| E-02 | Backtest terbaik: OOS MaxDD 37,5%, PF stress 0,39; data OOS dipakai ulang ≈40 versi skrip | Kuantitatif | REVIEW apex C8 |
| E-03 | Look-ahead di `backtest.rs:199-207` dan `rolling(center=True)` | Kualitatif | REVIEW apex H1, H4 |
| E-04 | `risk.rs` menaikkan risk ke 1,67% equity saat equity ≥ $120 dan menerima SL sampai 6% | Kuantitatif | `rust-engine/src/risk.rs:29-36` |
| E-05 | BTCUSDT butuh notional minimum $80,30 (minQty 0,001 × $80.297,9), sedangkan risk $1 pada SL 2% hanya mengizinkan $47,19 | Kuantitatif | FEASIBILITY §2 (exchangeInfo 2026-11-04T18:58Z) |
| E-06 | ISOLATED 25x terlikuidasi pada gerak 3,0–3,6%, yaitu di dalam SL 4% | Kuantitatif | FEASIBILITY §3; REVIEW H3 |
| E-07 | SL 2% pada ETH = 13,5 ATR 5m; median waktu harga bergerak ±2% = 17 jam | Kuantitatif | FEASIBILITY §4 |
| E-08 | 522 perp USDT berstatus TRADING; 4 dari 30 teratas memiliki leverage maks < 25x; 3 gagal aturan risk minimum | Kuantitatif | FEASIBILITY §2 |
| E-09 | 2 dari 28 skrip TradingView memakai `request.security` tanpa `[1]` (nilai TF tinggi berubah selama bar berjalan); 1 skrip source terkunci | Kuantitatif | `pine/CATALOG.md` |
| E-10 | Endpoint `/fapi/v1/order` menolak STOP_MARKET/TAKE_PROFIT_MARKET (error -4120) sejak 2025-12-09 | Kualitatif | developers.binance.com change log 2025-11-06 |
| E-11 | Waktu operator untuk memantau pasar sideways vs trending secara manual tidak terukur [ASUMSI] | Kualitatif | Percakapan user 5 Nov 2026 |

## 3. Goals & Metrik Keberhasilan

### 3.1 Business Goals
- **BG-1:** Menghasilkan keputusan GO/NO-GO tertulis untuk 5 strategi (S1–S5), berdasarkan 11 gerbang di Rules §4, dalam ≤ 8 minggu sejak Fase 0 dimulai.
- **BG-2:** Menjaga equity akun live ≥ $40 (floor) dengan ekspektansi realized ≥ +0,05R per trade setelah 100 trade live, dalam ≤ 6 bulan sejak ARM pertama di mainnet.
- **BG-3:** Nol posisi tanpa SL di exchange lebih dari 5 detik selama testnet dan live, sejak Fase 3.

### 3.2 Product Goals
- **PG-1:** 100% entry memiliki SL% ∈ [2,00%; 4,00%] dan risk aktual ∈ [$0,80; $1,00], sejak Fase 3.
- **PG-2:** 0 pelanggaran aturan max 2 posisi, cooldown 60 menit, dan stop harian di log audit, untuk seluruh periode testnet dan live.
- **PG-3:** 0 entry pada simbol berstate RANGING/BLOCKED, untuk seluruh periode testnet dan live.
- **PG-4:** Selisih rata-rata slippage realized vs model ≤ 0,05R pada ≥ 30 trade testnet, sebelum ARM mainnet.

### 3.3 Metrik Keberhasilan

| Metrik | Rumus | Baseline | Target | Cara Ukur |
|---|---|---|---|---|
| M-1 Keputusan strategi (BG-1) | jumlah strategi dengan verdict GO/NO-GO tertulis | 0 | 5 dalam ≤ 8 minggu | file `reports/validation/<strategi>.json` + ringkasan ROADMAP |
| M-2 Equity floor (BG-2) | min(equity harian, 00:00 UTC) | $60 | ≥ $40 | tabel `daily_risk` |
| M-3 Ekspektansi live (BG-2) | Σ PnL neto / (n × $1) | tidak ada | ≥ +0,05R pada n ≥ 100 | `GET /api/trades` agregat |
| M-4 Posisi tanpa SL (BG-3) | jumlah interval posisi terbuka tanpa algo SL aktif > 5 dtk | tidak terukur (apex: SL di memori) | 0 | event `SL_MISSING` di `events_audit` |
| M-5 Kepatuhan SL%/risk (PG-1) | entry patuh / total entry | — | 100% | query `orders` + `positions` |
| M-6 Pelanggaran risk gate (PG-2) | count `RULE_VIOLATION` | — | 0 | `events_audit` |
| M-7 Entry di RANGING (PG-3) | entry dengan `screener_state ∉ {TREND_UP, TREND_DOWN}` | — | 0 | join `signals` × `screener_snapshots` |
| M-8 Gap slippage (PG-4) | mean(realized_slip_R − model_slip_R) | — | ≤ 0,05R, n ≥ 30 | laporan testnet T-3.12 |

## 4. Target Users & User Stories

### 4.1 Target Users

| Segmen | Karakteristik | Pain Point (→ §2.3) | Nilai yang Dicari | Prioritas |
|---|---|---|---|---|
| Trader-Operator Modal Kecil | 1 orang (soros way), WIB, modal $60 USDT, Binance USDⓈ-M, risk tetap $1 | E-01, E-04, E-05, E-06, E-08, E-10, E-11 | Eksekusi patuh aturan tanpa diawasi terus; SL selalu ada di exchange | P0 |
| Peneliti Strategi | Orang yang sama dalam mode riset; memakai 28 skrip TradingView | E-02, E-03, E-07, E-09 | Bukti statistik yang tidak bisa menipu diri sendiri sebelum memakai uang | P0 |

### 4.2 User Stories

**US-01** Sebagai Trader-Operator, saya ingin melihat status kesehatan engine dan akun dalam satu endpoint/halaman, sehingga saya tahu engine hidup dan ARM-nya dalam ≤ 5 detik.
- Given engine berjalan / When saya membuka `GET /api/status` / Then respons memuat `arm`, `mode`, `open_positions`, `daily_sl_count`, `cooldown_until` dalam ≤ 300 ms.

**US-02** Sebagai Trader-Operator, saya ingin hanya saya yang bisa meng-ARM engine secara manual, sehingga tidak ada order nyata tanpa keputusan saya.
- Given engine baru start / When saya cek status / Then `arm=false`.
- Given `arm=false` / When sinyal valid muncul / Then 0 order dikirim dan sinyal dicatat `NOT_ARMED`.

**US-03** Sebagai Trader-Operator, saya ingin setiap posisi langsung punya SL dan TP di exchange, sehingga crash bot tidak membuat posisi tanpa stop.
- Given entry terisi / When ≤ 2 detik berlalu / Then 1 algo STOP_MARKET dan 1 algo TAKE_PROFIT_MARKET (`closePosition=true`, `MARK_PRICE`) berstatus aktif.
- Given SL gagal dipasang 3× / When 5 detik sejak fill / Then posisi ditutup MARKET reduceOnly dan `arm=false`.

**US-04** Sebagai Trader-Operator, saya ingin ukuran posisi dihitung dari risk $1 dan jarak SL, sehingga margin menyesuaikan dan kerugian SL ≈ $1.
- Given entry 100, SL 98, stepSize 0,001 / When sizing dihitung / Then qty dibulatkan ke bawah, risk aktual ∈ [$0,80; $1,00], margin = notional/25.

**US-05** Sebagai Trader-Operator, saya ingin setup dengan SL di luar 2–4% atau risk < $0,80 dilewati, sehingga aturan risiko tidak pernah dilanggar.
- Given SL struktural 1,8% / When sinyal diproses / Then status `SKIP_SL_RANGE` dan 0 order.

**US-06** Sebagai Trader-Operator, saya ingin maksimal 2 posisi terbuka, sehingga risk terbuka ≤ $2.
- Given 2 posisi/pending aktif / When sinyal ke-3 muncul / Then `SKIP_MAX_POSITIONS`.

**US-07** Sebagai Trader-Operator, saya ingin trading berhenti setelah 6 SL atau rugi realized $6 dalam satu hari UTC, sehingga rugi harian ≤ 10% modal awal.
- Given 6 exit SL hari ini / When sinyal baru / Then `SKIP_DAILY_STOP` sampai 00:00 UTC (07:00 WIB).

**US-08** Sebagai Trader-Operator, saya ingin jeda 60 menit setelah setiap SL, sehingga tidak ada balas dendam beruntun.
- Given SL terisi pukul 10:00:00 UTC / When sinyal pukul 10:59:59 UTC / Then `SKIP_COOLDOWN`; sinyal pukul 11:00:00 UTC lolos gate ini.

**US-09** Sebagai Trader-Operator, saya ingin engine hanya entry pada simbol trending searah sinyal, sehingga pasar sideways dihindari.
- Given state simbol RANGING / When sinyal muncul / Then `SKIP_REGIME`.

**US-10** Sebagai Trader-Operator, saya ingin engine merekonsiliasi posisi dan order dengan exchange saat start dan berkala, sehingga catatan lokal selalu sama dengan exchange.
- Given ada posisi di exchange tanpa SL / When engine start / Then SL dipasang ≤ 5 detik atau posisi ditutup, dan `arm=false`.

**US-11** Sebagai Trader-Operator, saya ingin tombol kill switch, sehingga saya bisa menghentikan entry baru seketika.
- Given `arm=true` / When `POST /api/kill` / Then `arm=false` dan semua order entry pending dibatalkan ≤ 3 detik; SL/TP posisi tetap aktif.

**US-12** Sebagai Peneliti, saya ingin mengunduh histori klines 5m/15m/1h dan aggTrades dari data.binance.vision, sehingga backtest memakai data asli dengan delta taker.
- Given rentang tanggal & simbol / When perintah download / Then file lengkap dengan checksum cocok dan 0 celah bar.

**US-13** Sebagai Peneliti, saya ingin replay portofolio yang menegakkan semua aturan akun dan biaya, sehingga hasil backtest setara dengan aturan live.
- Given dataset dan strategi / When replay / Then output memuat trades, equity, fee, funding, dan alasan skip, dengan hasil identik pada 2 kali jalan (deterministik).

**US-14** Sebagai Peneliti, saya ingin laporan gerbang validasi (walk-forward, Monte Carlo, Deflated Sharpe, plateau, stress biaya, holdout), sehingga keputusan GO/NO-GO bersifat biner.
- Given hasil replay / When validasi dijalankan / Then laporan JSON memuat verdict PASS/FAIL/INCONCLUSIVE untuk 11 gerbang Rules §4.

**US-15** Sebagai Peneliti, saya ingin setiap percobaan parameter tercatat sebagai trial, sehingga Deflated Sharpe tidak dimanipulasi oleh banyaknya percobaan.
- Given replay dijalankan / When selesai / Then 1 baris `trials_ledger` bertambah dengan hash parameter + hash data.

## 5. Functional Requirements

| ID | Nama Fitur | Perilaku (WHEN-THEN) | Prioritas | US Terkait |
|---|---|---|---|---|
| FR-001 | Health check | Ketika `GET /api/health` dipanggil, sistem HARUS menjawab HTTP 200 `{"status":"ok"}` dalam ≤ 100 ms. | P0 | US-01 |
| FR-002 | Start aman | Ketika engine start, sistem HARUS menetapkan `arm=false` terlepas dari nilai config/env apa pun. | P0 | US-02 |
| FR-003 | Muat filter simbol | Ketika engine start dan setiap 60 menit, sistem HARUS memuat ulang exchangeInfo (tickSize, stepSize, minQty, minNotional, status) dan leverage bracket; gagal 3× berturut → `arm=false`. | P0 | US-04 |
| FR-004 | Cek akun | Ketika engine start, sistem HARUS memverifikasi positionMode = one-way, dan untuk tiap simbol universe marginType = CROSSED dan leverage = 25 (mengaturnya bila berbeda); bila verifikasi gagal sistem HARUS menolak `arm=true`. | P0 | US-02, US-04 |
| FR-005 | Sinkron waktu | Ketika offset `serverTime` Binance vs jam lokal > 500 ms, sistem HARUS menolak kirim order baru sampai offset ≤ 500 ms. | P0 | US-03 |
| FR-006 | Ingest bar tertutup | Ketika WS kline mengirim event `x=true`, sistem HARUS menyimpan bar itu dan memicu evaluasi; event `x=false` HARUS diabaikan oleh strategi. | P0 | US-09, US-13 |
| FR-007 | Deteksi celah data | Ketika ada celah ≥ 1 bar pada simbol, sistem HARUS memblok simbol itu (`BLOCKED:data_gap`) sampai backfill REST mengisi celah. | P0 | US-09 |
| FR-008 | Screener | Ketika bar 1h tertutup (+5 dtk), sistem HARUS menghitung state TREND_UP/TREND_DOWN/RANGING/BLOCKED per simbol menurut SCREENER.md §2–§4 dalam ≤ 10 dtk untuk ≤ 30 simbol. | P0 | US-09 |
| FR-009 | Gate regime | Ketika sinyal muncul pada simbol yang state-nya tidak searah sinyal, sistem HARUS mencatat `SKIP_REGIME` dan tidak mengirim order. | P0 | US-09 |
| FR-010 | Sinyal strategi | Ketika bar TF trigger tertutup, sistem HARUS mengevaluasi strategi berstatus GO di `config/strategies.toml` dan mencatat setiap sinyal ke tabel `signals`. | P0 | US-13 |
| FR-011 | Sizing risk $1 | Ketika sinyal lolos gate, sistem HARUS menghitung qty, notional, dan margin dengan rumus Rules §4.1 (qty dibulatkan ke bawah ke stepSize). | P0 | US-04 |
| FR-012 | Skip jarak SL | Ketika SL% < 2,00% atau > 4,00%, sistem HARUS mencatat `SKIP_SL_RANGE` tanpa mengubah SL. | P0 | US-05 |
| FR-013 | Skip risk minimum | Ketika risk aktual < $0,80, notional < minNotional, atau qty < minQty, sistem HARUS mencatat `SKIP_SIZE`. | P0 | US-05 |
| FR-014 | Batas posisi | Ketika jumlah posisi terbuka + entry pending ≥ 2 atau simbol sudah punya posisi/pending, sistem HARUS mencatat `SKIP_MAX_POSITIONS`. | P0 | US-06 |
| FR-015 | Cooldown SL | Ketika sebuah posisi tertutup oleh SL, sistem HARUS menolak entry baru di semua simbol selama 3.600 dtk sejak waktu fill SL. | P0 | US-08 |
| FR-016 | Stop harian | Ketika jumlah exit SL hari UTC berjalan = 6 atau PnL realized neto hari itu ≤ −$6,00, sistem HARUS menolak entry baru sampai 00:00:00 UTC berikutnya. | P0 | US-07 |
| FR-017 | Entry order | Ketika sinyal lolos semua gate dan `arm=true`, sistem HARUS mengirim LIMIT GTX (atau MARKET untuk strategi berlabel `entry=market`) dengan `newClientOrderId` deterministik dan `newOrderRespType=RESULT`; entry LIMIT yang belum terisi dalam 3.600 dtk HARUS dibatalkan. | P0 | US-03, US-04 |
| FR-018 | SL/TP di exchange | Ketika entry terisi (sebagian atau penuh), sistem HARUS mengirim algo STOP_MARKET dan TAKE_PROFIT_MARKET (`algoType=CONDITIONAL`, `closePosition=true`, `workingType=MARK_PRICE`) dan mendapat konfirmasi aktif dalam ≤ 2 dtk. | P0 | US-03 |
| FR-019 | Gagal SL → tutup | Ketika SL belum terkonfirmasi aktif 5 dtk setelah fill (setelah 3 percobaan), sistem HARUS menutup posisi dengan MARKET reduceOnly, menetapkan `arm=false`, dan mencatat `SL_MISSING`. | P0 | US-03 |
| FR-020 | User data stream | Ketika event `ORDER_TRADE_UPDATE`, `ALGO_UPDATE`, atau `ACCOUNT_UPDATE` diterima, sistem HARUS memperbarui state order/posisi lokal dalam ≤ 200 ms dan memperpanjang listenKey setiap 30 menit. | P0 | US-10 |
| FR-021 | Rekonsiliasi | Ketika engine start dan setiap 60 dtk, sistem HARUS membandingkan posisi (`/fapi/v3/positionRisk`), order terbuka, dan algo order terbuka dengan state lokal; selisih HARUS dicatat `RECON_DIFF`, dan posisi tanpa SL HARUS ditangani seperti FR-019. | P0 | US-10 |
| FR-022 | ARM manual | Ketika `POST /api/arm` diterima dengan token lokal valid dan frasa `ARM <mode>` yang benar, sistem HARUS menetapkan `arm=true` hanya bila FR-004 lulus, stop harian tidak aktif, dan equity ≥ $40. | P0 | US-02 |
| FR-023 | Kill switch | Ketika `POST /api/kill` diterima, sistem HARUS menetapkan `arm=false` dan membatalkan semua entry pending dalam ≤ 3 dtk tanpa menyentuh SL/TP posisi terbuka. | P0 | US-11 |
| FR-024 | Equity floor | Ketika equity (wallet + unrealized) < $40,00, sistem HARUS menetapkan `arm=false` dan mencatat `EQUITY_FLOOR`. | P0 | US-07 |
| FR-025 | Audit log | Ketika keputusan gate, order, fill, atau perubahan ARM terjadi, sistem HARUS menulis 1 baris `events_audit` dengan timestamp UTC ms dan tanpa nilai secret. | P0 | US-01, US-10 |
| FR-026 | API status | Ketika `GET /api/status` dipanggil, sistem HARUS mengembalikan bentuk data Schema §2 dalam ≤ 300 ms. | P0 | US-01 |
| FR-027 | API trades & PnL harian | Ketika `GET /api/trades?date=YYYY-MM-DD` dipanggil, sistem HARUS mengembalikan trade hari UTC itu plus agregat PnL, R, fee, funding. | P1 | US-01 |
| FR-028 | API screener | Ketika `GET /api/screener` dipanggil, sistem HARUS mengembalikan snapshot screener terakhir untuk semua simbol universe. | P1 | US-09 |
| FR-029 | Downloader histori | Ketika `apex-research download` dijalankan, sistem HARUS mengunduh klines dan aggTrades dari data.binance.vision, memverifikasi file `.CHECKSUM`, dan melaporkan 0 celah atau daftar celah. | P0 | US-12 |
| FR-030 | Replay portofolio | Ketika `apex-research replay` dijalankan, sistem HARUS mensimulasikan aturan Rules §4.1–§4.3 dengan biaya FEASIBILITY §1 dan menghasilkan output identik byte-per-byte pada 2 kali jalan dengan input sama. | P0 | US-13 |
| FR-031 | Laporan validasi | Ketika `apex-research validate` dijalankan, sistem HARUS menghasilkan verdict PASS/FAIL/INCONCLUSIVE untuk 11 gerbang Rules §4.3. | P0 | US-14 |
| FR-032 | Trials ledger | Ketika satu replay selesai, sistem HARUS menambah 1 baris `trials_ledger` (hash parameter, hash data, metrik). | P0 | US-15 |
| FR-033 | Dashboard baca-saja | Ketika dashboard dibuka di `http://127.0.0.1:5173`, sistem HARUS menampilkan status, posisi, screener, dan trade dari API Schema §2, dengan refresh 5 dtk. | P1 | US-01 |
| FR-034 | Mode jaringan | Ketika `APEX_MODE=testnet`, sistem HARUS memakai `https://demo-fapi.binance.com` dan WS testnet; mainnet hanya ketika `APEX_MODE=mainnet` dan frasa ARM `ARM mainnet`. | P0 | US-02 |
| FR-035 | Notifikasi Telegram | Ketika event `SL_MISSING`, `EQUITY_FLOOR`, `DAILY_STOP`, atau `RECON_DIFF` terjadi, sistem HARUS mengirim pesan ke chat Telegram yang dikonfigurasi dalam ≤ 10 dtk. | P2 | US-01 |

## 6. Non-Functional Requirements

| ID | Kategori | Requirement (EKSPLISIT) | Target | Verifikasi | FR Terkait |
|---|---|---|---|---|---|
| NFR-01 | Performa | Latensi dari event bar close sampai order entry terkirim | p50 ≤ 150 ms, p95 ≤ 500 ms (universe ≤ 30 simbol, 5 strategi) | histogram `latency_signal_to_order_ms` 7 hari testnet | FR-010, FR-017 |
| NFR-02 | Performa | Fill entry → SL algo terkonfirmasi aktif | p95 ≤ 2.000 ms, maks ≤ 5.000 ms | metrik `latency_fill_to_sl_ms` testnet ≥ 30 trade | FR-018 |
| NFR-03 | Performa | Respons API lokal | `/api/health` p95 ≤ 100 ms; `/api/status` p95 ≤ 300 ms (1 klien) | `oha -n 1000` lokal | FR-001, FR-026 |
| NFR-04 | Keamanan | API key/secret hanya dibaca dari env proses backend; 0 kemunculan secret di log, DB, respons API, repo | 0 temuan | `gitleaks detect` + test grep log T-3.14 | FR-025 |
| NFR-05 | Keamanan | API HTTP hanya bind 127.0.0.1; endpoint POST butuh token lokal 32 byte | 0 port terbuka di 0.0.0.0 | `ss -ltnp` + test 401 tanpa token | FR-022, FR-023 |
| NFR-06 | Keamanan | API key Binance tanpa izin withdraw, dibatasi IP | 1 cek manual checklist | Rules §6, cek UI Binance oleh user | FR-004 |
| NFR-07 | Reliabilitas | Reconnect WS market & user stream setelah putus | ≤ 5 dtk, backfill REST untuk bar yang hilang | chaos test: putus jaringan 30 dtk di testnet | FR-006, FR-020 |
| NFR-08 | Reliabilitas | Uptime proses engine selama soak test | ≥ 99,5% dalam 72 jam | log supervisor (systemd restart count) | FR-001 |
| NFR-09 | Reliabilitas | Restart di tengah posisi terbuka | state pulih dan SL terverifikasi ≤ 10 dtk setelah start | test kill -9 di testnet | FR-021 |
| NFR-10 | Reliabilitas | Memori proses engine | RSS ≤ 300 MB dan pertumbuhan ≤ 10% selama 72 jam | `ps`/metrik setiap 5 menit | FR-006 |
| NFR-11 | Skalabilitas | Kapasitas universe | ≤ 40 simbol × 3 TF WS tanpa melewati NFR-01 | load test replay WS rekaman | FR-008 |
| NFR-12 | Skalabilitas | Volume data riset | 2 tahun × 30 simbol klines 5m/15m/1h; replay 1 strategi ≤ 120 dtk di 4 vCPU | `time apex-research replay` | FR-030 |
| NFR-13 | Usability | Waktu operator dari buka dashboard sampai tahu status ARM, posisi, dan stop harian | ≤ 5 dtk, 0 klik | uji manual dengan stopwatch | FR-033 |
| NFR-14 | Usability | Langkah kill switch | ≤ 2 klik atau 1 perintah curl | uji manual | FR-023 |
| NFR-15 | Kompatibilitas | Engine | Linux x86_64 (Ubuntu 22.04+), Rust stable ≥ 1.87 | CI build | FR-001 |
| NFR-16 | Kompatibilitas | Dashboard | Chrome/Edge ≥ 120, Firefox ≥ 121, lebar layar ≥ 360 px | uji manual 2 browser | FR-033 |
| NFR-17 | Reliabilitas | Determinisme replay | 2 jalan berurutan → hash output identik | test CI T-2.2 | FR-030 |
| NFR-18 | Reliabilitas | Kebenaran kausal | Prefix test: sinyal di t dari data terpotong di t = sinyal dari data penuh, 100% sampel | test CI T-2.7 | FR-008, FR-010 |
| NFR-19 | Reliabilitas | Rate limit Binance | Bobot request ≤ 50% batas `X-MBX-USED-WEIGHT-1M` (≤ 1.200 dari 2.400) | metrik header | FR-003, FR-021 |

## 7. Scope

### 7.1 In-Scope (MVP)

| FR-ID | Nama | Ukuran |
|---|---|---|
| FR-001 | Health check | S |
| FR-002 | Start aman | S |
| FR-003 | Muat filter simbol | S |
| FR-004 | Cek akun | M |
| FR-005 | Sinkron waktu | S |
| FR-006 | Ingest bar tertutup | M |
| FR-007 | Deteksi celah data | S |
| FR-008 | Screener | L |
| FR-009 | Gate regime | S |
| FR-010 | Sinyal strategi | L |
| FR-011 | Sizing risk $1 | S |
| FR-012 | Skip jarak SL | S |
| FR-013 | Skip risk minimum | S |
| FR-014 | Batas posisi | S |
| FR-015 | Cooldown SL | S |
| FR-016 | Stop harian | S |
| FR-017 | Entry order | M |
| FR-018 | SL/TP di exchange | M |
| FR-019 | Gagal SL → tutup | M |
| FR-020 | User data stream | M |
| FR-021 | Rekonsiliasi | L |
| FR-022 | ARM manual | S |
| FR-023 | Kill switch | S |
| FR-024 | Equity floor | S |
| FR-025 | Audit log | S |
| FR-026 | API status | S |
| FR-027 | API trades & PnL harian | S |
| FR-028 | API screener | S |
| FR-029 | Downloader histori | M |
| FR-030 | Replay portofolio | L |
| FR-031 | Laporan validasi | L |
| FR-032 | Trials ledger | S |
| FR-033 | Dashboard baca-saja | M |
| FR-034 | Mode jaringan | S |

### 7.2 Out-of-Scope (Non-Goals)

| Item | Alasan Eksplisit | Risiko jika dipaksakan |
|---|---|---|
| LLM/agen AI dalam keputusan trade | REVIEW C7: LLM membuka trade tanpa syarat; 0 bukti nilai tambah | Trade tak terkendali, tidak bisa divalidasi |
| Compounding / risk % equity | GR-1 risk tetap $1 | Melanggar aturan user; E-04 |
| Leverage ≠ 25x atau ISOLATED | E-06: ISOLATED 25x likuidasi 3,0–3,6% < SL 4% | Likuidasi sebelum SL |
| BTCUSDT sebagai simbol trading | E-05: min notional $80,30 > $47,19 | Risk > $1 atau order ditolak |
| Strategi berbasis histori order book | Tidak ada histori depth gratis → tidak bisa backtest jujur | Klaim tanpa bukti |
| Hedge mode (long+short simbol sama) | Max 1 posisi per simbol | State ganda, rekonsiliasi rumit |
| Grid/martingale/averaging down | Melanggar GR-1 dan GR-5 | Risk tak terbatas |
| Exchange selain Binance USDⓈ-M | 1 venue untuk MVP | Duplikasi kerja eksekusi |
| Akses dashboard dari internet | NFR-05 bind lokal | Kebocoran kontrol ARM |
| Tuning parameter otomatis saat live | REVIEW H8 | Overfit live, risk berubah diam-diam |

### 7.3 Post-MVP

| FR-ID/Nama | Ukuran | Fase target |
|---|---|---|
| FR-035 Notifikasi Telegram | S | Fase 4 [→ ROADMAP Fase 4] |
| Footprint aggTrades live (varian S2-F) | L | Fase 4 |
| Depth snapshot forward-logging untuk validasi Eye Heatmap | M | Fase 4 |

## 8. ⚠ Asumsi & Keputusan

| ID | Asumsi / Keputusan |
|---|---|
| A-01 🔺 | **Bahasa Rust** untuk engine dan riset (pilihan eksplisit user). Ini menyimpang dari Stack Pre-Approved RULES §10 (Node/Python). Library Rust ada di architecture §1 dan dianggap disetujui lewat keputusan ini. Dashboard tetap Vite+React (pre-approved) |
| A-02 🔺 | "Max DD harian 6x" ditafsirkan sebagai: **6 exit SL ATAU PnL realized neto ≤ −$6 dalam satu hari UTC** (00:00 UTC = 07:00 WIB), mana yang lebih dulu. −$6 = 10% dari $60 |
| A-03 🔺 | Cooldown 60 menit setelah SL bersifat **GLOBAL** (semua simbol & strategi), dihitung dari waktu fill SL. Alasan: alt kripto berkorelasi tinggi dan max posisi hanya 2. Ini berbeda dari riset apex (60 menit antar fill entry). Alternatif per-simbol dicatat untuk diuji sebagai trial |
| A-04 🔺 | Risk $1 = **kerugian neto terencana** termasuk fee masuk, fee keluar, dan slippage SL 0,02%. Gap/funding dapat membuat rugi realized sedikit > $1 |
| A-05 | Margin **tidak tetap**: margin = notional/25 = $0,97–$1,89 per posisi. Tidak ada sizing acak |
| A-06 | Modal awal **$60 USDT**, tanpa compounding; equity floor $40 → DISARM |
| A-07 🔺 | Holding time diperkirakan **berjam-jam** (median gerak ±2% = 3–17 jam), bukan menit. "Scalping" di sini = entry presisi di 5m/15m dengan SL struktural 2–4% |
| A-08 | Position mode one-way, 1 posisi per simbol, marginType CROSSED, leverage 25 per simbol |
| A-09 | SL/TP memakai `POST /fapi/v1/algoOrder` (CONDITIONAL, closePosition, MARK_PRICE) sesuai perubahan Binance 2025-12-09 |
| A-10 | RR tetap 2,0; varian BE/time-stop diuji sebagai trial terpisah |
| A-11 | Fee VIP0: maker 0,02%, taker 0,05%; diskon BNB tidak diasumsikan |
| A-12 🔺 | Holdout final = **2026-05-01 s/d 2026-10-31 (UTC)**, dikunci dan dijalankan sekali. Data BTC/ETH 2025–2026 dianggap terkontaminasi oleh ≈40 versi pencarian apex; N trial historis = 40 dimasukkan ke Deflated Sharpe |
| A-13 | Testnet ≥ 14 hari dan ≥ 30 trade sebelum mainnet. Mainnet diawali shadow mode (`arm=false`, sinyal saja) ≥ 7 hari |
| A-14 | Universe awal: ETH, SOL, XRP, DOGE, HYPE, SUI, LINK, ENA, ZEC, NEAR, 1000PEPE, WLD (snapshot 2026-11-04T18:58Z); BTC hanya konteks |
| A-15 | Semua waktu internal UTC (ms); tampilan dashboard WIB (UTC+7) |
| A-16 | Engine berjalan di 1 VPS/PC Linux milik user; dashboard hanya lokal |
| A-17 | 28 skrip TradingView adalah sumber **ide**; port Rust adalah implementasi sendiri, bukan salinan. Skrip terkunci SyWHPxSr hanya dari deskripsi publik |
| A-18 | Engine tidak pernah membaca/mencetak isi `.env` ke log; agen AI tidak membuka `.env` |
| A-19 | Strategi dengan < 100 trade IS = INCONCLUSIVE, tidak boleh live |
| A-20 | Base URL testnet USDⓈ-M = `https://demo-fapi.binance.com` (Binance Demo Trading). Diverifikasi ulang di T-3.1 karena URL testnet pernah berubah |
| A-21 | BLACKLIST.md, VOLUME_GATE.md, MARKET_READER.md (dokumen paralel di folder ini) adalah SSoT untuk allowlist kripto, gate likuiditas, dan regime global. FR-008 (screener) memakai output ketiganya sesuai SCREENER.md §9. Universe kerja = 31 simbol VOLUME_GATE.md §4 |
| A-22 🔺 | Sumber primer BTC.D, ETH.D, USDT.D, OTHERS.D, TOTAL, TOTAL2, TOTAL3 = TradingView CRYPTOCAP (scanner + chart WS anonim, tidak resmi). Cadangan BTCDOMUSDT/ALLUSDT Binance (return saja). Keduanya gagal → NO_TRADE [→ MARKET_READER.md §2.1–2.3] |
| A-23 🔺 | Histori TV anonim intraday terbatas (1h sejak 2026-09-01; TOTAL 1h sejak 2026-01-01; 1D sejak 2014). Backtest IS 2025 memakai cadangan Binance; data TV direkam sendiri mulai 2026-11-05 |
| A-24 🔺 | User menerima risiko syarat penggunaan TradingView (pemakaian pribadi, ≤ 1 request scanner/60 s) |
