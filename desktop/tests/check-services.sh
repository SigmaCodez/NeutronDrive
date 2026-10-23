#!/usr/bin/env bash
set -euo pipefail
nd_binary=$1
nd_root=$(mktemp -d)
export XDG_STATE_HOME="$nd_root/state"
nd_pid=
nd_background_pid=
cleanup() {
    if [[ -n $nd_background_pid ]] && kill -0 "$nd_background_pid" 2>/dev/null; then
        kill -TERM "$nd_background_pid"
        wait "$nd_background_pid" || true
    fi
    if [[ -n $nd_pid ]] && kill -0 "$nd_pid" 2>/dev/null; then
        kill -TERM "$nd_pid"
        wait "$nd_pid" || true
    fi
    rm -rf -- "$nd_root"
}
trap cleanup EXIT
set +e
/usr/bin/env -u DISPLAY -u WAYLAND_DISPLAY PATH=/nonexistent QT_QPA_PLATFORM=invalid-platform \
    "$nd_binary" --open-uri 'file:///tmp/not-a-browser-url' >"$nd_root/browser.log" 2>&1
nd_browser_status=$?
set -e
[[ $nd_browser_status == 1 ]]
grep -q 'Only valid HTTP(S)' "$nd_root/browser.log"
echo NATIVE_HEADLESS_BROWSER_VALIDATION_OK
# An invalid platform would abort a QGuiApplication. Headless modes must not create one.
/usr/bin/env -u DISPLAY -u WAYLAND_DISPLAY -u SNAP -u FLATPAK_ID \
    PATH=/nonexistent QT_QPA_PLATFORM=invalid-platform XDG_STATE_HOME="$nd_root/state" \
    "$nd_binary" --desktop-service --cli /missing-synthetic-sdk >"$nd_root/service.log" 2>&1 &
nd_pid=$!
nd_ready=false
for ((nd_try=0;nd_try<100;nd_try++)); do
    if ! kill -0 "$nd_pid" 2>/dev/null; then cat "$nd_root/service.log"; exit 1; fi
    nd_owner=$(/usr/bin/gdbus call --session --dest org.freedesktop.DBus \
        --object-path /org/freedesktop/DBus --method org.freedesktop.DBus.NameHasOwner \
        io.github.SigmaCodez.NeutronDrive.Service)
    if [[ $nd_owner == '(true,)' ]]; then nd_ready=true; break; fi
    /bin/sleep 0.05
done
[[ $nd_ready == true ]]
[[ -d "$nd_root/state/neutrondrive/ondemand-service" ]]
nd_status=$(/usr/bin/gdbus call --session --dest io.github.SigmaCodez.NeutronDrive.Service \
    --object-path /io/github/SigmaCodez/NeutronDrive/Service \
    --method io.github.SigmaCodez.NeutronDrive.Service.Request '{"action":"status"}')
[[ $nd_status == *'"ok":true'* ]]
nd_cli_status=$(/usr/bin/env PATH=/nonexistent QT_QPA_PLATFORM=invalid-platform XDG_STATE_HOME="$nd_root/state" \
    "$nd_binary" --service-status --cli /missing-synthetic-sdk)
[[ $nd_cli_status == *'"mounted": false'* ]]
/usr/bin/env -u DISPLAY -u WAYLAND_DISPLAY -u SNAP -u FLATPAK_ID \
    PATH=/nonexistent QT_QPA_PLATFORM=invalid-platform XDG_STATE_HOME="$nd_root/state" \
    "$nd_binary" --background --state-dir "$nd_root/combined" --cli /missing-synthetic-sdk >"$nd_root/background.log" 2>&1 &
nd_background_pid=$!
nd_ready=false
for ((nd_try=0;nd_try<100;nd_try++)); do
    if ! kill -0 "$nd_background_pid" 2>/dev/null; then cat "$nd_root/background.log"; exit 1; fi
    if [[ -S "$nd_root/combined/control.sock" ]]; then nd_ready=true; break; fi
    /bin/sleep 0.05
done
[[ $nd_ready == true ]]
/usr/bin/env PATH=/nonexistent QT_QPA_PLATFORM=invalid-platform XDG_STATE_HOME="$nd_root/state" \
    "$nd_binary" --stop-background --state-dir "$nd_root/combined"
