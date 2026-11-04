# Arsitektur: Apex Scalper RS

> v1.0 · 5 Nov 2026 (WIB). Requirement: [→ PRD §5, §6]. Bentuk data: [→ Schema §2]. Rumus: [→ Rules §4].

## 1. Tech Stack

| Layer | Teknologi | Alasan |
|---|---|---|
| Bahasa engine & riset | Rust stable ≥ 1.87 (`rust-toolchain.toml`) | A-01; NFR-01, NFR-10, NFR-17 (deterministik, tanpa GC); port langsung `scalp_portfolio.rs` |
| Runtime async | `tokio` 1.x (multi-thread) | NFR-01, NFR-07 |
| HTTP API lokal | `axum` 0.7 + `tower-http` (CORS hanya `http://127.0.0.1:5173`) | FR-001, FR-026; NFR-03, NFR-05 |
| HTTP klien Binance | `reqwest` (rustls) | FR-003, FR-017, FR-018; NFR-19 |
| WebSocket | `tokio-tungstenite` (rustls) | FR-006, FR-020; NFR-07 |
| Signing | `hmac` + `sha2` (HMAC-SHA256), `hex` | FR-017 |
| Angka dana | `rust_decimal` untuk harga/qty/USDT; `f64` hanya untuk fitur/indikator | Schema §5 |
| Serialisasi | `serde`, `serde_json`, `toml` | Schema §2 |
| Database | SQLite via `sqlx` (fitur `sqlite`, `runtime-tokio`), mode WAL | FR-025, FR-032; Stack default RULES §10 |
| Data makro (BTC.D, TOTAL1–3, USDT.D) | TradingView CRYPTOCAP: scanner REST (poll 60 s) + chart WS anonim (backfill 6 jam) via `reqwest`/`tokio-tungstenite`; cadangan Binance BTCDOMUSDT/ALLUSDT; tabel SQLite `tv_snapshot`, `tv_bar` | MARKET_READER.md §2.1–2.5 (TV-01…TV-07, DC-01…DC-04) |
| Data riset | file CSV/ZIP data.binance.vision → Parquet via `arrow`/`parquet` (opsional) | FR-029; NFR-12 |
| Paralel riset | `rayon` | NFR-12 |
| Secret di memori | `secrecy` (`SecretString`) | NFR-04 |
| Log & metrik | `tracing`, `tracing-subscriber` (JSON), filter redaksi secret | FR-025; NFR-04 |
| Config & env | `dotenvy` + `config/*.toml` | Stack default (dotenv + .env.example) |
| Test | `cargo test`, `proptest` (sizing), `wiremock` (mock Binance REST), `insta` (snapshot laporan) | Rules §5 |
| Dashboard | Vite + React + TypeScript (baca-saja) | FR-033; pre-approved |
| Statistik validasi | implementasi sendiri di `crates/validate` (bootstrap, PSR/DSR, walk-forward) | FR-031 |

## 2. Diagram Alur Sistem

```
                  data.binance.vision (zip+CHECKSUM)                 Binance USDⓈ-M (mainnet / demo-fapi)
                              │                                       │ REST (signed)        │ WS market / user
                              ▼                                       ▼                      ▼
┌──────────────────── apex-research (CLI) ─────────────┐   ┌────────────────── apex-engine (daemon) ──────────────────┐
│ download → features → strategies → replay → validate │   │ MarketData ─► BarStore ─► Screener ─► Strategies          │
│      │                                     │         │   │                    │             │            │           │
│      └────────── trials_ledger ◄───────────┘         │   │                    ▼             ▼            ▼           │
│              reports/validation/*.json               │   │              RiskGate (GR-1..GR-7) ─► Sizer ─► OrderManager│
└──────────────────────────┬───────────────────────────┘   │                                          │  ▲          │
                           │ config/strategies.toml        │   UserStream (ORDER/ALGO/ACCOUNT) ───────┘  │          │
                           │ (status GO saja)              │   Reconciler (60 dtk) ─────────────────────┘          │
                           └──────────────────────────────►│   ArmControl ◄── axum API 127.0.0.1:8787 ◄── Dashboard  │
                                                           │   SQLite (WAL): signals, orders, positions, audit     │
                                                           └───────────────────────────────────────────────────────┘
```

Crate `features`, `screener`, `strategies`, `risk` dipakai **bersama** oleh riset dan engine. Kode sinyal yang dibacktest = kode sinyal live (NFR-18).

## 3. Struktur Folder

