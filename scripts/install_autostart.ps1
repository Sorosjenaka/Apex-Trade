# Pasang/lepas auto-start watchdog saat login Windows (folder Startup user, tanpa admin).
#   .\scripts\install_autostart.ps1            -> pasang
#   .\scripts\install_autostart.ps1 -Remove    -> lepas
param([switch]$Remove)
$startup = [Environment]::GetFolderPath("Startup")
$vbs = Join-Path $startup "ApexWatchdog.vbs"
if ($Remove) { Remove-Item $vbs -Force -ErrorAction SilentlyContinue; Write-Output "auto-start dilepas"; exit 0 }
$ps1 = Join-Path $PSScriptRoot "apex_watchdog.ps1"
$cmd = "powershell.exe -NoProfile -ExecutionPolicy Bypass -WindowStyle Hidden -File """"$ps1"""""
Set-Content -Path $vbs -Encoding ASCII -Value "CreateObject(""WScript.Shell"").Run ""$cmd"", 0, False"
Write-Output "auto-start terpasang: $vbs"
