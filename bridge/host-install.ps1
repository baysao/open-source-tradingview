﻿# bridge/host-install.ps1: one-shot host setup for the MT5 bridge machine (DEBT-039's host).
#
# Run on the Windows box that runs MetaTrader 5 (the Exness terminal). Idempotent: re-running
# skips what is already installed. Every CLI dependency comes from the mise registry
# (python, cloudflared); nothing is installed from vendor installers.
#
#   powershell -ExecutionPolicy Bypass -File bridge\host-install.ps1
#   # or from CMD:  bridge\install.cmd
#
# What it does:
#   1. installs mise (irm https://mise.run/powershell | iex)
#   2. mise install python  (bridge/mt5_bridge.py is stdlib-only: no pip packages)
#   3. mise install cloudflared (the HTTPS front; a quick tunnel prints the public URL)
#   4. deploys this repo's bridge/ + docs/ into C:\Users\Administrator\project\vtradingview
#      (copy when run locally; git clone when the repo is public)
#   5. writes bridge\bridge.env (BRIDGE_TOKEN generated once, MT5_MCP_TOKEN asked once)
#   6. starts the bridge on 127.0.0.1:8797 and a cloudflared quick tunnel, saves the URLs
#
# The only value it cannot fetch: MT5's MCP token (Tools > Options > MCP on the terminal).
# Re-running regenerates nothing: bridge.env is kept if present.

$ErrorActionPreference = "Stop"

# --- layout
$RepoRoot = if (Test-Path "$PSScriptRoot\..\.git") { Split-Path $PSScriptRoot -Parent } else { $null }
$DeployRoot = "C:\Users\Administrator\project\vtradingview"
if (-not $RepoRoot) { $RepoRoot = $DeployRoot }
$BridgeDir = Join-Path $DeployRoot "bridge"
$EnvFile = Join-Path $BridgeDir "bridge.env"
$LogDir = Join-Path $DeployRoot "logs"
New-Item -ItemType Directory -Force -Path $BridgeDir, $LogDir | Out-Null

function Say($m) { Write-Host "[host-install] $m" }

# --- 1. mise (registry-backed tool manager)
$MiseShim = "$env:USERPROFILE\.local\bin\mise.exe"
if (Test-Path $MiseShim) {
    Say "mise already installed: $MiseShim"
} else {
    Say "installing mise (mise.run)"
    irm https://mise.run/powershell | iex
    if (-not (Test-Path $MiseShim)) { throw "mise install did not produce $MiseShim" }
}
$Mise = $MiseShim

# --- 2. python (the bridge is stdlib-only; mise registry: core:python)
Say "mise install python"
& $Mise install python 2>&1 | Select-Object -Last 2
$PythonExe = (& $Mise which python 2>$null)
if (-not $PythonExe) { throw "mise which python returned nothing" }
Say "python: $PythonExe (& $PythonExe --version)"

# --- 3. cloudflared (the HTTPS front; mise registry: aqua:cloudflare/cloudflared)
Say "mise install cloudflared"
& $Mise install cloudflared 2>&1 | Select-Object -Last 2
$TunnelExe = (& $Mise which cloudflared 2>$null)
if (-not $TunnelExe) { throw "mise which cloudflared returned nothing" }
Say "cloudflared: $TunnelExe"

# --- 4. deploy the bridge sources
$BridgeSrc = Join-Path $RepoRoot "bridge\mt5_bridge.py"
if (-not (Test-Path $BridgeSrc)) { throw "bridge source not found: $BridgeSrc" }
if ($RepoRoot -ne $DeployRoot) {
    Say "deploying bridge sources to $DeployRoot"
    Copy-Item $BridgeSrc (Join-Path $BridgeDir "mt5_bridge.py") -Force
    $DocsSrc = Join-Path $RepoRoot "docs\mt5-bridges.md"
    if (Test-Path $DocsSrc) {
        New-Item -ItemType Directory -Force -Path (Join-Path $DeployRoot "docs") | Out-Null
        Copy-Item $DocsSrc (Join-Path $DeployRoot "docs\mt5-bridges.md") -Force
    }
} else {
    Say "sources already in place: $BridgeDir\mt5_bridge.py"
}

# --- 5. bridge.env: BRIDGE_TOKEN generated once; MT5_MCP_TOKEN asked once and stored
if (Test-Path $EnvFile) {
    Say "bridge.env already present, kept: $EnvFile"
} else {
    Say "writing $EnvFile"
    $TokenBytes = New-Object byte[] 32
    [Security.Cryptography.RandomNumberGenerator]::Create().GetBytes($TokenBytes)
    $BridgeToken = [Convert]::ToBase64String($TokenBytes) -replace "\+", "_" -replace "/", "_" -replace "=", ""
    $McpToken = Read-Host "MT5 MCP token (MT5: Tools > Options > MCP)"
    if (-not $McpToken) { throw "MT5_MCP_TOKEN is required (Tools > Options > MCP on the terminal)" }
    @"
BRIDGE_TOKEN=$BridgeToken
MT5_MCP_TOKEN=$McpToken
BRIDGE_BIND=127.0.0.1
BRIDGE_PORT=8797
"@ | Set-Content -Encoding ascii $EnvFile
}

