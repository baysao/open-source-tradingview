#!/usr/bin/env bash
# Deploy on the production VPS: fetch the pushed commit, build the release binary, keep the
# previous one for rollback, write hub/SOURCE.txt (the AGPL source offer), restart the service,
# smoke the public origin. Runs ON the host; it never pushes.
#
#   bash ops/deploy.sh [commit]      # default: origin/<current branch>
#
# Prints the rollback command at the end; keep it.

set -euo pipefail

cd /opt/vtv/repo
COMMIT="${1:-$(git rev-parse --abbrev-ref HEAD)}"
git fetch origin
git checkout --detach "origin/${COMMIT#origin/}" 2>/dev/null || git checkout --detach "$COMMIT"
REV="$(git rev-parse HEAD)"

echo "==> cargo build --release ($REV)"
cargo build --release -p hub-server

echo "==> swapping binaries (old kept for rollback)"
if [ -f /opt/vtv/hub-server ]; then
  cp /opt/vtv/hub-server /opt/vtv/hub-server.old
fi
install -m 755 target/release/hub-server /opt/vtv/hub-server

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
