# MT5 bridges: run your own broker feed

An MT5 bridge is a single release binary (`mt5-bridge`, built from `crates/mt5-bridge`) that
you run on the machine where your MetaTrader 5 terminal lives. The hub calls it over HTTPS and
your charts show your broker's candles. It is read-only: it can never place an order.

## What you need

- The machine with your MT5 terminal (your Windows box, or a Linux server with MT5 under Wine
  for the best-effort linux builds). The terminal must run its native MCP server
  (Tools > Options > MCP) and be logged in.
- [mise](https://mise.jdx.dev) on that machine (the one-command install).
- A way to publish an HTTPS URL that reaches the bridge: your own Cloudflare Tunnel or Caddy.

## Run the bridge

**The supported path is the manager app**, `BridgeManager` from the release. It installs the bridge
and cloudflared through mise, writes `bridge.env`, registers both startup tasks, pairs the bridge to
your account with a one-time code from Settings, and shows status. See [manager.md](manager.md).

The manual route below stays supported for a host where you would rather not run a desktop app, and
it is what the manager does underneath.

Install the pinned release through mise (the windows-x64 asset is the supported leg; the linux
x64/arm64 assets are best-effort and untested against a real terminal):

```sh
MISE_ENV=mt5 mise install mt5-bridge
```

Then set the environment and start it (a `bridge.env` file next to the exe works too; existing
environment variables win):

```sh
export BRIDGE_TOKEN="$(python3 -c 'import secrets; print(secrets.token_urlsafe(32))')"
export MT5_MCP_TOKEN="<the token from MT5: Tools > Options > MCP>"
mt5-bridge
```

The bridge refuses to start without `BRIDGE_TOKEN`. It binds `127.0.0.1:8797` by default
(`BRIDGE_BIND` and `BRIDGE_PORT` override); the terminal is the only thing it talks to besides
the hub. `BRIDGE_STATE_DIR` (default: the exe's directory) holds the small `.bridge_offset`
server-time file.

On the MT5 host, `bridge/host-install.ps1` does all of it: mise, the bridge binary from
`mise.mt5.toml`, cloudflared, `bridge.env`, ONSTART scheduled tasks (SYSTEM, restart-on-failure),
and the tunnel. It is the repair path beside the manager; both write the same files and tasks.

## Publish it over HTTPS

The hub only dials `https://` URLs, so the bridge needs TLS in front of it. Two proven ways:

**Cloudflare Tunnel** (no open ports on your machine):

```sh
cloudflared tunnel --url http://127.0.0.1:8797
```

This prints a temporary `https://<something>.trycloudflare.com` URL. For a stable name, create
a named tunnel in your Cloudflare account and point it at `http://127.0.0.1:8797`.

**Caddy** (a VPS with a domain):

```
bridge.example.com {
    reverse_proxy 127.0.0.1:8797
}
```

Caddy obtains and renews the certificate automatically.

## Register it in the hub

1. Sign in, open the avatar menu, MT5 bridges.
2. Enter the HTTPS URL and the `BRIDGE_TOKEN` value, and press Test.
3. The server checks the address (it refuses private, loopback and link-local targets), calls
   the bridge's `/health`, and shows the real result: connected with your terminal's symbol
   count, or the exact failure.
4. On success your MT5 symbols appear in the symbol picker as an `mt5` source, and a chart on
   one of them streams your broker's candles.

The token is stored encrypted on the server and never shown again. No other user can see or
use your bridge.

## What a failure means

| The screen says | Why |
|---|---|
| the bridge url must be https | the URL must start with `https://` |
| the bridge address was refused: ... | the URL points at a private, loopback or link-local address; the server never dials those |
| the bridge host did not resolve | the domain name does not resolve from the server |
| certificate error | the HTTPS certificate is invalid or self-signed |
| the bridge refused the token | the `BRIDGE_TOKEN` in Settings does not match the bridge's |
| the bridge did not answer in time | the bridge or your tunnel was slower than 10 seconds |
| the response exceeded the size cap | the bridge answered with more than 5 MB |
| anything else | the bridge's own error text, passed through |

## Local development

On a development machine with `AUTH_ENABLED=false`, the server accepts one loopback bridge
configured by environment variables instead of a Settings entry:

```sh
VTV_LOCAL_BRIDGE_URL=http://127.0.0.1:8797
VTV_LOCAL_BRIDGE_TOKEN=<the same value the bridge runs with>
```

Loopback is refused whenever auth is on; registered bridges are always `https://` and vetted.

## Security notes

- The bridge exposes read-only market data of the logged-in terminal: candles, symbols, health.
  It never places orders (the code has no order path).
- The bearer token is the only gate between the internet and your terminal. Keep it secret;
  the hub stores it encrypted and the bridge never logs it.
- Keep `BRIDGE_BIND` on `127.0.0.1` and let the tunnel or Caddy own the TLS; the bridge itself
  never speaks HTTPS.
