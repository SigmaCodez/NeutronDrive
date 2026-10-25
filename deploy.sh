#!/usr/bin/env bash
# Always deploy this project's marketing site to the requested server.
set -Eeuo pipefail
ROOT=$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)
HOST=161.97.140.89
# Host port 31219 is configured in website/scripts/deploy-remote.sh.
MODE=${1:-}
if [[ $# -gt 1 || ( -n "$MODE" && "$MODE" != --configure-proxy ) ]]; then
  echo "Usage: ./deploy.sh [--configure-proxy]" >&2; exit 2
fi
SSH_USER=${NEUTRONDRIVE_SSH_USER:-root}
SSH_KEY=${NEUTRONDRIVE_SSH_KEY:-$HOME/.ssh/SaaS01}
SSH=(ssh -F /dev/null -i "$SSH_KEY" -o IdentitiesOnly=yes -o BatchMode=yes -o StrictHostKeyChecking=yes -o ConnectTimeout=15 "$SSH_USER@$HOST")
command -v ssh >/dev/null
command -v tar >/dev/null
test -f "$ROOT/website/pnpm-lock.yaml" || { echo 'Missing website/pnpm-lock.yaml' >&2; exit 1; }
"${SSH[@]}" 'command -v docker >/dev/null && command -v python3 >/dev/null && command -v flock >/dev/null'
RELEASE="$(date -u +%Y%m%dT%H%M%SZ)-$$"
REMOTE="/opt/neutrondrive/releases/$RELEASE"
"${SSH[@]}" "umask 077; mkdir -p '$REMOTE'"
tar -C "$ROOT/website" --exclude=node_modules --exclude=.next --exclude=out --exclude=.git --exclude='.env*' --exclude=security-headers.conf -czf - . | "${SSH[@]}" "tar --no-same-owner -xzf - -C '$REMOTE'"
"${SSH[@]}" "bash '$REMOTE/scripts/deploy-remote.sh' '$MODE'"
