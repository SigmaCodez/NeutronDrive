#!/usr/bin/env bash
set -euo pipefail
nd_binary=$(realpath "$1")
nd_root=$(mktemp -d /tmp/nd-gio-emblems-XXXXXX)
cleanup() {
    local result=$?
    # D-Bus activation children flush metadata asynchronously when the isolated
    # bus exits. Retry only our private test directory until those writes end.
    for ((attempt=0;attempt<50;attempt++)); do
        if rm -rf -- "$nd_root" 2>/dev/null; then return "$result"; fi
        sleep .1
    done
    echo "GVfs test cleanup did not finish; retained $nd_root" >&2
    return 1
}
trap cleanup EXIT
mkdir -m 700 "$nd_root/runtime"
nd_test=$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)
# No user metadata or account is accessed by this separate bus and temporary HOME.
env XDG_RUNTIME_DIR="$nd_root/runtime" HOME="$nd_root" XDG_CONFIG_HOME="$nd_root/config" XDG_STATE_HOME="$nd_root/state" XDG_DATA_HOME="$nd_root/data" XDG_CACHE_HOME="$nd_root/cache" dbus-run-session -- /usr/bin/python3 "$nd_test/gio-emblems.py" "$nd_binary"
