#!/usr/bin/env bash
set -euo pipefail
nd_binary=$(realpath "$1")
nd_tests=$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)
nd_work=$(mktemp -d /tmp/nd-tray-check-XXXXXX)
nd_server=
cleanup() {
    local result=$?
    if [[ -n $nd_server ]]; then kill "$nd_server" 2>/dev/null || true; wait "$nd_server" 2>/dev/null || true; fi
    if [[ $result != 0 ]]; then cat "$nd_work/xvfb.log" >&2; fi
    rm -rf -- "$nd_work"
}
trap cleanup EXIT
cc "$nd_tests/tray-window-control.c" -o "$nd_work/window-control" $(pkg-config --cflags --libs x11)
Xvfb -displayfd 3 -screen 0 1280x800x24 -nolisten tcp 3> "$nd_work/display" > "$nd_work/xvfb.log" 2>&1 &
nd_server=$!
for ((n=0;n<100;n++)); do [[ -s $nd_work/display ]] && break; kill -0 "$nd_server"; sleep .05; done
[[ -s $nd_work/display ]]
export DISPLAY=:$(cat "$nd_work/display")
unset WAYLAND_DISPLAY
dbus-run-session --config-file="$nd_tests/session.conf" -- /usr/bin/python3 "$nd_tests/check-tray-autostart.py" "$nd_binary" "$nd_work/window-control"
