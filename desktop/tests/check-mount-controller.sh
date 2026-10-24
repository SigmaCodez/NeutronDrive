#!/usr/bin/env bash
set -euo pipefail
nd_binary=$1
nd_catalog=$2
nd_root=$(mktemp -d)
export XDG_STATE_HOME="$nd_root/state"
export XDG_CONFIG_HOME="$nd_root/config"
export ND_SDK_DOWNLOADS="$nd_root/downloads"
nd_pid=
cleanup() {
    if [[ -n $nd_pid ]] && kill -0 "$nd_pid" 2>/dev/null; then
        kill -TERM "$nd_pid"
        wait "$nd_pid" || true
    fi
    if /usr/bin/mountpoint -q "$nd_root/mount"; then
        /usr/bin/fusermount3 -u "$nd_root/mount" || { echo "Test mount retained at $nd_root" >&2; return 1; }
    fi
    rm -rf -- "$nd_root"
}
trap cleanup EXIT
mkdir "$nd_root/helpers"
ln -s /usr/bin/fusermount3 "$nd_root/helpers/fusermount3"
cat > "$nd_root/sdk" <<'FIXTURE'
#!/bin/sh
for last do :; done
if [ "$1" = events ]; then printf '%s\n' '{"protocol":1,"type":"batch","roots":[{"path":"/my-files","uid":"root","scopeId":"scope"}],"events":[],"cursors":{"scope":"next"},"resets":[]}'; exit 0; fi
file='{"uid":"data","name":{"ok":true,"value":"data"},"type":"file","activeRevision":{"uid":"r1","claimedSize":3,"claimedDigests":{"sha1":"a9993e364706816aba3e25717850c26c9cd0d89d","sha1Verified":true}}}'
if [ "$2" = download ]; then printf '%s\n' download >> "$ND_SDK_DOWNLOADS"; printf abc > "$last/data"; printf '%s\n' '{}'; exit 0; fi
if [ "$2" = list ]; then printf '[%s]\n' "$file"; exit 0; fi
if [ "$last" = /my-files ]; then printf '%s\n' '{"uid":"root","name":{"ok":true,"value":"root"},"type":"folder"}'; exit 0; fi
printf '%s\n' "$file"
FIXTURE
chmod 700 "$nd_root/sdk"
/usr/bin/env -u SNAP -u FLATPAK_ID PATH="$nd_root/helpers" "$nd_binary" --desktop-service --cli "$nd_root/sdk" >"$nd_root/service.log" 2>&1 &
nd_pid=$!
nd_ready=false
for ((nd_try=0;nd_try<100;nd_try++)); do
    if ! kill -0 "$nd_pid" 2>/dev/null; then cat "$nd_root/service.log"; exit 1; fi
    nd_owner=$(/usr/bin/gdbus call --session --dest org.freedesktop.DBus --object-path /org/freedesktop/DBus \
        --method org.freedesktop.DBus.NameHasOwner io.github.SigmaCodez.NeutronDrive.Service)
    if [[ $nd_owner == '(true,)' ]]; then nd_ready=true; break; fi
    /bin/sleep 0.05
done
[[ $nd_ready == true ]]
[[ -d "$nd_root/state/neutrondrive/ondemand-service" ]]
/usr/bin/env -u SNAP -u FLATPAK_ID PATH="$nd_root/helpers" "$nd_binary" --check-mount "$nd_catalog" "$nd_root/sdk" "$nd_root/mount"
if /usr/bin/mountpoint -q "$nd_root/mount"; then echo 'QML left test mount active' >&2; exit 1; fi
mapfile -t nd_objects < <(find "$nd_root/state/neutrondrive/ondemand-service" -path '*/objects/*' -type f)
[[ ${#nd_objects[@]} == 1 ]]
[[ $(cat "${nd_objects[0]}") == abc ]]
[[ $(wc -l < "$ND_SDK_DOWNLOADS") == 2 ]]
kill -TERM "$nd_pid"
wait "$nd_pid"
nd_pid=
[[ ! -e "$nd_root/config/autostart/io.github.SigmaCodez.NeutronDrive.desktop" ]]
echo NATIVE_MOUNT_CONTROLLER_CLEANUP_OK
