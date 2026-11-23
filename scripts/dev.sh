#!/usr/bin/env bash
# TradeX dev runner.
#
# `dev [svc...]` starts gaffa (https://github.com/oryon-dominik/interstellar-gaffa), the Procfile
# supervisor, on the repo Procfile. Each Procfile line calls `scripts/dev.sh _svc <name>`, which
# builds and runs that one service in the foreground, so gaffa prefixes, logs and stops it.
# Do not pass `gaffa --shell bash` on Windows: gaffa 2026.11.1 then drops the arguments after the
# script name. The Procfile lines start with plain `bash`; `dev` puts this bash first on PATH.
#
#   bash scripts/dev.sh dev          # every service in the Procfile (today: hub)
#   bash scripts/dev.sh run hub      # one service without gaffa
#   bash scripts/dev.sh _svc hub     # what the Procfile calls
#
# hub: `cargo build -p hub-server`, then target/debug/hub-server on 127.0.0.1:${HUB_HTTP_PORT:-8770}.
# It refuses to start when that port is taken (the Python hub, or another hub-server).

set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
PROCFILE="$ROOT/Procfile"
LOG_DIR="$ROOT/.dev/logs"
ALL_SERVICES=(hub)

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
  case "$1" in hub) return 0 ;; *) return 1 ;; esac
}

svc_port() {
  case "$1" in hub) echo "${HUB_HTTP_PORT:-8770}" ;; *) echo 0 ;; esac
}

# _svc <name>: one service in the foreground.
cmd_svc() {
  local name="${1:-}"
  svc_known "$name" || die "_svc: unknown service '$name' (known: ${ALL_SERVICES[*]})"
  local port
  port="$(svc_port "$name")"
  if [ "$port" -gt 0 ] && port_busy "$port"; then
    die "$name: 127.0.0.1:$port is already in use (stop the Python hub or the other hub-server first)"
  fi
  cd "$ROOT"
  case "$name" in
    hub)
      cargo build -p hub-server
      exec "$ROOT/target/debug/hub-server$EXE"
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
