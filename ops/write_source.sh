#!/usr/bin/env bash
# Write hub/SOURCE.txt (the AGPL source offer) for the current checkout.
#
# Called by ops/deploy.sh after its checkout, and by the manual redeploy sequence in
# ops/README.md. Any path that runs `git checkout` must end with this, or the offer reverts
# to the repo placeholder and the served source is unnamed (DEBT-032).
set -euo pipefail
cd "$(dirname "$0")/.."
REV="$(git rev-parse HEAD)"
REPO_URL="$(git remote get-url origin | sed -e 's#git@github.com:#https://github.com/#' -e 's#\.git$##')"
cat > hub/SOURCE.txt <<EOF
vtradingview hosted service
commit: $REV
built:  $(date -u +%Y-%m-%dT%H:%M:%SZ)
source: $REPO_URL
Licence: AGPL-3.0-only. This file names the exact revision the service runs; the full source
of that revision lives at the URL above.
EOF
echo "hub/SOURCE.txt: offer written for $REV"
