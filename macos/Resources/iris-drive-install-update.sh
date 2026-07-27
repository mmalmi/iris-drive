#!/bin/sh

set -eu

current_app="$1"
new_app="$2"
log_path="${3:-${TMPDIR:-/tmp}/iris-drive-install-update.log}"

exec >>"$log_path" 2>&1

if [ ! -d "$current_app" ]; then
    echo "iris-drive updater: current app is missing: $current_app"
    exit 1
fi
if [ ! -d "$new_app" ] || [ ! -x "$new_app/Contents/MacOS/Iris Drive" ]; then
    echo "iris-drive updater: downloaded app is incomplete: $new_app"
    exit 1
fi

parent_dir="$(dirname "$current_app")"
stage_app="$parent_dir/.iris-drive-update-$$.app"
backup_app="$parent_dir/.iris-drive-update-backup-$$.app"

cleanup_stage() {
    rm -rf "$stage_app" 2>/dev/null || true
}
trap cleanup_stage EXIT HUP INT TERM

echo "iris-drive updater: staging $new_app beside $current_app"
rm -rf "$stage_app" "$backup_app"
/usr/bin/ditto "$new_app" "$stage_app"
/usr/bin/codesign --verify --deep --strict "$stage_app"

if [ "$(id -u)" -eq 0 ]; then
    current_uid="$(/usr/bin/stat -f '%u' "$current_app")"
    current_gid="$(/usr/bin/stat -f '%g' "$current_app")"
    /usr/sbin/chown -R "$current_uid:$current_gid" "$stage_app"
fi

echo "iris-drive updater: replacing $current_app"
/bin/mv "$current_app" "$backup_app"
if ! /bin/mv "$stage_app" "$current_app"; then
    echo "iris-drive updater: replacement failed; restoring previous app"
    /bin/mv "$backup_app" "$current_app"
    exit 1
fi

/usr/bin/xattr -dr com.apple.quarantine "$current_app" 2>/dev/null || true
rm -rf "$backup_app"
trap - EXIT HUP INT TERM
echo "iris-drive updater: installed $current_app"
