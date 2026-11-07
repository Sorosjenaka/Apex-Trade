#!/usr/bin/env bash
# Wrapper Bash untuk menjalankan apex demo-smoke di Linux.
# Membutuhkan target demo di config/engine.toml (mode="testnet", arm=true)
# dan kredensial API Demo di environment BINANCE_API_KEY / BINANCE_API_SECRET.

set -euo pipefail

CONFIG_DIR="${1:-config}"

echo "=== Menjalankan Apex Demo Smoke Test ==="
echo "Pastikan config/engine.toml telah disetel ke mode='testnet' dan arm=true."

if [[ -z "${BINANCE_API_KEY:-}" || -z "${BINANCE_API_SECRET:-}" ]]; then
    echo "ERROR: Variabel environment BINANCE_API_KEY dan BINANCE_API_SECRET wajib disetel sebelum demo smoke." >&2
    exit 1
fi

APEX_BIN="target/debug/apex"
if [[ ! -x "$APEX_BIN" ]]; then
    APEX_BIN="target/release/apex"
fi
if [[ ! -x "$APEX_BIN" ]]; then
    echo "Mengompilasi binary apex..."
    cargo build -p apex
    APEX_BIN="target/debug/apex"
fi

exec "$APEX_BIN" --config "$CONFIG_DIR" demo-smoke
