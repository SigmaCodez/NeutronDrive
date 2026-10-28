#!/usr/bin/env bash
# Run as root with the dedicated CI public-key file; never install a private key.
set -Eeuo pipefail
[[ $# == 1 && $EUID == 0 ]] || { echo 'Usage (root): install-release-trigger.sh PUBLIC_KEY_FILE' >&2; exit 1; }
ssh-keygen -l -f "$1" >/dev/null
[[ $(wc -l < "$1") == 1 ]] && grep -Eq '^ssh-ed25519 [A-Za-z0-9+/=]+( .*)?$' "$1"
account=neutrondrive-release
if ! id "$account" >/dev/null 2>&1; then
  useradd --system --no-create-home --home-dir /var/lib/neutrondrive-release --shell /bin/sh "$account"
fi
# Root owns the home/key so the CI identity cannot replace its forced command.
install -d -m 755 -o root -g root /var/lib/neutrondrive-release
install -d -m 755 -o root -g root /var/lib/neutrondrive-release/.ssh
printf 'restrict,command="/usr/bin/sudo -n /usr/bin/systemctl start neutrondrive-content.service" %s\n' "$(cat "$1")" > /var/lib/neutrondrive-release/.ssh/authorized_keys
chmod 644 /var/lib/neutrondrive-release/.ssh/authorized_keys
rule=$(mktemp)
trap 'rm -f "$rule"' EXIT
printf '%s ALL=(root) NOPASSWD: /usr/bin/systemctl start neutrondrive-content.service\n' "$account" > "$rule"
visudo -cf "$rule" >/dev/null
install -o root -g root -m 440 "$rule" /etc/sudoers.d/neutrondrive-release
