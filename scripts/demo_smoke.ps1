# Wrapper PowerShell untuk menjalankan apex demo-smoke di Windows.
# Membutuhkan target demo di config/engine.toml (mode="testnet", arm=true)
# dan kredensial API Demo di environment BINANCE_API_KEY / BINANCE_API_SECRET.

param (
    [string]$ConfigDir = "config"
)

$ErrorActionPreference = "Stop"

Write-Host "=== Menjalankan Apex Demo Smoke Test ===" -ForegroundColor Cyan
Write-Host "Pastikan config/engine.toml telah disetel ke mode='testnet' dan arm=true." -ForegroundColor Yellow

if (-not $env:BINANCE_API_KEY -or -not $env:BINANCE_API_SECRET) {
    Write-Error "Variabel environment BINANCE_API_KEY dan BINANCE_API_SECRET wajib disetel sebelum demo smoke."
    exit 1
}

$ApexBin = "target-msvc\debug\apex.exe"
if (-not (Test-Path $ApexBin)) {
    $ApexBin = "target-msvc\release\apex.exe"
}
if (-not (Test-Path $ApexBin)) {
    Write-Host "Mengompilasi binary apex..." -ForegroundColor Gray
    .\cargo-msvc.cmd build -p apex
    $ApexBin = "target-msvc\debug\apex.exe"
}

& $ApexBin --config $ConfigDir demo-smoke
exit $LASTEXITCODE
