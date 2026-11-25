#!/usr/bin/env bash
# Restore drill: prove the nightly backup restores, without ever touching the production
# database. Downloads the newest R2 backup, restores it into a scratch database on the same
# Postgres server, asserts the schema and row counts, boots the release hub-server against the
# scratch, and tears everything down (only PIDs this script started).
#
#   VTV_ENV_FILE      env file to source           default /etc/vtv/env
#   DATABASE_URL      production connection        required; names the Postgres server only
#   R2_BACKUP_REMOTE  rclone remote                required
#   R2_BACKUP_BUCKET  rclone bucket                required
#   HUB_BIN           hub-server binary to boot    default /opt/vtv/hub-server
#   SCRATCH_PORT      port for the scratch server  default 18770
#
#   bash ops/restore_drill.sh         # the drill
#   bash ops/restore_drill.sh --check # validate the drill's own structure (no database touched)

set -euo pipefail

SELF="${BASH_SOURCE[0]}"
ENV_FILE="${VTV_ENV_FILE:-/etc/vtv/env}"
HUB_BIN="${HUB_BIN:-/opt/vtv/hub-server}"
SCRATCH_PORT="${SCRATCH_PORT:-18770}"
DRILL_DB="vtv_drill_$(date -u +%Y%m%d%H%M%S)"
WORK="$(mktemp -d /tmp/vtv-drill.XXXXXX)"
FAILURES=0

# --check: the structural self-test (the mutation target for the plan's m4). It asserts the
# drill still refuses on a failed assertion and still never targets the production database.
if [ "${1:-}" = "--check" ]; then
  # Scan only the drill BODY, never the whole file: a pattern that also appears in this block
  # would match itself and the check could never fail (found by mutation m4).
  PIPE=0
  grep -q '^set -euo pipefail' "$SELF" && PIPE=1
  BODY="$(sed -n '/^# --- env/,$p' "$SELF")"
  hay() { printf '%s' "$BODY" | grep -q -- "$1"; }
  [ "$PIPE" -eq 1 ] || { echo "FAIL: set -euo pipefail missing"; exit 1; }
  hay 'RESTORE DRILL: PASS' || { echo "FAIL: no PASS verdict"; exit 1; }
  hay 'exit 1' || { echo "FAIL: no nonzero exit on failure"; exit 1; }
  hay 'fail "pg_restore exited nonzero"' || { echo "FAIL: pg_restore failure not asserted"; exit 1; }
  hay 'fail "table \$table unreadable"' || { echo "FAIL: row-count assertion missing"; exit 1; }
  hay 'SCRATCH_URL' || { echo "FAIL: no scratch database URL"; exit 1; }
  hay 'pg_restore --no-owner --dbname "\$SCRATCH_URL"' || { echo "FAIL: restore does not target the scratch"; exit 1; }
  hay 'DROP DATABASE IF EXISTS \$DRILL_DB' || { echo "FAIL: scratch database is not created/dropped"; exit 1; }
  echo "restore_drill.sh structure OK"
  exit 0
fi

note() { printf '==> %s\n' "$*"; }
fail() { printf 'FAIL: %s\n' "$*" >&2; FAILURES=$((FAILURES + 1)); }
pass() { printf 'PASS: %s\n' "$*"; }

cleanup() {
  # Only PIDs this script started (R: never kill by name).
  if [ -n "${SERVER_PID:-}" ] && kill -0 "$SERVER_PID" 2>/dev/null; then
    kill "$SERVER_PID" 2>/dev/null || true
    wait "$SERVER_PID" 2>/dev/null || true
  fi
  if [ -n "${SERVER_PID:-}" ]; then
    psql "$SCRATCH_URL" -c 'SELECT 1' >/dev/null 2>&1 && psql "$ADMIN_URL" -c "DROP DATABASE IF EXISTS $DRILL_DB" >/dev/null 2>&1 || true
  fi
  rm -rf "$WORK"
}
trap cleanup EXIT

note "drill work dir: $WORK"

# --- env ---------------------------------------------------------------------
if [ ! -r "$ENV_FILE" ]; then
  echo "error: env file not readable: $ENV_FILE" >&2
  exit 2
fi
set -a
# shellcheck disable=SC1090
. "$ENV_FILE"
set +a
for var in DATABASE_URL R2_BACKUP_REMOTE R2_BACKUP_BUCKET; do
  [ -n "${!var:-}" ] || { echo "error: missing $var in $ENV_FILE" >&2; exit 2; }
done

# The production URL names the server; the drill only ever reads the server address from it.
# Derive the admin URL (same server, default postgres db, same credentials) and the scratch URL.
ADMIN_URL="$(python3 - "$DATABASE_URL" <<'PY'
import sys, urllib.parse as u
p = u.urlparse(sys.argv[1])
q = u.parse_qs(p.query)
q.pop('sslmode', None)
qs = u.urlencode(q)
print(u.urlunparse(p._replace(path='/postgres', query=qs)))
PY
)"
SCRATCH_URL="$(python3 - "$DATABASE_URL" <<PY
import sys, urllib.parse as u
p = u.urlparse("$DATABASE_URL")
q = u.parse_qs(p.query); q.pop('sslmode', None)
print(u.urlunparse(p._replace(path='/$DRILL_DB', query=u.urlencode(q))))
PY
)"

