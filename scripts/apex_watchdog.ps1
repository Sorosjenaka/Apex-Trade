# Apex watchdog: menjaga `apex.exe run --mode paper` dan dashboard (port 8787) tetap hidup.
# - Tidak mengubah config/engine.toml (arm tetap false) dan tidak mengirim order apa pun.
# - Restart otomatis kalau proses mati atau run.log tidak bergerak > StaleMin menit.
# - Mencegah sistem sleep selama watchdog berjalan (SetThreadExecutionState).
# - Satu instance saja (named mutex). Log: logs\watchdog.log
param([int]$StaleMin = 12, [int]$LoopSec = 15)

$ErrorActionPreference = "Continue"
$root = Split-Path -Parent $PSScriptRoot
Set-Location $root
New-Item -ItemType Directory -Force -Path "$root\logs" | Out-Null
$logf = "$root\logs\watchdog.log"
function Log($m) {
    $line = "{0:yyyy-MM-ddTHH:mm:ssZ} {1}" -f (Get-Date).ToUniversalTime(), $m
    Add-Content -Path $logf -Value $line -Encoding UTF8
    if ((Get-Item $logf).Length -gt 2MB) { Move-Item $logf "$logf.1" -Force }
}

$created = $false
$mutex = New-Object System.Threading.Mutex($true, "Global\ApexWatchdog", [ref]$created)
if (-not $created) { Write-Output "watchdog sudah berjalan"; exit 0 }

Add-Type -Namespace Apex -Name Pwr -MemberDefinition '[System.Runtime.InteropServices.DllImport("kernel32.dll")] public static extern uint SetThreadExecutionState(uint f);'
[void][Apex.Pwr]::SetThreadExecutionState([uint32]"0x80000001")  # ES_CONTINUOUS | ES_SYSTEM_REQUIRED

$py = "C:\Python314\python.exe"
if (-not (Test-Path $py)) { $py = (Get-Command python -ErrorAction SilentlyContinue).Source }
$exe = "$root\target-msvc\release\apex.exe"
$restarts = New-Object System.Collections.Generic.List[datetime]
$holdUntil = [datetime]::MinValue

function Get-Engine {
    Get-CimInstance Win32_Process -Filter "Name='apex.exe'" -ErrorAction SilentlyContinue |
        Where-Object { $_.CommandLine -match "\brun\b.*--mode\s+paper" } | Select-Object -First 1
}
function Start-Engine {
    Start-Process -FilePath "cmd.exe" -WindowStyle Hidden -ArgumentList "/c `"cd /d $root && $exe run --mode paper >> state\paper\run.log 2>> state\paper\run.err.log`""
}
function Dash-Up {
    $conns = Get-NetTCPConnection -LocalPort 8787 -State Listen -ErrorAction SilentlyContinue
    if (-not $conns) { return $false }
    foreach ($c in $conns) {
        $proc = Get-CimInstance Win32_Process -Filter "ProcessId = $($c.OwningProcess)" -ErrorAction SilentlyContinue
        if ($proc -and $proc.CommandLine -notmatch "dashboard[\\/]server\.py") {
            Log "WARN: proses asing ($($proc.Name) PID=$($proc.ProcessId)) membajak port 8787 -> matikan"
            Stop-Process -Id $c.OwningProcess -Force -ErrorAction SilentlyContinue
            return $false
        }
    }
    return $true
}
function Start-Dash {
    Start-Process -FilePath $py -WindowStyle Hidden -WorkingDirectory $root -ArgumentList "dashboard\server.py" `
        -RedirectStandardOutput "$root\dashboard\dash_out.log" -RedirectStandardError "$root\dashboard\dash_err.log"
}
function Can-Restart {
    $now = Get-Date
    $restarts.RemoveAll([Predicate[datetime]]{ param($t) ($now - $t).TotalMinutes -gt 10 }) | Out-Null
    if ($now -lt $holdUntil) { return $false }
    if ($restarts.Count -ge 5) {
        $script:holdUntil = $now.AddMinutes(5)
        Log "WARN: >=5 restart dalam 10 menit, tahan 5 menit"
        return $false
    }
    $restarts.Add($now); return $true
}

Log "watchdog start (root=$root stale=${StaleMin}m)"
while ($true) {
    try {
        $eng = Get-Engine
        if (-not $eng) {
            if (Can-Restart) { Log "engine tidak berjalan -> start paper"; Start-Engine; Start-Sleep -Seconds 10 }
        } else {
            $age = ((Get-Date) - (Get-Item "$root\state\paper\run.log").LastWriteTime).TotalMinutes
            $up  = ((Get-Date) - $eng.CreationDate).TotalMinutes
            if ($age -gt $StaleMin -and $up -gt $StaleMin) {
                if (Can-Restart) {
                    Log ("engine stale: run.log {0:N1} menit tidak bergerak -> restart" -f $age)
                    Stop-Process -Id $eng.ProcessId -Force -ErrorAction SilentlyContinue
                    Start-Sleep -Seconds 3; Start-Engine; Start-Sleep -Seconds 10
                }
            }
        }
        if (-not (Dash-Up)) {
            if (Can-Restart) { Log "dashboard :8787 mati -> start"; Start-Dash; Start-Sleep -Seconds 5 }
        }
    } catch { Log "ERR: $($_.Exception.Message)" }
    Start-Sleep -Seconds $LoopSec
}
