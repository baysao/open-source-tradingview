# Ops: the production VPS runbook

The production host is one rented Linux VPS in a region that reaches the Binance and Bybit
public endpoints (not US; spec Q1). It opens **no inbound port except SSH**. Everything public
arrives through the Cloudflare Tunnel. This file is the runbook; every command assumes you are
on the host with sudo, or says otherwise.

## What runs where

| Process | Supervisor | Listens on | Config |
|---|---|---|---|
| hub-server (release binary) | systemd `vtv.service` | 127.0.0.1:8770 | `/etc/vtv/env` |
| cloudflared | systemd (its own installer) | outbound only | `/etc/cloudflared/config.yml` |
| Postgres 17 | distro service | 127.0.0.1:5432 | distro config |
| backup | systemd `vtv-backup.timer` | nightly 04:00 UTC | `ops/backup.sh` |

Nothing else. No Redis, no second instance, no container runtime, no admin panel on the box.

## VPS prep (one time)

```sh
# packages
sudo apt update && sudo apt install -y postgresql-client-17 curl
# cloudflared and rclone from their own install docs; verify:
cloudflared --version && rclone --version

# rustup is only for the build fallback (ops/deploy.sh without --from-mise); the prebuilt
# release route installs the binary with mise and needs no toolchain
# curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh -s -- -y
curl https://mise.run | sh   # the binary installer: mise install, mise which vtv

# service user: no login shell, no sudo
sudo useradd -r -m -d /opt/vtv -s /usr/sbin/nologin vtv

# layout
sudo mkdir -p /opt/vtv/repo /opt/vtv/hub/logs /opt/vtv/hub/.cache/orderflow
sudo git clone <REPO_URL> /opt/vtv/repo        # the public repo; see AGPL below
sudo chown -R vtv:vtv /opt/vtv

# firewall: SSH only (ufw example; adjust to the provider's tooling)
sudo ufw default deny incoming && sudo ufw allow OpenSSH && sudo ufw enable
```

## Postgres 17

Install the server from the PGDG repo (the distro repo may lag). Then:

```sh
sudo -u postgres createuser vtv --pwprompt   # or peer auth; adjust DATABASE_URL
sudo -u postgres createdb -O vtv vtv
```

Keep Postgres on loopback (`listen_addresses = 'localhost'`): the hub connects locally; the
tunnel never reaches it directly.

## The env file (the only place secrets live)

```sh
sudo install -m 640 -o root -g vtv /dev/null /etc/vtv/env
sudoedit /etc/vtv/env
```

Contents (no quotes needed, systemd parses them; never commit this file):

```text
HUB_HTTP_PORT=8770
AUTH_ENABLED=true
PUBLIC_ORIGIN=https://vtradingview.com
DATABASE_URL=postgres://vtv:<password>@127.0.0.1:5432/vtv
GOOGLE_CLIENT_ID=<from Google Cloud console>
GOOGLE_CLIENT_SECRET=<from Google Cloud console>
VTV_SECRET_KEY=<32+ random bytes, base64url; see .env.example for a generator>
ADMIN_EMAILS=<comma-separated admin addresses>
R2_BACKUP_REMOTE=r2
R2_BACKUP_BUCKET=vtradingview-db
```