```
apex-scalper-rs/
├── Cargo.toml                 # workspace
├── rust-toolchain.toml        # 1.87
├── .env.example
├── .gitignore
├── README.md
├── config/
│   ├── engine.toml            # universe, mode, port, interval
│   ├── risk.toml              # angka GR (dibaca, divalidasi = Rules §1)
│   ├── screener.toml          # ambang SCREENER.md
│   ├── strategies.toml        # status GO/NO-GO per strategi + parameter terkunci
│   └── blocklist.toml
├── crates/
│   ├── core/                  # tipe: Bar, Side, Symbol, Filters, Decimal helpers, clock UTC
│   ├── data/                  # rest.rs, ws.rs, vision.rs (downloader), csv.rs, bar_store.rs
│   ├── features/              # basic.rs (port indicators.rs), pivots.rs, fib.rs, regime.rs (ER, CHOP, SRV, BFT, Decycler), delta.rs, profile.rs
│   ├── screener/              # state machine SCREENER.md
│   ├── strategies/            # s1_tfp.rs, s2_lsr.rs, s3_sbr.rs, s4_wsl.rs, s5_rmp.rs, mod.rs (trait Strategy)
│   ├── risk/                  # sizing.rs, gates.rs (max pos, cooldown, daily), guards.rs (equity floor)
│   ├── exec/                  # binance/{rest,sign,algo,userstream}.rs, order_sm.rs, reconcile.rs, arm.rs
│   ├── backtest/              # portfolio.rs (port scalp_portfolio.rs), cost.rs, tests/
│   └── validate/              # gates.rs (port validation.rs), walkforward.rs, montecarlo.rs, dsr.rs, plateau.rs
├── apps/
│   ├── engine/                # main.rs (tokio + axum), api/*.rs
│   └── research-cli/          # main.rs: download | replay | validate | screen | ledger
├── web/                       # Vite + React + TS dashboard (baca-saja)
├── research/oracle/           # salinan referensi apex tv7 Python (tidak dikompilasi)
├── data/                      # (gitignored) raw/, parquet/, apex.sqlite
├── reports/                   # (gitignored kecuali summary) validation/*.json
└── tests/e2e/                 # mock exchange (wiremock) + skenario testnet
```

## 4. Alur Data Kritis

**A. Sinyal → posisi terlindungi (FR-006 → FR-019)**
1. WS kline mengirim `x=true` untuk bar 15m simbol S → `BarStore.append` (FR-006). Cek celah (FR-007).
2. `Screener` membaca state S terbaru (snapshot 1h, umur ≤ 65 menit) (FR-008).
3. `Strategies::on_bar_close` → `Signal{side, entry, sl, tp, strategy_id}` → disimpan `signals` (FR-010).
4. `RiskGate`: urutan cek tetap → ARM (FR-022) → regime (FR-009) → SL range (FR-012) → max posisi (FR-014) → cooldown (FR-015) → stop harian (FR-016) → equity floor (FR-024) → spread live → `Sizer` (FR-011, FR-013). Gagal pertama → `SKIP_<alasan>`, berhenti.
5. `OrderManager` mengirim LIMIT GTX `newClientOrderId = "as-{strategy}-{sym}-{bar_close_ms}"`, `newOrderRespType=RESULT` (FR-017). Bila ditolak (-5022 GTX akan menjadi taker) → sinyal `REJECTED_GTX`, tanpa retry ke harga lain.
6. `UserStream` menerima `ORDER_TRADE_UPDATE` FILLED/PARTIALLY_FILLED → `OrderManager` mengirim 2 algo order (FR-018):
   - `POST /fapi/v1/algoOrder`, `algoType=CONDITIONAL`, `type=STOP_MARKET`, `side=lawan`, `triggerPrice=SL` (dibulatkan tickSize ke arah aman), `closePosition=true`, `workingType=MARK_PRICE`, `priceProtect=TRUE`.
   - Sama untuk `TAKE_PROFIT_MARKET` pada TP.
7. Konfirmasi lewat respons + `ALGO_UPDATE` / `GET /fapi/v1/openAlgoOrders`. Bila belum ada dalam 5 dtk (3 percobaan) → MARKET reduceOnly → `arm=false` → `SL_MISSING` (FR-019).
8. Partial fill: SL/TP `closePosition=true` otomatis melindungi seluruh posisi berjalan. Sisa entry pending tetap ikut TTL 60 menit.
9. Exit: `ALGO_UPDATE` triggered + `ORDER_TRADE_UPDATE` reduce fill → posisi tutup → algo lawan dibatalkan → `trades` ditulis. Bila exit = SL → `cooldown_until = fill_ts + 3.600.000 ms`, `daily_sl_count += 1` (FR-015, FR-016).

**B. Rekonsiliasi (FR-021)** — tiap 60 dtk dan saat start:
- `GET /fapi/v3/positionRisk`, `GET /fapi/v1/openOrders`, `GET /fapi/v1/openAlgoOrders`.
- Diff terhadap SQLite. Exchange = sumber kebenaran.
- Posisi tanpa SL → langkah A.6–A.7.
- Order lokal tanpa pasangan di exchange → ditandai `CLOSED_EXTERNAL`.

**C. Riset (FR-029 → FR-032)**
- `download` → verifikasi `.CHECKSUM` → Parquet.
- `replay --strategy S1 --params p.toml --period IS` → `trials_ledger += 1` → `validate` → `reports/validation/S1.json`.
- Status GO hanya ditulis manual oleh user ke `config/strategies.toml` setelah laporan PASS.

## 5. Komponen Utama & Tanggung Jawabnya

