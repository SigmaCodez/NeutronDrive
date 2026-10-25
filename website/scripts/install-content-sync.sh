#!/usr/bin/env bash
set -Eeuo pipefail
cd "$(dirname "$0")/.."
DEST=/usr/local/lib/neutrondrive-content
if ! id neutrondrive-content >/dev/null 2>&1; then
  useradd --system --no-create-home --home-dir /nonexistent --shell /usr/sbin/nologin neutrondrive-content
fi
systemctl stop neutrondrive-content.timer neutrondrive-content.service 2>/dev/null || true
install -d -m 755 "$DEST/seed/screenshots"
install -m 644 scripts/sync-server-content.py "$DEST/sync-server-content.py"
install -m 644 public/content-version.json public/website-content.json "$DEST/seed/"
install -m 644 public/screenshots/*.png "$DEST/seed/screenshots/"
install -m 644 scripts/systemd/neutrondrive-content.{service,timer} /etc/systemd/system/
systemctl daemon-reload
# Bootstrap verified content before mounting it into the website.
systemctl start neutrondrive-content.service
systemctl enable --now neutrondrive-content.timer
systemctl is-active --quiet neutrondrive-content.timer
