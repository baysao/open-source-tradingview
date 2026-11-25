#!/usr/bin/env bash
# TradeX dev runner.
#
# `dev [svc...]` starts gaffa (https://github.com/oryon-dominik/interstellar-gaffa), the Procfile
# supervisor, on the repo Procfile. Each Procfile line calls `scripts/dev.sh _svc <name>`, which
# builds and runs that one service in the foreground, so gaffa prefixes, logs and stops it.
# Do not pass `gaffa --shell bash` on Windows: gaffa 2026.11.1 then drops the arguments after the
# script name. The Procfile lines start with plain `bash`; `dev` puts this bash first on PATH.
#
#   bash scripts/dev.sh dev          # every service in the Procfile (db, hub)
#   bash scripts/dev.sh run hub      # one service without gaffa
#   bash scripts/dev.sh _svc hub     # what the Procfile calls
#
# hub: `cargo build -p hub-server`, then target/debug/hub-server on 127.0.0.1:${HUB_HTTP_PORT:-8770}.
# It refuses to start when that port is taken (the Python hub, or another hub-server).
# db: pgsqlite (PGSQLITE_BIN, default ~/.local/bin/pgsqlite) over data/vtv.db on
# 127.0.0.1:${PGSQLITE_PORT:-55432}. Only hub-server with AUTH_ENABLED=true uses it (D-011).
# tunnel: cloudflared with ops/cloudflared.yml (production only; idles with a message when
# cloudflared is not installed locally).

set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
PROCFILE="$ROOT/Procfile"
LOG_DIR="$ROOT/.dev/logs"
ALL_SERVICES=(db hub tunnel)

case "$(uname -s)" in
  MINGW*|MSYS*|CYGWIN*) EXE=".exe" ;;
  *)                    EXE="" ;;
esac

step() { printf '==> %s\n' "$*"; }
err()  { printf 'error: %s\n' "$*" >&2; }
die()  { err "$*"; exit 1; }
have() { command -v "$1" >/dev/null 2>&1; }

port_busy() {
  local p="$1"
  (exec 3<>"/dev/tcp/127.0.0.1/$p") 2>/dev/null
}

svc_known() {
  case "$1" in db|hub|tunnel) return 0 ;; *) return 1 ;; esac
}

svc_port() {
  case "$1" in
    db)  echo "${PGSQLITE_PORT:-55432}" ;;
    hub) echo "${HUB_HTTP_PORT:-8770}" ;;
    *)   echo 0 ;;
  esac
}

# The pgsqlite binary: PGSQLITE_BIN, else ~/.local/bin/pgsqlite (the owner's install, D-011).
pgsqlite_bin() {
  local b="${PGSQLITE_BIN:-$HOME/.local/bin/pgsqlite$EXE}"
  [ -x "$b" ] || [ -f "$b" ] || return 1
  printf '%s\n' "$b"
}

# _svc <name>: one service in the foreground.
cmd_svc() {
  local name="${1:-}"
  svc_known "$name" || die "_svc: unknown service '$name' (known: ${ALL_SERVICES[*]})"
  local port
  port="$(svc_port "$name")"
  if [ "$port" -gt 0 ] && port_busy "$port"; then
    die "$name: 127.0.0.1:$port is already in use (stop the process that holds it first)"
  fi
  cd "$ROOT"
  case "$name" in
    db)
      local bin
      bin="$(pgsqlite_bin)" || die "db: pgsqlite not found; set PGSQLITE_BIN (default \$HOME/.local/bin/pgsqlite$EXE)"
      mkdir -p "$ROOT/data"
      # pgsqlite 0.0.22 has no listen-address option and binds 0.0.0.0 with no password
      # (DEBT-011): pass --host 127.0.0.1 once the installed binary supports it.
      if "$bin" --help 2>/dev/null | grep -q -- '--host'; then
        exec "$bin" --host 127.0.0.1 --database "$ROOT/data/vtv.db" --port "$port"
      fi
      err "db: this pgsqlite listens on every interface (no --host option); do not leave it running on an untrusted network"
      exec "$bin" --database "$ROOT/data/vtv.db" --port "$port"
      ;;
    hub)
      cargo build -p hub-server
      exec "$ROOT/target/debug/hub-server$EXE"
      ;;
    tunnel)
      # Production only (ops/README.md). A dev machine without cloudflared idles instead of
      # crash-looping, so running the whole Procfile locally still works.
      if ! have cloudflared; then
        err "tunnel: cloudflared not installed; the tunnel runs on the production host (ops/README.md)"
        sleep infinity
      fi
      exec cloudflared tunnel --config "$ROOT/ops/cloudflared.yml" run vtradingview
      ;;
  esac
}

cmd_dev() {
  have gaffa || die "gaffa not found: cargo binstall gaffa (or cargo install gaffa)"
  [ -f "$PROCFILE" ] || die "no Procfile at $PROCFILE"
  local names=("$@")
  [ "${#names[@]}" -gt 0 ] || names=("${ALL_SERVICES[@]}")
  local n
  for n in "${names[@]}"; do svc_known "$n" || die "unknown service '$n' (known: ${ALL_SERVICES[*]})"; done
  mkdir -p "$LOG_DIR"
  # Procfile lines start with `bash`: make it this bash, not WSL's or another one.
  PATH="$(dirname "$BASH"):$PATH"
  export PATH
  step "gaffa: ${names[*]}. Ctrl+C stops all. Log: .dev/logs/gaffa-<date>_NNN.log"
  local rc=0
  trap ':' INT   # a handler, not an ignore: gaffa must still receive Ctrl+C
  (cd "$ROOT" && gaffa run "${names[@]}" --procfile "$PROCFILE" --log-file "$LOG_DIR/gaffa.log") || rc=$?
  trap - INT
  return "$rc"
}

usage() {
  sed -n '2,15p' "${BASH_SOURCE[0]}" | sed 's/^# \{0,1\}//'
}

main() {
  local cmd="${1:-}"
  [ $# -gt 0 ] && shift
  case "$cmd" in
    dev)       cmd_dev "$@" ;;
    run|_svc)  cmd_svc "$@" ;;
    ""|-h|--help|help) usage ;;
    *)         usage; die "unknown command '$cmd'" ;;
  esac
}

main "$@"
