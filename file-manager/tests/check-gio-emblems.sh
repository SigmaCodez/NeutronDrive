#!/usr/bin/env bash
set -euo pipefail
nd_binary=$(realpath "$1")
nd_root=$(mktemp -d /tmp/nd-gio-emblems-XXXXXX)
trap 'rm -rf -- "$nd_root"' EXIT
mkdir -m 700 "$nd_root/runtime"
nd_test=$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)
# No user metadata or account is accessed by this separate bus and temporary HOME.
env XDG_RUNTIME_DIR="$nd_root/runtime" HOME="$nd_root" XDG_CONFIG_HOME="$nd_root/config" XDG_STATE_HOME="$nd_root/state" XDG_DATA_HOME="$nd_root/data" XDG_CACHE_HOME="$nd_root/cache" dbus-run-session -- /usr/bin/python3 "$nd_test/gio-emblems.py" "$nd_binary"
