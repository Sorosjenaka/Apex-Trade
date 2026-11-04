# Apex Scalper RS

Bot scalping **Binance USDⓈ-M Futures** (crypto saja) dalam Rust: Market Reader → Screener + Volume Gate → 5 strategi → Risk Engine → OMS (paper / testnet / live).

> ⚠️ **Default AMAN:** `mode = "paper"` dan `arm = false` di `config/engine.toml`.
> Paper = simulasi lokal (MockExchange) dengan harga publik — **tidak ada order ke Binance**.
> Semua strategi berstatus **NO-GO** (belum lolos gate validasi). Testnet/live hanya mengirim order untuk strategi berstatus `GO`.

## Golden Rules (dikunci di kode; config yang beda = engine menolak start)
| Aturan | Nilai |
|---|---|
| Modal / risk per trade | $60 / $1 (risk aktual setelah pembulatan stepSize harus 0,80–1,00, kalau tidak → skip) |
| Stop loss | 2–4 % dari entry, di luar itu → skip |
| Leverage / margin | 25× CROSS, one-way, margin variabel (qty dari risk, bukan margin tetap) |
| Posisi maks | 2 (pending entry ikut dihitung) |
| Stop harian | 6 SL **atau** −$6 realized → stop sampai 00:00 UTC (07:00 WIB) |
| Cooldown | 1 jam setelah SL |
| Sideways | tidak trading (Market Reader `CHOP_NO_TRADE`) |
| Universe | allowlist crypto default-deny (TradFi/saham/pre-market ditolak) + volume gate; BTC hanya konteks |

## Struktur
```
crates/core           tipe, config, sizing, rounding filter, risk gate, audit log
crates/indicators     EMA/ATR/ADX/ER/CHOP/SRV dll (closed-bar)
crates/data           REST publik (klines, funding, exchangeInfo, ticker), WS publik, CSV, TradingView scanner
crates/market_reader  regime global (RISK_ON_ALTS / BTC_LED / RISK_OFF / CHOP_NO_TRADE / SHOCK_HALT)
crates/screener       regime per simbol, allowlist, volume gate V-01..V-10
crates/strategies     S1 TFP, S2 LSR, S3 SBR, S4 WSL, S5 RMP
crates/backtest       engine event-driven tanpa look-ahead, statistik, Monte Carlo, PSR/DSR
crates/oms            Order Management System Binance (+ MockExchange)
apps/apex             CLI `apex`
config/               engine.toml, risk.toml, screener.toml, strategies.toml, blacklist.json, filters_snapshot.json
docs/plan/            dokumen rencana (PRD, Rules, STRATEGIES, …)
```

## Setup Windows
1. Install Rust (MSVC) dari https://rustup.rs — butuh Rust ≥ 1.85.
2. Buka PowerShell di folder ini:
   ```powershell
   cargo build --release
   cargo test --workspace
   ```
   TLS memakai **rustls** (tanpa OpenSSL), jadi tidak perlu install apa-apa lagi.
3. Binary: `target\release\apex.exe`.

## Perintah
```powershell
# data publik (tanpa API key)
.\target\release\apex.exe fetch --from 2025-01-01 --to 2026-05-01 --data data
.\target\release\apex.exe reader                 # regime global sekarang (+ BTC.D/TOTAL dari TradingView bila tersedia)
.\target\release\apex.exe screen --top 15        # allowlist + volume gate + regime per simbol
.\target\release\apex.exe record --data data     # rekam bar tertutup dari WebSocket publik

# backtest / validasi (holdout 2026-05-01..2026-10-31 dikunci)
.\target\release\apex.exe backtest --data data --per-strategy
.\target\release\apex.exe backtest --data data --strategies S1 --stress
.\target\release\apex.exe validate --data data --strategies S1,S5

# engine
.\target\release\apex.exe run --mode paper            # simulasi, tanpa order
.\target\release\apex.exe run --mode paper --once     # satu siklus 15m
.\target\release\apex.exe status
.\target\release\apex.exe kill --reason "manual"      # blok entry baru
.\target\release\apex.exe kill --flatten              # engine menutup semua posisi
.\target\release\apex.exe kill --release
```
Hasil backtest → `reports\bt_*.json` + `*.trades.csv`; setiap run dicatat di `reports\trials_ledger.jsonl` (untuk DSR).

