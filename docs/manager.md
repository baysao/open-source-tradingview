# The bridge manager app (phase 09)

`BridgeManager` is a Tauri 2 desktop app (`crates/mt5-manager`, identifier
`com.vtradingview.bridgemanager`) that runs on the Windows machine with your MetaTrader 5 terminal.
It does four things: it installs the bridge and the HTTPS tunnel on that machine, it pairs the
bridge to your account with a one-time code, it starts and stops the two, and it shows whether they
are actually up.

It replaces `bridge/host-install.ps1` as the normal path. The script still works and stays as the
repair path; both write the same files, the same two scheduled tasks and the same env contract, so
either one can repair what the other installed.

## Install (step 1)

1. Download `mt5-manager-vX.Y.Z-windows-x64.zip` from the release, unpack it anywhere, and run
   `mt5-manager.exe`. The `.exe` inside the zip is portable — no installer, no admin prompt. The
   release also carries `-setup.exe` (an NSIS installer) if you prefer a Start-menu entry.

   **Before the first `v0.3.0` tag exists** there is no release asset to download. Build it on a
   Windows box with a Rust toolchain and WebView2 (the dev box qualifies) and copy the artefact
   over:

   ```sh
   cargo tauri build --features gui   # run in crates/mt5-manager
   ```

   `--features gui` is not optional: without it the binary prints
   `mt5-manager: built without the 'gui' feature` and exits 2.

   That leaves `crates/mt5-manager/target/release/mt5-manager.exe` — copy **that one file** and run
   it. Nothing else is needed: `rust-toolchain.toml` pins `x86_64-pc-windows-msvc`, where
   `webview2-com-sys` links `WebView2Loader` **statically**, so the exe imports no custom DLL.

   Do not build with a `*-windows-gnu` toolchain. There the loader is linked dynamically and the exe
   will not start without `WebView2Loader.dll` beside it — Windows reports `0xC0000135`
   (`STATUS_DLL_NOT_FOUND`). `release.yml` copies that DLL into the zip when a build produces it and
   fails the job when it is missing, but the NSIS installer from a GNU build does not carry it
   (DEBT-048). MSVC avoids the whole question.
2. On first run the window is one panel of **five numbered steps, top to bottom**. Work down them in
   order; steps 1 and 2 are one-time setup, 3 and 4 are what you come back to, 5 is terminal.

   | Step | Zone | When |
   |------|------|------|
   | 1 | **Install or repair** | first run, and whenever something looks broken |
   | 2 | **Pair with your account** | once, after step 1 |
   | 3 | **Check it is running** | any time |
   | 4 | **Start or stop the services** | any time |
   | 5 | **Unregister** | last: deletes the bridge from your account |

3. In **step 1, Install or repair**:
   - paste the **MT5 MCP token** (in the terminal: *Tools > Options > MCP*) — it is only needed on a
     first install;
   - leave **MT5 MCP address** at `http://127.0.0.1:22346/mcp` unless the terminal shows another
     port. That is the terminal's own endpoint: `127.0.0.1`, port `22346`, path `/mcp`, plain HTTP,
     loopback only. The app refuses `https://`, a non-loopback host, a path other than `/mcp`, or a
     missing port, because the terminal serves none of those;
   - the two dots beside the labels check themselves: ~1 s after you stop typing, the app probes the
     address (green = port and path answer) and the token (green = the terminal accepted it, red =
     refused), without installing anything;
   - press **Install or repair**.

**While it runs** a progress bar names the step and, for a download, the file mise is fetching
(`mt5-bridge-v0.3.0-windows-x64.zip`, `cloudflared-…-windows-amd64.exe`). Fifteen seconds of silence
is worse than fifteen seconds of "3 of 8: bridge.env", and mise names the asset it resolved — so the
detail is read from mise's own output, never guessed.

**Step 1 checks the terminal too, not just the bridge.** After registering the tasks it probes the
MCP address directly and reports the answer as its own step: `reachable: answered 401` means the
terminal is up and asking for the token you already stored (MetaTrader5's MCP answers an
unauthenticated call with 401, measured). Without this the panel could show a healthy bridge that
can never fetch a bar — the bridge answers `/health` even when the terminal's MCP is dead, exactly as
the retired Python bridge did.

The deploy root is fixed at `C:\Users\Administrator\project\vtradingview` (the path the host
scripts and the tunnel config assume); the panel has no field for it.

It then works through these steps in order, and the panel lists each one as *installed*,
*updated*, *kept* or *skipped with a reason*:

| Step | What happens |
|------|--------------|
| `mise install mt5-bridge` | fetches the pinned bridge release through mise (no Rust toolchain, no build) |
| `mise install cloudflared` | fetches the tunnel binary the same way |
| `bridge.env` | creates it if absent (`BRIDGE_TOKEN` generated once, `MT5_MCP_TOKEN` stored); **never rewritten** when present |
| `bridge.env beside the binary` | copies the same file next to the exe, which is the only place the binary reads it |
| `cloudflared tunnel` | reuses the named tunnel `mt5`, creates it when missing, or *skips* and tells you to run `cloudflared tunnel login` once |
| `task VTVBridge` / `task VTVTunnel` | registers both as `ONSTART`, `SYSTEM`, restart-on-failure |
| `bridge /health` | one bounded 15-second poll; the panel names the likely cause when it fails |

A first install therefore needs **one manual step**: `cloudflared tunnel login` in a browser on that
host, which authorises the domain. Run it, then press **Install or repair** again. Until then the
tunnel half is skipped, and the bridge half still installs and starts.

