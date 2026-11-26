# MT5 bridges: run your own broker feed

An MT5 bridge is a small Python script (`bridge/mt5_bridge.py`) that you run on the machine
where your MetaTrader 5 terminal lives. The hub calls it over HTTPS and your charts show your
broker's candles. It is read-only: it can never place an order.

## What you need

- The machine with your MT5 terminal (your Windows box or a Linux server with MT5 under Wine).
  The terminal must run its native MCP server (Tools > Options > MCP) and be logged in.
- Python 3 on that machine (standard library only, no packages to install).
- A way to publish an HTTPS URL that reaches the bridge: your own Cloudflare Tunnel or Caddy.

## Run the bridge

Set three environment variables and start it:

```sh
export BRIDGE_TOKEN="$(python3 -c 'import secrets; print(secrets.token_urlsafe(32))')"
export MT5_MCP_TOKEN="<the token from MT5: Tools > Options > MCP>"
python3 bridge/mt5_bridge.py
```

The bridge refuses to start without `BRIDGE_TOKEN`. It binds `127.0.0.1:8797` by default
(`BRIDGE_BIND` and `BRIDGE_PORT` override); the terminal is the only thing it talks to besides
the hub.

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
