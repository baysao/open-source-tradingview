# bridge/host-install.ps1: one-shot host setup for the MT5 bridge machine.
#
# Run on the Windows box that runs MetaTrader 5 (the Exness terminal). Idempotent: re-running
# skips what is already installed. Every CLI dependency comes from the mise registry
# (mt5-bridge, cloudflared); nothing is installed from vendor installers. The bridge itself is
# the Rust release binary (crates/mt5-bridge), fetched as a prebuilt asset — no Python, no
# build step.
#
#   powershell -ExecutionPolicy Bypass -File bridge\host-install.ps1
#   # or from CMD:  bridge\install.cmd
#
# What it does:
#   1. installs mise (irm https://mise.run/powershell | iex)
#   2. MISE_ENV=mt5 mise install mt5-bridge  (the release binary from mise.mt5.toml)
#   3. mise install cloudflared (the HTTPS front; a quick tunnel prints the public URL)
#   4. writes bridge\bridge.env (BRIDGE_TOKEN generated once, MT5_MCP_TOKEN asked once)
#   5. registers ONLOGON scheduled tasks that start the bridge (bare exe path) and the tunnel;
#      starts both now and saves the URLs
#
# The only value it cannot fetch: MT5's MCP token (Tools > Options > MCP on the terminal).
# Re-running regenerates nothing: bridge.env is kept if present.
#
# ONLOGON is the initial choice; whether the tasks must survive without a logon session
# (ONSTART + the System account's exe-relative bridge.env resolution) is phase 09's real-host
# checkpoint.

$ErrorActionPreference = "Stop"

# --- layout
$RepoRoot = if (Test-Path "$PSScriptRoot\..\.git") { Split-Path $PSScriptRoot -Parent } else { $null }
$DeployRoot = "C:\Users\Administrator\project\vtradingview"
if (-not $RepoRoot) { $RepoRoot = $DeployRoot }
$BridgeDir = Join-Path $DeployRoot "bridge"
$EnvFile = Join-Path $BridgeDir "bridge.env"
$MiseConfig = Join-Path $BridgeDir "mise.mt5.toml"
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

# --- 2. the bridge binary (mise.mt5.toml: the release asset, MISE_ENV=mt5)
if (-not (Test-Path $MiseConfig)) { throw "mise.mt5.toml not found at $MiseConfig (deploy the repo's bridge\ directory first)" }
Say "mise install mt5-bridge (MISE_ENV=mt5)"
$env:MISE_ENV = "mt5"
& $Mise install mt5-bridge 2>&1 | Select-Object -Last 2
$BridgeExe = (& $Mise which mt5-bridge 2>$null)
if (-not $BridgeExe) { throw "mise which mt5-bridge returned nothing" }
Remove-Item Env:MISE_ENV
Say "bridge binary: $BridgeExe"

# --- 3. cloudflared (the HTTPS front; mise registry: aqua:cloudflare/cloudflared)
Say "mise install cloudflared"
& $Mise install cloudflared 2>&1 | Select-Object -Last 2
$TunnelExe = (& $Mise which cloudflared 2>$null)
if (-not $TunnelExe) { throw "mise which cloudflared returned nothing" }
Say "cloudflared: $TunnelExe"

# --- 4. bridge.env: BRIDGE_TOKEN generated once; MT5_MCP_TOKEN asked once and stored
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

# --- 5. scheduled tasks (ONLOGON): the bridge as a bare exe path, the tunnel beside it.
# The binary reads bridge.env from its own directory (exe-relative), so no working directory
# or shell wrapper is needed. Phase 09 verifies ONSTART + the System account on the real host.
$BridgeTask = "VTVBridge"
$TunnelTask = "VTVTunnel"
$BridgeAct = New-ScheduledTaskAction -Execute $BridgeExe
$TunnelAct = New-ScheduledTaskAction -Execute $TunnelExe `
    -Argument "tunnel --url http://127.0.0.1:$($BridgeEnv['BRIDGE_PORT']) --no-autoupdate"
$Trigger = New-ScheduledTaskTrigger -AtLogOn
foreach ($t in @(@{ Name = $BridgeTask; Action = $BridgeAct }, @{ Name = $TunnelTask; Action = $TunnelAct })) {
    if (Get-ScheduledTask -TaskName $t.Name -ErrorAction SilentlyContinue) {
        Say "scheduled task $($t.Name) already registered, updated"
        Set-ScheduledTask -TaskName $t.Name -Trigger $Trigger -Action $t.Action | Out-Null
    } else {
        Say "registering scheduled task $($t.Name) (ONLOGON)"
        Register-ScheduledTask -TaskName $t.Name -Trigger $Trigger -Action $t.Action | Out-Null
    }
}

# --- 6. start both now, verify /health, save the public URL
function Stop-Old($name) {
    Get-Process -Name $name -ErrorAction SilentlyContinue | Stop-Process -Force -ErrorAction SilentlyContinue
}
Stop-Old "mt5-bridge"
Stop-Old "cloudflared"

Say "starting the bridge on 127.0.0.1:$($BridgeEnv['BRIDGE_PORT'])"
$BridgeProc = Start-Process -FilePath $BridgeExe -WorkingDirectory $BridgeDir -WindowStyle Hidden -PassThru `
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