## Testnet dulu, live belakangan
1. Buat API key di **Binance Futures Demo/Testnet** (`demo-fapi.binance.com`).
2. Set key **hanya di environment proses** (engine tidak membaca file `.env`):
   ```powershell
   $env:BINANCE_API_KEY="..."; $env:BINANCE_API_SECRET="..."
   ```
3. Edit `config/engine.toml`: `arm = true` (**Anda sendiri** yang mengubah ini), lalu `apex run --mode testnet`.
4. Strategi hanya diperdagangkan bila status `GO` di `config/strategies.toml`. Ubah status hanya setelah strategi lolos `validate` + holdout + review Anda.
5. **Live** butuh ketiganya: `arm = true`, `--mode live`, dan `$env:APEX_LIVE_CONFIRM="I_UNDERSTAND_LIVE_ORDERS"`. Base URL juga dicek: mode testnet menolak host live, mode live menolak host non-live.

## OMS (Order Management)
- Signed REST HMAC-SHA256, `recvWindow`, sinkron jam server (retry sekali saat -1021), throttle weight + backoff 429/418.
- Saat start: one-way mode, margin CROSSED, leverage 25× per simbol.
- Entry LIMIT GTX (post-only) atau MARKET, `newClientOrderId` unik `apx…`; state disimpan **sebelum** order dikirim.
- Proteksi SL/TP lewat **Algo Order** (`/fapi/v1/algoOrder`, CONDITIONAL, `closePosition=true`, `workingType=MARK_PRICE`, `priceProtect`).
- ACK ≠ FILLED: user data stream (ORDER_TRADE_UPDATE / ALGO_UPDATE / ACCOUNT_UPDATE) adalah sumber kebenaran; hasil tidak pasti (timeout/5xx) → query by clientOrderId.
- SL gagal dipasang → retry → flatten; flatten gagal → status `Unprotected` + kill switch, terus dicoba. Close gagal **tidak** menghapus posisi dari buku.
- Rekonsiliasi saat boot & tiap 5 menit: posisi yatim → pasang SL (atau flatten sesuai `orphan_policy`).
- Kill switch berbasis file (`state/KILL`), audit log JSONL (`state/<mode>/audit.jsonl`), state `state/<mode>/oms_state.json`.
- `MockExchange` mengimplementasikan trait yang sama → dipakai untuk paper & 16 tes state machine.

## Varian S5 kNN

Varian K bersifat opsional dan default `s5_knn_enabled = false`. Bila diaktifkan dalam `[params]`, S5 memerlukan 15 tetangga dengan label yang sudah tutup sebelum waktu sinyal; kurang data berarti skip. Backtest melatih awal lewat replay S5 pada data sebelum awal periode yang diuji, lalu menambah hanya trade OOS yang sudah tutup. Trade penutupan paksa di akhir data tidak menjadi label. Replay pelatihan ditolak bila rentangnya beririsan dengan holdout terkunci. Latih awal memakai biaya dasar juga pada uji stres; label trade OOS yang kemudian masuk tetap memakai biaya skenario tersebut, sehingga urutan sinyal dapat berbeda antar skenario. Engine memakai riwayat trade S5 yang tertutup dan memiliki fitur tersimpan dalam state OMS; riwayat lama tanpa fitur dilewati. Bila varian K diaktifkan saat riwayat belum cukup, semua sinyal S5 akan di-skip; riwayat dapat dikumpulkan di paper dengan varian K nonaktif sebelum penilaian. Versi ini belum membuktikan parity terhadap Pine atau keuntungan strategi.

## Batasan yang perlu Anda tahu
- Strategi adalah **proxy** dari Pine (lihat CHANGELOG); belum ada yang lolos gate → semua NO-GO.
- Regime backtest memakai proxy BTCDOMUSDT (bukan BTC.D TradingView historis); filter exchange = snapshot sekarang, bukan historis; volume gate di backtest hanya V-01/V-02 proxy.
- OMS **belum pernah diuji ke exchange sungguhan** (hanya MockExchange). Jalankan testnet lama dulu.
- Tidak ada SQLite / dashboard web (state = JSON, log = JSONL).