# --- newest backup -----------------------------------------------------------
note "finding the newest backup in $R2_BACKUP_REMOTE:$R2_BACKUP_BUCKET"
NEWEST="$(rclone lsf "$R2_BACKUP_REMOTE:$R2_BACKUP_BUCKET" --files-only | grep '\.dump$' | sort | tail -n 1)"
if [ -z "$NEWEST" ]; then
  echo "error: no .dump object in the bucket" >&2
  exit 1
fi
note "newest: $NEWEST"
rclone copy "$R2_BACKUP_REMOTE:$R2_BACKUP_BUCKET/$NEWEST" "$WORK" || { echo "error: download failed" >&2; exit 1; }
DUMP="$WORK/$NEWEST"
[ -s "$DUMP" ] || { echo "error: downloaded dump is empty" >&2; exit 1; }

# --- restore into the scratch ------------------------------------------------
note "creating scratch database $DRILL_DB (server from DATABASE_URL, never the prod db)"
psql "$ADMIN_URL" -c "DROP DATABASE IF EXISTS $DRILL_DB" >/dev/null
psql "$ADMIN_URL" -c "CREATE DATABASE $DRILL_DB" >/dev/null

note "pg_restore into the scratch"
pg_restore --no-owner --dbname "$SCRATCH_URL" "$DUMP" || { fail "pg_restore exited nonzero"; }

# --- assertions --------------------------------------------------------------
note "asserting schema_migrations against the repo's migration set"
APPLIED="$(psql "$SCRATCH_URL" -t -A -c 'SELECT version FROM schema_migrations ORDER BY version' 2>/dev/null || true)"
if [ -z "$APPLIED" ]; then
  fail "schema_migrations missing or empty in the scratch"
else
  pass "schema_migrations versions: $(echo "$APPLIED" | tr '\n' ' ')"
fi
REPO_MIGRATIONS="$(ls "$(dirname "$0")/../crates/hub-core/migrations/"*.up.sql 2>/dev/null | while read -r f; do basename "$f" | cut -d_ -f1 | sed 's/^0*//'; echo; done | tr '\n' ' ')"
if [ -n "$REPO_MIGRATIONS" ]; then
  MATCH="yes"
  for v in $APPLIED; do
    case " $REPO_MIGRATIONS " in
      *" $v "*) : ;;
      *) MATCH="no: version $v not in the repo set" ;;
    esac
  done
  if [ "$MATCH" = "yes" ]; then pass "applied versions match the repo's migrations"; else fail "$MATCH"; fi
fi

note "row counts per account table"
for table in users identities sessions desktop_auth_codes; do
  COUNT="$(psql "$SCRATCH_URL" -t -A -c "SELECT count(*) FROM $table" 2>/dev/null || echo ERR)"
  if [ "$COUNT" = "ERR" ]; then fail "table $table unreadable"; else pass "$table: $COUNT rows"; fi
done

# --- boot the release binary against the scratch -----------------------------
if [ -x "$HUB_BIN" ]; then
  note "booting hub-server against the scratch on 127.0.0.1:$SCRATCH_PORT"
  mkdir -p "$WORK/logs"
  env -i HOME="$HOME" PATH="$PATH" \
    DATABASE_URL="$SCRATCH_URL" \
    AUTH_ENABLED=true \
    HUB_HTTP_PORT="$SCRATCH_PORT" \
    HUB_LOG_DIR="$WORK/logs" \
    PUBLIC_ORIGIN="${PUBLIC_ORIGIN:-https://vtradingview.com}" \
    GOOGLE_CLIENT_ID="${GOOGLE_CLIENT_ID:-drill-placeholder}" \
    GOOGLE_CLIENT_SECRET="${GOOGLE_CLIENT_SECRET:-drill-placeholder}" \
    VTV_SECRET_KEY="${VTV_SECRET_KEY:?VTV_SECRET_KEY must be set in the env file}" \
    "$HUB_BIN" > "$WORK/hub.log" 2>&1 &
  SERVER_PID=$!

  UP=no
  for _ in $(seq 1 30); do
    if curl -fs "http://127.0.0.1:$SCRATCH_PORT/api/health" > /dev/null 2>&1; then UP=yes; break; fi
    if ! kill -0 "$SERVER_PID" 2>/dev/null; then break; fi
    sleep 1
  done
  if [ "$UP" = "yes" ]; then
    pass "/api/health answered against the restored database"
  else
    fail "hub-server did not come up against the scratch (log: $WORK/hub.log)"
  fi
  if curl -fs "http://127.0.0.1:$SCRATCH_PORT/auth/me" 2>/dev/null | grep -q '"auth"'; then
    pass "/auth/me answered"
  else
    fail "/auth/me did not answer"
  fi
else
  note "SKIP: hub binary not found at $HUB_BIN (assertions above still ran)"
fi

# --- verdict -----------------------------------------------------------------
if [ "$FAILURES" -eq 0 ]; then
  echo "RESTORE DRILL: PASS"
else
  echo "RESTORE DRILL: FAIL ($FAILURES failing assertion(s))"
  exit 1
fi
