@echo off
rem bridge/install.cmd: CMD entry for bridge/host-install.ps1 (same install, one double-click).
rem Run on the Windows box that runs MetaTrader 5.
rem   - the mt5-bridge binary and cloudflared come from the mise registry (installed by the ps1)
rem   - mise itself: irm https://mise.run/powershell | iex
rem Asks once for the MT5 MCP token (Tools > Options > MCP), then starts the bridge + tunnel
rem and prints the public HTTPS URL and the BRIDGE_TOKEN to register in the hub's Settings.

setlocal
set "SCRIPT_DIR=%~dp0"
where powershell >nul 2>nul
if errorlevel 1 (
  echo [install.cmd] powershell.exe not found on PATH
  exit /b 1
)
powershell -NoProfile -ExecutionPolicy Bypass -File "%SCRIPT_DIR%host-install.ps1" %*
endlocal & exit /b %errorlevel%