wait "$nd_background_pid"
nd_background_pid=
[[ ! -e "$nd_root/combined/control.sock" ]]
[[ ! -e "$nd_root/combined/daemon.pid" ]]
kill -TERM "$nd_pid"
wait "$nd_pid"
nd_pid=
/usr/bin/env -u DISPLAY -u WAYLAND_DISPLAY PATH=/nonexistent QT_QPA_PLATFORM=invalid-platform \
    XDG_STATE_HOME="$nd_root/state" "$nd_binary" --daemon --state-dir "$nd_root/background" \
    --cli /missing-synthetic-sdk >"$nd_root/daemon.log" 2>&1 &
nd_pid=$!
nd_ready=false
for ((nd_try=0;nd_try<100;nd_try++)); do
    if ! kill -0 "$nd_pid" 2>/dev/null; then cat "$nd_root/daemon.log"; exit 1; fi
    if [[ -S "$nd_root/background/control.sock" ]]; then nd_ready=true; break; fi
    /bin/sleep 0.05
done
[[ $nd_ready == true ]]
nd_sync_status=$(/usr/bin/env PATH=/nonexistent QT_QPA_PLATFORM=invalid-platform XDG_STATE_HOME="$nd_root/state" \
    "$nd_binary" --sync-status --state-dir "$nd_root/background")
[[ $nd_sync_status == *'"running": true'* ]]
kill -TERM "$nd_pid"
wait "$nd_pid"
nd_pid=
[[ ! -e "$nd_root/background/control.sock" ]]
[[ ! -e "$nd_root/background/daemon.pid" ]]
nd_sync_status=$(/usr/bin/env PATH=/nonexistent QT_QPA_PLATFORM=invalid-platform XDG_STATE_HOME="$nd_root/state" \
    "$nd_binary" --sync-status --state-dir "$nd_root/background")
[[ $nd_sync_status == *'"running": false'* ]]
/usr/bin/env PATH=/nonexistent QT_QPA_PLATFORM=invalid-platform XDG_STATE_HOME="$nd_root/state" \
    "$nd_binary" --stop-background --state-dir "$nd_root/background"
cat > "$nd_root/version-sdk" <<'FIXTURE'
#!/bin/sh
[ "$1" = version ] || exit 1
if [ -n "${ND_DIAG_PID:-}" ]; then
    printf '%s\n' "$$" > "$ND_DIAG_PID"
    exec /bin/sleep 60
fi
printf '%s\n' 'Synthetic SDK version'
FIXTURE
chmod 700 "$nd_root/version-sdk"
nd_diagnose=$(/usr/bin/env PATH=/nonexistent QT_QPA_PLATFORM=invalid-platform \
    "$nd_binary" --diagnose --cli "$nd_root/version-sdk")
[[ $nd_diagnose == *'"backend_found": true'* ]]
[[ $nd_diagnose == *'"implementation": "rust"'* ]]
[[ $nd_diagnose == *'Synthetic SDK version'* ]]
/usr/bin/env PATH=/nonexistent QT_QPA_PLATFORM=invalid-platform ND_DIAG_PID="$nd_root/diagnose.pid" \
    "$nd_binary" --diagnose --cli "$nd_root/version-sdk" > "$nd_root/diagnose.json" &
nd_pid=$!
for ((nd_try=0;nd_try<100;nd_try++)); do
    [[ ! -s "$nd_root/diagnose.pid" ]] || break
    /bin/sleep 0.05
done
[[ -s "$nd_root/diagnose.pid" ]]
kill -TERM "$nd_pid"
if wait "$nd_pid"; then exit 1; else nd_exit=$?; fi
[[ $nd_exit == 1 ]]
nd_pid=
read -r nd_sdk_pid < "$nd_root/diagnose.pid"
if kill -0 "$nd_sdk_pid" 2>/dev/null; then echo 'Diagnostic SDK survived cancellation' >&2; exit 1; fi
if /usr/bin/env PATH=/nonexistent QT_QPA_PLATFORM=invalid-platform "$nd_binary" --diagnose \
    --cli /missing-synthetic-sdk > "$nd_root/missing.json"; then exit 1; fi
nd_version=$(/usr/bin/env PATH=/nonexistent QT_QPA_PLATFORM=invalid-platform "$nd_binary" --version)
[[ $nd_version == 'NeutronDrive '* ]]
echo NATIVE_HEADLESS_SERVICES_OK