| Komponen | Tanggung jawab | FR |
|---|---|---|
| `apps/engine::api` | `/api/health`, `/api/status`, `/api/screener`, `/api/trades`, `/api/arm`, `/api/kill` | FR-001, FR-022, FR-023, FR-026–FR-028 |
| `exec::arm` | State ARM in-memory (start `false`), syarat ARM | FR-002, FR-022, FR-024 |
| `exec::binance::rest` | Signed REST, recvWindow 5.000 ms, sinkron waktu, bobot rate limit | FR-003, FR-004, FR-005, FR-034 |
| `data::ws` | Stream `<sym>@kline_5m/15m/1h`, `@bookTicker`, reconnect + backfill | FR-006, FR-007 |
| `screener` | State per simbol per jam | FR-008, FR-009 |
| `strategies` | S1–S5 (trait `Strategy`), hanya yang GO | FR-010 |
| `risk::sizing` / `risk::gates` | Rumus Rules §4.1, gate GR | FR-011–FR-016, FR-024 |
| `exec::order_sm` | State machine order: NEW→PARTIAL→FILLED→PROTECTED→CLOSED / EXPIRED / REJECTED | FR-017–FR-019 |
| `exec::userstream` | listenKey, keepalive 30 menit, parsing ORDER/ALGO/ACCOUNT | FR-020 |
| `exec::reconcile` | Diff & perbaikan | FR-021 |
| `core::audit` | Tulis `events_audit` | FR-025 |
| `data::vision`, `backtest`, `validate` | Riset | FR-029–FR-032 |
| `web/` | Dashboard baca-saja | FR-033 |

## 6. Keamanan — Implementasi

Target keamanan: [→ PRD §6 NFR-04, NFR-05, NFR-06].
- **Secret:** `BINANCE_API_KEY`, `BINANCE_API_SECRET`, dan `APEX_LOCAL_TOKEN` hanya dibaca dari environment proses `apex-engine` (dimuat `dotenvy` dari `.env` di server). Disimpan sebagai `secrecy::SecretString`; `Debug`/`Display` menampilkan `***`. Layer `tracing` meredaksi field bernama `*key*`, `*secret*`, `*token*`, `signature`.
- **Larangan tooling:** agen AI dan skrip tidak membaca atau mencetak `.env` (Rules §6). `.env` ada di `.gitignore`. Pre-commit `gitleaks`.
- **Frontend** tidak pernah memegang API key Binance. Dashboard hanya memanggil `127.0.0.1:8787`. Endpoint POST membutuhkan header `X-Apex-Token`, yang dimasukkan user manual di dashboard dan disimpan di `sessionStorage`, bukan localStorage.
- **Jaringan:** axum bind `127.0.0.1:8787`. CORS hanya origin `http://127.0.0.1:5173`. Tidak ada docker port publish ke `0.0.0.0` (REVIEW M9).
- **API key Binance:** izin Futures trading saja, **tanpa withdraw**, whitelist IP server. Key testnet (demo.binance.com) terpisah dari mainnet.
- **ARM:** default `false` di setiap start (tidak dipersist). Hanya `POST /api/arm` dengan frasa `ARM testnet` / `ARM mainnet` sesuai `APEX_MODE`.

## 7. Titik Kegagalan & Mitigasi

| Kegagalan | Dampak | Mitigasi | NFR-ID |
|---|---|---|---|
| SL algo gagal dibuat (error, -2021 trigger langsung, margin) | Posisi tanpa stop di CROSS → seluruh wallet jadi jaminan | Retry 3× dalam 5 dtk → MARKET reduceOnly → DISARM (FR-019) | NFR-02 |
| Engine crash/restart saat posisi terbuka | Posisi tetap punya SL/TP di exchange; state lokal basi | Reconcile saat start ≤ 10 dtk (FR-021) | NFR-09 |
| WS putus | Bar terlewat → sinyal salah | Reconnect ≤ 5 dtk + backfill REST; simbol BLOCKED sampai lengkap (FR-007) | NFR-07 |
| Jam lokal melenceng | -1021 timestamp, order ditolak | Sinkron `serverTime` tiap 60 dtk, blok bila > 500 ms (FR-005) | NFR-01 |
| Rate limit 429/418 | Ban IP | Budget bobot ≤ 50%, backoff eksponensial, cache exchangeInfo | NFR-19 |
| Gap harga melewati SL | Rugi > $1 | Diterima (A-04); dicatat `slippage_R`; stop harian memakai PnL realized | — |
| Perubahan API Binance (seperti algoOrder 2025-12-09) | Order ditolak | Test kontrak harian di testnet (T-3.13), alarm `REJECTED_*` ≥ 3 berturut → DISARM | NFR-08 |
| Memory leak | OOM, proses mati | Bounded buffer BarStore (≤ 2.000 bar/TF/simbol), soak 72 jam | NFR-10 |
| Bug strategi (look-ahead) | Hasil live ≠ backtest | Kode sama untuk riset & live; prefix test CI | NFR-18 |
