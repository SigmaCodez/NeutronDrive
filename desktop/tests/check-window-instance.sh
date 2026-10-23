#!/usr/bin/env bash
set -euo pipefail
nd_binary=$1
nd_catalog=$2
nd_root=$(mktemp -d /tmp/nd-window-instance-XXXXXX)
nd_owner=
cleanup() {
    local nd_exit=$?
    if [[ -n $nd_owner ]]; then
        kill "$nd_owner" 2>/dev/null || true
        wait "$nd_owner" 2>/dev/null || true
    fi
    if [[ $nd_exit == 0 ]]; then
        rm -rf -- "$nd_root"
    else
        echo "Window-instance test failed; isolated diagnostics retained at $nd_root" >&2
    fi
}
trap cleanup EXIT
export XDG_CONFIG_HOME="$nd_root/config" XDG_STATE_HOME="$nd_root/state" XDG_CACHE_HOME="$nd_root/cache"
unset SNAP FLATPAK_ID APPIMAGE APPDIR
/usr/bin/env PATH=/nonexistent "$nd_binary" --demo > "$nd_root/owner.log" 2>&1 &
nd_owner=$!
nd_socket="$XDG_STATE_HOME/neutrondrive/qt-window-demo/window.sock"
for ((nd_n=0;nd_n<150;nd_n++)); do
    [[ -S $nd_socket ]] && break
    kill -0 "$nd_owner"
    sleep .02
done
[[ -S $nd_socket ]]
timeout 10 /usr/bin/env PATH=/nonexistent "$nd_binary" --demo > "$nd_root/second.log" 2>&1
kill -0 "$nd_owner"
# Simulate a crashed demo instance to exercise QLockFile's compatible stale-lock handling.
kill -KILL "$nd_owner"
set +e
wait "$nd_owner" 2>/dev/null
nd_status=$?
set -e
nd_owner=
[[ $nd_status == 137 && -S $nd_socket ]]
timeout 25 /usr/bin/env PATH=/nonexistent "$nd_binary" --check-window "$nd_catalog" > "$nd_root/recovered.log" 2>&1
[[ ! -e $nd_socket && ! -e ${nd_socket%/*}/window.lock ]]
echo NATIVE_WINDOW_ACTIVATION_AND_CRASH_RECOVERY_OK
