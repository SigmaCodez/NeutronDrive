#!/usr/bin/env bash
# Only changes this helper's GIO objects. The mount and other processes are untouched.
set -euo pipefail
nd_library=$(realpath "$1")
nd_mount=$2
nd_test=$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)
nd_state=$(mktemp -d /tmp/nd-volume-test-XXXXXX)
trap 'rm -rf -- "$nd_state"' EXIT
cc "$nd_test/volume.c" -o "$nd_state/check" $(pkg-config --cflags --libs gio-2.0 gmodule-2.0)
mkdir -p "$nd_state/neutrondrive/ondemand-service"
python3 - "$nd_state/neutrondrive/ondemand-service/mount.json" "$nd_mount" <<'PY'
import json,sys
with open(sys.argv[1],'w') as f:json.dump({'mountpoint':sys.argv[2]},f)
PY
XDG_STATE_HOME="$nd_state" "$nd_state/check" "$nd_library" "$nd_mount" "$nd_state/neutrondrive/ondemand-service/mount.json"
