#!/bin/bash
set -euo pipefail
binary=$1
catalog=$2
root=$(mktemp -d /tmp/nd-native-window-fixture-XXXXXX)
trap 'rm -rf -- "$root"' EXIT
cat > "$root/sdk" <<'SDK'
#!/bin/sh
case "$1 $2" in
 'account storage')
    printf '%s\n' '{"total":1000000,"used":250000,"free":750000}'
    ;;
 'filesystem list')
    if [ "$5" != /my-files ]; then printf '[]\n'; exit 0; fi
    printf '%s\n' '[{"uid":"folder","name":{"ok":true,"value":"Dokumente"},"type":"folder"},{"uid":"file","name":{"ok":true,"value":"Notizen.md"},"type":"file"}]'
    ;;
 'filesystem rename')
    [ "$6" = 'Meine Dateien {p0} 中文.txt' ] || exit 23
    printf 'ok\n' > "$ND_LITERAL_MARKER"
    printf '{}\n'
    ;;
 'sharing status')
    printf '%s\n' '{"urlAccess":{"url":"https://drive.proton.me/urls/synthetic?literal=%3Cb%3E"}}'
    ;;
 *) exit 24 ;;
esac
SDK
chmod 700 "$root/sdk"
/usr/bin/env -u SNAP -u FLATPAK_ID -u APPIMAGE -u APPDIR PATH=/nonexistent \
    XDG_CONFIG_HOME="$root/config" XDG_STATE_HOME="$root/state" XDG_CACHE_HOME="$root/cache" \
    ND_LITERAL_MARKER="$root/literal-value" \
    "$binary" --check-window-fixture "$catalog" "$root/sdk"
[[ $(cat "$root/literal-value") == ok ]]
echo NATIVE_WINDOW_LITERAL_SDK_ARGUMENT_OK