Re-running Install is safe and is the repair action. It keeps `bridge.env` (so the token does not
change under a bridge your account already knows), refreshes the derived tunnel config, updates the
two tasks in place, and re-polls `/health`.

## Pair (step 2)

1. On `vtradingview.com`, open **Settings → MT5 bridge** and generate a pairing code. It is shown
   once, expires in 10 minutes, and any failed attempt burns it — generate a fresh one instead of
   retrying a rejected code.
2. In the app's **step 2, Pair with your account**, press **Pair with a code…** and fill in:
   - **Hub URL** — `https://vtradingview.com` (pre-filled; it must be a bare `https://` origin, no
     path or query);
   - **Bridge label** — how this host appears in Settings (pre-filled `mt5-host`);
   - **Bridge URL** — the tunnel URL, `https://mt5.<your domain>` for a named tunnel;
   - **Bridge token** — the `BRIDGE_TOKEN` value from `bridge.env` on the host;
   - **Pairing code** — ten digits, spaces allowed.

   Both URLs must be `https://`. The app refuses a non-`https` **hub** URL before it sends anything
   (the bridge token would otherwise cross in clear text), and the hub refuses a non-`https` **bridge**
   URL when the bridge is registered.
3. On success the bridge appears in Settings with its symbol count, and the app shows the bridge id.
   The **revoke code** the hub returns is displayed once: it is what lets the app unregister the
   bridge later, and it is not stored anywhere on disk.

## Status (step 3)

**Status** answers four separate questions, each with its own number:

| Row | Question |
|-----|----------|
| Local bridge | does `http://127.0.0.1:8797/health` answer with the bearer token, and how long did it take? |
| Bridge task | what state does Windows report for `VTVBridge`? |
| Tunnel task | the same for `VTVTunnel` |
| Public URL | does `/health` answer at the tunnel URL — i.e. is the bridge reachable from the hub? |

**Check now** re-runs all four. Left alone, the panel re-checks no more often than every 15 seconds,
so an open window does not hammer the host.

## Control (step 4)

**Control** starts, stops, or restarts `VTVBridge` and `VTVTunnel` through their scheduled tasks.
When a task exists but its process is gone (a crash, or a manual kill), Stop also sweeps the orphaned
process by command line, so the panel's state matches reality rather than the task's last recorded
result.

## Unregister (step 5)

**Registration → Unregister** calls the hub and deletes the bridge from your account, using the
revoke code from pairing. Deleting the bridge in Settings on the website does the same from the
other side: the app's next status check finds it gone. Either direction works, and both leave the
other consistent.

## The two scheduled tasks

Both run as `SYSTEM` at **startup** (not login), with restart-on-failure (3 attempts, one minute
apart), no execution time limit, and *start when available* so a missed trigger still fires after a
reboot. Consequence: the machine needs no interactive logon for the bridge to come back after a
reboot — the property an ONLOGON task does not have.

The bridge task carries **no arguments**. The binary reads `bridge.env` from its own directory and
parses nothing else, so `install` copies the deploy-root file beside the exe. The tunnel task passes
`--config <deploy root>\cloudflared\config.yml`.

## Troubleshooting

| Symptom | Cause and fix |
|---------|---------------|
| `Check address and token` says *the token is not* | The address is right — the terminal answered and rejected the credential. Copy the token again from *Tools > Options > MCP*; it changes when the MCP server is restarted. |
| `MT5 MCP address and token` skipped: *did not answer … refused* | The terminal's MCP is off, or the port is wrong. Check *Tools > Options > MCP* and correct **MT5 MCP address** in step 1 (the default is `http://127.0.0.1:22346/mcp`). |
| `MT5 MCP endpoint` skipped: *answered 404 … is that the terminal's MCP address?* | Something else is listening on that port. Correct the address; the terminal's own UI shows its real one. |
| `bridge /health` skipped: *no /health answer in 15s* | The MT5 terminal is not running, or its MCP server is off (*Tools > Options > MCP*). Start the terminal, then **Install or repair** again. |
| `cloudflared tunnel` skipped: *run `cloudflared tunnel login`* | This host has no Cloudflare certificate yet. Run that command once in a browser on this host, then install again. |
| Pairing answers 401 | The code expired (10 minutes) or a previous attempt burned it. Generate a new one in Settings. |
| Pairing answers 422 | A required field is missing or the URL is not `https://`. The message names which. |
| Pairing answers 502 | The hub reached your Bridge URL but the bridge did not answer: check **Status → Public URL** and the host's `logs\bridge.err`. |
| The bridge works locally but not from the hub | The tunnel task is stopped, or the tunnel config points at another tunnel. **Control → Tunnel → Start**, then **Status**. |
| After a reboot nothing is running | Check `schtasks /query /tn VTVBridge /v` — the task should say `ONSTART` and `Ready`/`Running`. If it says `ONLOGON`, an old install is still in place: run **Install or repair** to update it in place. |
| `BRIDGE_TOKEN is required` in `logs\bridge.err` | The env file is missing beside the exe (a `mise install` replaced its directory). **Install or repair** re-writes it. |

Logs live under `<deploy root>\logs\`: `bridge.log`, `bridge.err`, `tunnel.log`, `tunnel.err`.

## What this app is not

- It does not place, modify, or cancel orders. The bridge is read-only.
- It does not store your bridge token on disk. Pairing keeps it in memory for the session; after a
  restart, pair again (the bridge itself keeps working — only the app's ability to act on it needs
  the code again).
- It does not deploy anything to the website. The hub is separate.
