#!/usr/bin/env bash
set -euo pipefail
nd_binary=$1
nd_root=$(mktemp -d)
export XDG_STATE_HOME="$nd_root/state"
nd_pid=
cleanup() {
    if [[ -n $nd_pid ]] && kill -0 "$nd_pid" 2>/dev/null; then
        kill -TERM "$nd_pid"
        wait "$nd_pid" || true
    fi
    if /usr/bin/mountpoint -q "$nd_root/mount"; then /usr/bin/fusermount3 -u "$nd_root/mount"; fi
    rm -rf -- "$nd_root"
}
trap cleanup EXIT
mkdir "$nd_root/helpers"
ln -s /usr/bin/fusermount3 "$nd_root/helpers/fusermount3"
cat > "$nd_root/sdk" <<'FIXTURE'
#!/bin/sh
for last do :; done
file='{"uid":"data","name":{"ok":true,"value":"data"},"type":"file","activeRevision":{"uid":"r1","claimedSize":3,"claimedDigests":{"sha1":"a9993e364706816aba3e25717850c26c9cd0d89d","sha1Verified":true}}}'
if [ "$2" = download ]; then printf abc > "$last/data"; printf '%s\n' '{}'; exit 0; fi
if [ "$2" = list ]; then printf '[%s]\n' "$file"; exit 0; fi
if [ "$last" = /my-files ]; then printf '%s\n' '{"uid":"root","name":{"ok":true,"value":"root"},"type":"folder"}'; exit 0; fi
printf '%s\n' "$file"
FIXTURE
chmod 700 "$nd_root/sdk"
for nd_mode in online offline; do
    nd_options=()
    [[ $nd_mode != offline ]] || nd_options+=(--offline)
    /usr/bin/env -u DISPLAY -u WAYLAND_DISPLAY -u SNAP -u FLATPAK_ID \
        PATH="$nd_root/helpers" QT_QPA_PLATFORM=invalid-platform XDG_STATE_HOME="$nd_root/state" \
        "$nd_binary" --mount-foreground "$nd_root/mount" --cli "$nd_root/sdk" "${nd_options[@]}" >"$nd_root/mount.log" 2>&1 &
    nd_pid=$!
    nd_ready=false
    for ((nd_try=0;nd_try<100;nd_try++)); do
        if ! kill -0 "$nd_pid" 2>/dev/null; then cat "$nd_root/mount.log"; exit 1; fi
        if /usr/bin/mountpoint -q "$nd_root/mount"; then nd_ready=true; break; fi
        /bin/sleep 0.05
    done
    [[ $nd_ready == true ]]
    [[ -d "$nd_root/state/neutrondrive/ondemand-foreground" ]]
    [[ $(cat "$nd_root/mount/data") == abc ]]
    kill -TERM "$nd_pid"
    wait "$nd_pid"
    nd_pid=
    if /usr/bin/mountpoint -q "$nd_root/mount"; then echo 'Foreground mount survived shutdown' >&2; exit 1; fi
done
echo NATIVE_FOREGROUND_MOUNT_OK
