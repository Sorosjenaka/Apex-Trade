# Changelog

Format: [Keep a Changelog](https://keepachangelog.com/id-ID/1.1.0/). Versi: SemVer. Tanggal dalam WIB.

## [Unreleased]
### Changed
- MARKET_READER.md: TradingView CRYPTOCAP jadi sumber primer BTC.D/ETH.D/USDT.D/OTHERS.D/TOTAL/TOTAL2/TOTAL3 (S-02…S-05, S-11), Binance cadangan; tambah §2.1–2.5 (inventori histori, perbandingan BTC.D vs BTCDOMUSDT, TV-01…TV-07, DC-01…DC-04, RK-TV1…4); VAL-08 diperbarui, VAL-09…VAL-11 baru.
- PRD.md: A-22…A-24 🔺. architecture.md: baris tech stack data makro.
### Added
- `tv_reader.py` (prototipe scanner + histori chart WS), `rust_sketch` `tv_source` (pemilihan sumber + staleness + sanity), `data/tradingview/` CSV + inventori.
- SCREENER.md: ID komponen regime `RG-x` → `RC-x` (hindari bentrok dengan MARKET_READER.md); tambah §9 integrasi dengan BLACKLIST/VOLUME_GATE/MARKET_READER; universe kerja = 31 simbol VOLUME_GATE.md §4; cek leverage 25x dan sizing untuk 31 simbol.
- PRD.md: tambah A-21; NFR-11 kapasitas 30 → 40 simbol.

## [0.1.0] - 2026-11-05
### Added
- Blueprint 9 dokumen (PRD, architecture, Schema, Design, Rules, ROADMAP, AGENT, CHANGELOG, README) mengikuti RULES.md v1.2.
- STRATEGIES.md: 5 strategi hipotesis S1–S5 dari 28 skrip TradingView.
- SCREENER.md: penyaring trending/ekspansi 4 lapis.
- FEASIBILITY.md: kelayakan simbol, likuidasi CROSS, biaya, volatilitas. Data Binance publik diambil 2026-11-04T18:58Z (01:58 WIB 5 Nov).
- APEX_MAPPING.md: keputusan per file apex-trading-system.
- `pine/`: 27 source Pine + CATALOG.md (1 skrip terkunci: SyWHPxSr).
### Changed
- Modal awal $60 (bukan $40 seperti riset apex); margin variabel = notional/25 (bukan tetap).
- Cooldown: 60 menit global setelah SL (riset apex: 60 menit antar fill).
