#!/usr/bin/env bash
# Deploy on the production VPS.
#
#   bash ops/deploy.sh                   # build from the pushed commit (rustup on the host)
#   bash ops/deploy.sh --from-mise 0.1.0 # install the prebuilt release asset instead (no build)
#   bash ops/deploy.sh [commit]          # build a specific commit/tag
#
# Both modes keep the previous binary for rollback, write hub/SOURCE.txt (the AGPL source offer),
# restart the service and smoke the public origin. Never pushes.
set -euo pipefail

MODE="build"
if [ "${1:-}" = "--from-mise" ]; then
  MODE="mise"
  shift
  VERSION="${1:-}"
  [ -n "$VERSION" ] || { echo "usage: bash ops/deploy.sh --from-mise <version>" >&2; exit 2; }
  shift || true
fi

cd /opt/vtv/repo
COMMIT="${1:-$(git rev-parse --abbrev-ref HEAD)}"
git fetch origin --tags
git checkout --detach "origin/${COMMIT#origin/}" 2>/dev/null || git checkout --detach "$COMMIT"
REV="$(git rev-parse HEAD)"

if [ "$MODE" = "mise" ]; then
  # The binary comes from the release asset (mise.toml [tools."http:vtv"]): the host needs no
  # rust toolchain and no build time. `mise use` pins the version in mise.toml; `mise install`
  # fetches it; `mise which` names the installed path.
  echo "==> mise: installing vtv@$VERSION (prebuilt release asset)"
  if grep -q '^\[tools\."http:vtv"\]' mise.toml; then
    mise use "vtv@$VERSION" >/dev/null
  else
    mise install "http:vtv@$VERSION" >/dev/null
  fi
  BUILT="$(mise which vtv)"
  [ -x "$BUILT" ] || { echo "mise did not install an executable for vtv@$VERSION ($BUILT)" >&2; exit 1; }
  echo "    $BUILT"
else
  echo "==> cargo build --release ($REV)"
  cargo build --release -p hub-server
  BUILT="target/release/hub-server"
fi

echo "==> swapping binaries (old kept for rollback)"
if [ -f /opt/vtv/hub-server ]; then
  cp /opt/vtv/hub-server /opt/vtv/hub-server.old
fi
install -m 755 "$BUILT" /opt/vtv/hub-server

echo "==> writing hub/SOURCE.txt (the AGPL offer: this exact commit is the served source)"
REPO_URL="$(git remote get-url origin | sed -e 's#git@github.com:#https://github.com/#' -e 's#\.git$##')"
cat > /opt/vtv/repo/hub/SOURCE.txt <<EOF
vtradingview hosted service
commit: $REV
built:  $(date -u +%Y-%m-%dT%H:%M:%SZ)
source: $REPO_URL
Licence: AGPL-3.0-only. This file names the exact revision the service runs; the full source
of that revision lives at the URL above.
EOF

echo "==> restarting vtv.service"
systemctl restart vtv

echo "==> smoke: public origin /api/health"
ORIGIN="$(grep -E '^PUBLIC_ORIGIN=' /etc/vtv/env | cut -d= -f2- | tr -d '"')"
for _ in $(seq 1 30); do
  curl -fs "$ORIGIN/api/health" > /dev/null 2>&1 && break
  sleep 1
done
curl -fs "$ORIGIN/api/health" > /dev/null || { echo "deploy FAILED: /api/health did not answer"; exit 1; }
echo "health OK"

echo
echo "ROLLBACK (if needed):"
echo "  cp /opt/vtv/hub-server.old /opt/vtv/hub-server && sudo systemctl restart vtv"
echo "DEPLOY OK: $REV"