Contract: every variable above is required except `ADMIN_EMAILS` (optional) and the two
`R2_BACKUP_*` names (required by the backup unit only). Mode 0640 root:vtv. `hub-server`
refuses to start naming any missing one. No secret value may appear in the repo, in any ops
file, or in any log (the server's `Config` Debug output redacts them already).

Rotation:

- `VTV_SECRET_KEY`: rotating it invalidates every session (users sign in again). Rotate on
  suspicion of leak only: new key, restart, users re-sign-in.
- `GOOGLE_CLIENT_SECRET`: rotate in the Google Cloud console, then update the env file and
  restart; sign-in is broken until they match.
- R2 token: rotate in the Cloudflare dashboard, update rclone's config (below), test one
  `ops/backup.sh --check` then one manual run.

## Cloudflare Tunnel

One-time, on the host (or from anywhere with the cert):

```sh
cloudflared tunnel login                     # browser once
cloudflared tunnel create vtradingview       # writes the credentials file
sudo cp /opt/vtv/repo/ops/cloudflared.yml /etc/cloudflared/config.yml
sudo cloudflared service install <token from the create output>
```

Then in the Cloudflare dashboard point `vtradingview.com` (and `www`, if used) at the tunnel.
Verify from the outside: `curl -fs https://vtradingview.com/api/health`. The host still has no
inbound port; HSTS is set by Cloudflare, not by the origin.

## Services

```sh
sudo install -m 644 /opt/vtv/repo/ops/vtv.service /etc/systemd/system/
sudo install -m 644 /opt/vtv/repo/ops/vtv-backup.service /etc/systemd/system/
sudo install -m 644 /opt/vtv/repo/ops/vtv-backup.timer /etc/systemd/system/
sudo systemctl daemon-reload
sudo systemctl enable --now vtv.service vtv-backup.timer
```

Deploy, two modes (both write `hub/SOURCE.txt`, restart, smoke `/api/health`, print the rollback
command):

```sh
# preferred: install the prebuilt release asset, no build and no rust toolchain on the host
bash /opt/vtv/repo/ops/deploy.sh --from-mise 0.1.0
# fallback: build from the pushed commit on the host (needs rustup, ~5 min)
bash /opt/vtv/repo/ops/deploy.sh
```

The prebuilt route uses `mise.toml`'s `[tools."http:vtv"]`: the release workflow
(`.github/workflows/release.yml`, triggered by a `vX.Y.Z` tag) publishes
`vtv-vX.Y.Z-linux-{x64,arm64}.tar.gz` from `baysao/open-source-tradingview`, and mise installs
that asset under `~/.local/share/mise/installs/http-vtv/<version>/`. `--from-mise` pins the
version with `mise use` and swaps in `mise which vtv`, so a rollback is the same one-liner as
before. A deploy is then: tag → CI publishes → `ops/deploy.sh --from-mise <version>`.

The dev stack no longer needs gaffa: `mise run dev` (or `mise run dev:db`, `dev:hub`,
`dev:tunnel`). `mise run gates` runs the Rust gate set; `mise run design` runs the served-page
design gates.

Verify afterwards:

```sh
systemctl is-active vtv
curl -fs https://vtradingview.com/api/health
grep -q 'NoNewPrivileges=true' /etc/systemd/system/vtv.service
grep -q 'ProtectSystem=strict' /etc/systemd/system/vtv.service
grep -q 'http_status:404' /etc/cloudflared/config.yml
grep -q '127.0.0.1:8770' /etc/cloudflared/config.yml
```

## Backups

`ops/backup.sh` runs nightly at 04:00 UTC under the timer: `pg_dump --format=custom`,
uploaded by rclone to R2, objects older than 14 days pruned, a zero-byte dump fails the unit
instead of becoming the only restore point.

rclone setup (as vtv, token scoped to the backup bucket only):

```sh
sudo -iu vtv rclone config    # create remote `r2` (Cloudflare R2, Access Key + Secret)
```

Restore drill (monthly, and after every change to the restore path):

```sh
sudo -iu vtv bash /opt/vtv/repo/ops/restore_drill.sh
```

The drill downloads the newest dump, restores into a scratch database on the same Postgres
server (never the production database), asserts the schema_migrations versions against the
repo's migration files, prints per-table row counts, boots the release hub-server against the
scratch, and tears down only its own PIDs. First real drill on the host is DEBT-019.

## Verification checklist (run after setup and after every deploy)

```sh
systemctl is-active vtv vtv-backup.timer cloudflared
ss -tlnp | grep -v -E '127.0.0.1|::1'   # nothing listening on a public interface but sshd
curl -fs https://vtradingview.com/api/health
curl -fs https://vtradingview.com/SOURCE.txt
sudo -iu vtv bash /opt/vtv/repo/ops/backup.sh --check
```

## AGPL note

The service is AGPL-3.0-only. The hosted site must offer its exact source: `hub/SOURCE.txt`
(written by `ops/deploy.sh` at every deploy) names the commit, and the repository URL in it
must be public before launch. Keep the repo public; a private repo breaks the licence offer.

## Launch blockers not solved on this host (owner rows)

- DEBT-018: the Cloudflare account, the `vtradingview` tunnel, DNS, and the VPS itself.
- DEBT-019: the first real restore drill on this host after the first nightly backup exists.
- DEBT-020: privacy page contact address and final text (Q9).
- DEBT-021: the public repository URL filled into `hub/notices.html` (DEBT-018's prerequisite
  domain work makes the URL real).
- Q2 (data redistribution terms), Q3 (trademark): read before announcing.
