#!/usr/bin/env bash
# The SSH identity can only start the existing sandboxed content updater.
set -Eeuo pipefail
: "${ND_CONTENT_SYNC_SSH_KEY:?Missing dedicated release sync key}"
: "${ND_CONTENT_SYNC_KNOWN_HOSTS:?Missing pinned server host key}"
[[ $# == 1 && -f $1 ]] || { echo 'Usage: trigger-release-sync.sh RELEASE_MANIFEST' >&2; exit 1; }
credentials=$(mktemp -d)
trap 'rm -rf "$credentials"' EXIT
chmod 700 "$credentials"
printf '%s\n' "$ND_CONTENT_SYNC_SSH_KEY" > "$credentials/key"
printf '%s\n' "$ND_CONTENT_SYNC_KNOWN_HOSTS" > "$credentials/known_hosts"
chmod 600 "$credentials/key" "$credentials/known_hosts"
unset ND_CONTENT_SYNC_SSH_KEY ND_CONTENT_SYNC_KNOWN_HOSTS
for attempt in 1 2 3; do
  if ssh -F /dev/null -i "$credentials/key" -o IdentitiesOnly=yes -o BatchMode=yes \
      -o StrictHostKeyChecking=yes -o UserKnownHostsFile="$credentials/known_hosts" \
      -o ConnectTimeout=15 -o ServerAliveInterval=15 -o ServerAliveCountMax=4 \
      neutrondrive-release@161.97.140.89 && \
      python3 "$(dirname "$0")/verify-release-sync.py" "$1"; then
    exit 0
  fi
  # Also handles a timer invocation already running when the asset was uploaded.
  if [[ $attempt != 3 ]]; then sleep 5; fi
done
echo 'Immediate homepage sync failed; the server timer remains enabled as fallback.' >&2
exit 1
