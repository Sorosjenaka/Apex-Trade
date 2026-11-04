# Apex Scalper RS

Apex Scalper RS adalah mesin trading otomatis berbasis Rust untuk Binance USDⓈ-M Futures, ditambah pipeline riset dan validasi strategi. Aturan risikonya ditegakkan di kode dan di exchange: risk tetap $1 per trade dengan margin variabel, SL 2–4%, 25x CROSS, maksimal 2 posisi, stop harian, dan cooldown 60 menit. Engine selalu start dalam keadaan **DISARMED** dan wajib diuji di testnet sebelum mainnet.

> Status: **blueprint** (v0.1.0). Belum ada kode. Semua strategi berstatus HIPOTESIS.

## Prasyarat
- Linux x86_64 (Ubuntu 22.04+), 4 vCPU, RAM ≥ 4 GB, disk ≥ 40 GB (data riset).
- Rust 1.87 (`rustup toolchain install 1.87`).
- Node.js 20 LTS + pnpm 9 (dashboard).
- SQLite 3.40+.
- API key **Binance Demo Trading** (https://demo.binance.com) untuk testnet. Key mainnet hanya izin Futures, **tanpa withdraw**, whitelist IP.

## Konfigurasi
```bash
cp .env.example .env      # lalu isi di server; JANGAN commit, JANGAN tampilkan isinya
```
Isi `.env.example` (placeholder):
```
APEX_MODE=testnet                  # testnet | mainnet
BINANCE_API_KEY=__ISI_DI_SERVER__
BINANCE_API_SECRET=__ISI_DI_SERVER__
APEX_LOCAL_TOKEN=__32_BYTE_HEX__
APEX_HTTP_ADDR=127.0.0.1:8787
DATABASE_URL=sqlite://data/apex.sqlite
RUST_LOG=info
```

## Install / run / test
```bash
# build & test
cargo build --workspace --release
cargo test --workspace

# engine (DISARMED saat start)
cargo run -p apex-engine --release
curl -s http://127.0.0.1:8787/api/health      # → {"status":"ok"}
curl -s http://127.0.0.1:8787/api/status

# riset
cargo run -p apex-research --release -- download --symbols ETHUSDT,SOLUSDT --from 2023-01-01 --to 2026-10-31
cargo run -p apex-research --release -- replay   --strategy S3 --params config/s3.is.toml --period IS
cargo run -p apex-research --release -- validate --strategy S3

# dashboard (baca-saja)
cd web && pnpm install && pnpm dev            # http://127.0.0.1:5173

# ARM manual (testnet)
curl -s -X POST http://127.0.0.1:8787/api/arm  -H "X-Apex-Token: $TOKEN" -d '{"phrase":"ARM testnet"}'
curl -s -X POST http://127.0.0.1:8787/api/kill -H "X-Apex-Token: $TOKEN" -d '{}'
```

## Struktur folder singkat
```
crates/   core data features screener strategies risk exec backtest validate
apps/     engine (daemon + API)  research-cli
web/      dashboard Vite+React
config/   engine.toml risk.toml screener.toml strategies.toml
plan/     dokumen blueprint (PRD, Rules, STRATEGIES, …)
```

## Troubleshooting
| Gejala | Penyebab | Solusi |
|---|---|---|
| `Address already in use 127.0.0.1:8787` | port bentrok | ubah `APEX_HTTP_ADDR` atau hentikan proses lama (`ss -ltnp \| grep 8787`) |
| Engine berhenti: `missing env BINANCE_API_KEY` | `.env` belum ada di server | `cp .env.example .env`, isi di server |
| `-1021 Timestamp outside recvWindow` | jam server melenceng | aktifkan NTP (`timedatectl set-ntp true`) |
| `-4120 STOP_ORDER_SWITCH_ALGO` | SL dikirim ke `/fapi/v1/order` | bug, wajib lewat `/fapi/v1/algoOrder` |
| ARM ditolak `ARM_PRECONDITION` | leverage/margin type/stop harian/equity | lihat `account_check` di `/api/status` |
| `risk.toml mismatch` saat start | angka config ≠ Rules §1 | samakan dengan Rules.md §1 |
| Testnet 401 | memakai key mainnet di testnet | buat key di demo.binance.com |
