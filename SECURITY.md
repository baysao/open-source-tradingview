# Security

## Threat model (hosted mode)

- **Edge:** Cloudflare Tunnel fronts the site. The origin opens no inbound ports; `hub-server`
  binds `127.0.0.1:8770` only. `CF-Connecting-IP` is trusted only when the TCP peer is loopback
  (the tunnel's own address), so a direct remote caller cannot forge its client IP.
- **Auth:** Google sign-in only; server-side sessions with a 30-day sliding expiry and an
  `HttpOnly` `SameSite=Lax` cookie signed with `VTV_SECRET_KEY`. CSRF is Origin-checked with
  `SameSite=Lax`, no token (D-010). Every POST is JSON-only, Origin-checked and capped at
  300 kB (`net::post_guard`).
- **Admin:** every `/admin/*` path answers 404 unless the signed-in account's email is in
  `ADMIN_EMAILS` (404, not 403: the surface is not advertised to outsiders).
- **Rate limits:** four per-client-IP buckets: `/api/*` 20/s burst 40, `/auth/*` 5/s burst 10,
  `/admin/*` 5/s burst 10, `/ws` 10/s burst 30 (hosted mode only). A 429 carries `Retry-After`.
- **Static tree:** an allow-list (top-level files and `vendor/` with web extensions, plus the
  `.txt` licence, notice and source files). Everything else, including `.env`, logs and caches,
  answers 404.
- **CSP:** computed once at boot, per page: `sha256` hashes of the exact inline `<script>`
  bodies, `default-src 'none'`, `object-src 'none'`, `base-uri 'none'`, `form-action 'none'`,
  `frame-ancestors 'self'` (the hub frames its own engine pages). Residual risk, named:
  `script-src` keeps `'unsafe-eval'` because the bundled Pine compiler uses `new Function`.
  Everything else runs under the hash-locked policy, and a page edited after boot fails closed:
  the drifted block is blocked in the browser, never silently allowed.
- **TTFC beacon:** the one anonymous write (`POST /api/metrics/ttfc`). It stores a count in a
  fixed histogram with no identity, no IP and no session attached; the body is capped at 1 kB,
  `ms` at 120000, and the Api bucket flood-controls it. `GET /admin/metrics` (admin-gated) reads
  the histogram back.
- **Secrets:** environment variables only (`VTV_SECRET_KEY`, `GOOGLE_CLIENT_SECRET`,
  `DATABASE_URL`), from `/etc/vtv/env` on the host; `.env` is git-ignored and 404 from the
  static handler. Log lines are redacted (tokens, passwords, keys, long hex and base64) before
  they are shown or stored.
- **MT5 bridges (Phase 7):** per-user HTTPS bridge URLs behind an SSRF guard (resolve-and-pin,
  deny private ranges, no redirects, size and time caps). The bridge only reads candles and
  symbols; nothing in this project places, changes or closes orders.

## Do not

- Do not open an inbound port on the host; the tunnel is the only path in.
- Do not commit `.env`, MT5 tokens or SSH keys.
- Do not weaken the CSP to ship a new inline script; the boot scanner hashes what is there.

## Reporting a problem

Please open a GitHub issue without exploit details and ask for a private contact, or use GitHub's
"Report a vulnerability" (private advisory) on this repository.
