#!/usr/bin/env bash
# Nightly database backup: pg_dump (custom format) to Cloudflare R2, 14-day retention.
# Runs on the production VPS under ops/vtv-backup.timer (04:00 UTC). Never commit secrets:
# everything comes from the env file (/etc/vtv/env on the host).
#
#   VTV_ENV_FILE    env file to source            default /etc/vtv/env
#   DATABASE_URL    postgres connection string    required (from the env file)
#   R2_BACKUP_REMOTE  rclone remote name          required (from the env file)
#   R2_BACKUP_BUCKET  rclone bucket name          required (from the env file)
#   BACKUP_KEEP_DAYS  retention in days           default 14
#
#   ops/backup.sh --check   validate the script's own structure (no database touched)

set -euo pipefail

SELF="${BASH_SOURCE[0]}"
ENV_FILE="${VTV_ENV_FILE:-/etc/vtv/env}"
KEEP_DAYS="${BACKUP_KEEP_DAYS:-14}"

# --check: the structural self-test (the mutation target for the plan's m3).
if [ "${1:-}" = "--check" ]; then
  grep -q '^set -euo pipefail' "$SELF" || { echo "FAIL: set -euo pipefail missing"; exit 1; }
  grep -q 'pg_dump' "$SELF" || { echo "FAIL: no pg_dump line"; exit 1; }
  grep -q -- '--format=custom' "$SELF" || { echo "FAIL: dump is not custom format"; exit 1; }
  grep -q 'rclone rcat' "$SELF" || { echo "FAIL: no rclone upload"; exit 1; }
  grep -q -- '--min-age' "$SELF" || { echo "FAIL: no retention prune"; exit 1; }
  grep -q 'if \[ "\$SIZE" -eq 0 \]' "$SELF" || { echo "FAIL: no zero-byte guard"; exit 1; }
  echo "backup.sh structure OK"
  exit 0
fi

# Load the secrets. The env file is root:vtv 0640 on the host; nothing here echoes it.
if [ ! -r "$ENV_FILE" ]; then
  echo "error: env file not readable: $ENV_FILE" >&2
  exit 2
fi
set -a
# shellcheck disable=SC1090
. "$ENV_FILE"
set +a

for var in DATABASE_URL R2_BACKUP_REMOTE R2_BACKUP_BUCKET; do
  if [ -z "${!var:-}" ]; then
    echo "error: missing $var (set it in $ENV_FILE)" >&2
    exit 2
  fi
done

STAMP="$(date -u +%Y%m%dT%H%M%SZ)"
TMP="$(mktemp /tmp/vtv-backup.XXXXXX.dump)"
trap 'rm -f "$TMP"' EXIT

echo "==> pg_dump -> $TMP"
pg_dump "$DATABASE_URL" --format=custom --file "$TMP"

# A zero-byte dump would silently become the only restore point: fail the unit instead.
SIZE="$(wc -c < "$TMP")"
if [ "$SIZE" -eq 0 ]; then
  echo "FAIL: ZERO BYTE dump; refusing to upload" >&2
  exit 1
fi
echo "==> dump size $SIZE bytes"

OBJECT="$R2_BACKUP_REMOTE:$R2_BACKUP_BUCKET/hub-$STAMP.dump"
echo "==> upload -> $OBJECT"
rclone rcat "$OBJECT" < "$TMP"

echo "==> prune objects older than ${KEEP_DAYS}d"
rclone delete "$R2_BACKUP_REMOTE:$R2_BACKUP_BUCKET" --min-age "${KEEP_DAYS}d"

echo "==> backup done: $OBJECT"