# read the env back (single source of truth for the steps below)
$BridgeEnv = @{}
Get-Content $EnvFile | ForEach-Object {
    if ($_ -match "^([A-Z0-9_]+)=(.*)$") { $BridgeEnv[$Matches[1]] = $Matches[2] }
}
if (-not $BridgeEnv["BRIDGE_TOKEN"]) { throw "BRIDGE_TOKEN missing from $EnvFile" }
if (-not $BridgeEnv["MT5_MCP_TOKEN"]) { throw "MT5_MCP_TOKEN missing from $EnvFile" }

# --- 6. start the bridge + tunnel, verify /health, save the public URL
function Stop-Old($name) {
    Get-Process -Name $name -ErrorAction SilentlyContinue | Where-Object {
        $_.Path -and ($_.CommandLine -match "vtradingview" -or $_.Name -eq "mt5_bridge")
    } | Stop-Process -Force -ErrorAction SilentlyContinue
}

Say "starting the bridge on 127.0.0.1:$($BridgeEnv['BRIDGE_PORT'])"
$BridgeProc = Start-Process -FilePath $PythonExe -ArgumentList "`"$BridgeDir\mt5_bridge.py`"" `
    -WorkingDirectory $BridgeDir -WindowStyle Hidden -PassThru `
    -RedirectStandardOutput (Join-Path $LogDir "bridge.log") `
    -RedirectStandardError (Join-Path $LogDir "bridge.err")
$BridgeProc.Id | Set-Content (Join-Path $LogDir "bridge.pid")

# wait for /health (max 15 s; the terminal must be running with MCP enabled)
$Health = $null
foreach ($i in 1..15) {
    Start-Sleep -Seconds 1
    try {
        $r = Invoke-WebRequest -Uri "http://127.0.0.1:$($BridgeEnv['BRIDGE_PORT'])/health" `
            -Headers @{ Authorization = "Bearer $($BridgeEnv['BRIDGE_TOKEN'])" } `
            -UseBasicParsing -TimeoutSec 4
        $Health = $r.Content
        break
    } catch { }
}
if (-not $Health) {
    Say "the bridge did not answer /health; last log lines:"
    Get-Content (Join-Path $LogDir "bridge.log"), (Join-Path $LogDir "bridge.err") -Tail 10 -ErrorAction SilentlyContinue
    throw "bridge /health failed — is the MT5 terminal running with MCP enabled (Tools > Options > MCP)?"
}
Say "bridge /health OK: $Health"

Say "starting the cloudflared quick tunnel"
$TunnelLog = Join-Path $LogDir "tunnel.log"
$TunnelProc = Start-Process -FilePath $TunnelExe -ArgumentList "tunnel", "--url", "http://127.0.0.1:$($BridgeEnv['BRIDGE_PORT'])", "--no-autoupdate" `
    -WindowStyle Hidden -PassThru -RedirectStandardOutput $TunnelLog -RedirectStandardError (Join-Path $LogDir "tunnel.err")
$TunnelProc.Id | Set-Content (Join-Path $LogDir "tunnel.pid")

# the quick-tunnel URL prints on stderr after a few seconds
$PublicUrl = $null
foreach ($i in 1..20) {
    Start-Sleep -Seconds 1
    $t = (Get-Content $TunnelLog, (Join-Path $LogDir "tunnel.err") -ErrorAction SilentlyContinue) -join "`n"
    if ($t -match "https://[a-z0-9-]+\.trycloudflare\.com") {
        $PublicUrl = $Matches[0]
        break
    }
}
if (-not $PublicUrl) {
    Say "tunnel logs so far:"
    Get-Content $TunnelLog, (Join-Path $LogDir "tunnel.err") -Tail 15 -ErrorAction SilentlyContinue
    throw "no trycloudflare URL in the tunnel log"
}

# end-to-end: the public URL must answer /health with the same body
$Public = $null
foreach ($i in 1..10) {
    Start-Sleep -Seconds 2
    try {
        $r = Invoke-WebRequest -Uri "$PublicUrl/health" `
            -Headers @{ Authorization = "Bearer $($BridgeEnv['BRIDGE_TOKEN'])" } `
            -UseBasicParsing -TimeoutSec 8
        $Public = $r.Content
        break
    } catch { }
}

Say "=============================================="
Say "BRIDGE_URL (register in the hub's Settings):"
Say "  $PublicUrl"
Say "BRIDGE_TOKEN (the Settings form asks for it):"
Say "  $($BridgeEnv['BRIDGE_TOKEN'])"
if ($Public) { Say "public /health OK: $Public" }
else { Say "WARNING: public /health not verified yet (tunnel may still be warming; retry from the hub)" }
Say "logs: $LogDir (bridge.log, tunnel.log)"
Say "=============================================="
