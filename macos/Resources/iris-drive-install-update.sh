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

signature_team_identifier() {
    /usr/bin/codesign -dv --verbose=4 "$1" 2>&1 \
        | /usr/bin/sed -n 's/^TeamIdentifier=//p' \
        | /usr/bin/head -n 1
}

entitlement_value() {
    bundle="$1"
    key_path="$2"
    /usr/bin/codesign -d --entitlements :- "$bundle" 2>/dev/null \
        | /usr/bin/plutil -extract "$key_path" raw -o - - 2>/dev/null
}

validate_file_provider_app() {
    app="$1"
    extension="$app/Contents/PlugIns/IrisDriveFileProvider.appex"

    /usr/bin/codesign --verify --deep --strict "$app" || {
        echo "iris-drive updater: downloaded app signature is invalid"
        return 1
    }
    [ "$(/usr/libexec/PlistBuddy -c 'Print :CFBundleIdentifier' "$app/Contents/Info.plist" 2>/dev/null)" = "to.iris.drive.macos" ] || {
        echo "iris-drive updater: downloaded app has the wrong bundle identifier"
        return 1
    }
    [ -d "$extension" ] || {
        echo "iris-drive updater: downloaded app is missing its File Provider extension"
        return 1
    }
    [ "$(/usr/libexec/PlistBuddy -c 'Print :CFBundleIdentifier' "$extension/Contents/Info.plist" 2>/dev/null)" = "to.iris.drive.macos.FileProvider" ] || {
        echo "iris-drive updater: downloaded File Provider has the wrong bundle identifier"
        return 1
    }

    app_team="$(signature_team_identifier "$app")"
    extension_team="$(signature_team_identifier "$extension")"
    [ -n "$app_team" ] && [ "$app_team" != "not set" ] || {
        echo "iris-drive updater: downloaded app is ad-hoc signed"
        return 1
    }
    [ "$extension_team" = "$app_team" ] || {
        echo "iris-drive updater: app and File Provider signing teams differ"
        return 1
    }

    expected_group="$app_team.to.iris.drive"
    [ "$(entitlement_value "$app" 'com\.apple\.security\.app-sandbox')" = "true" ] || {
        echo "iris-drive updater: downloaded app is missing its sandbox entitlement"
        return 1
    }
    [ "$(entitlement_value "$extension" 'com\.apple\.security\.app-sandbox')" = "true" ] || {
        echo "iris-drive updater: downloaded File Provider is missing its sandbox entitlement"
        return 1
    }
    [ "$(entitlement_value "$app" 'com\.apple\.security\.application-groups.0')" = "$expected_group" ] || {
        echo "iris-drive updater: downloaded app is missing its File Provider app group"
        return 1
    }
    [ "$(entitlement_value "$extension" 'com\.apple\.security\.application-groups.0')" = "$expected_group" ] || {
        echo "iris-drive updater: downloaded File Provider is missing its app group"
        return 1
    }

    current_team="$(signature_team_identifier "$current_app")"
    if [ -n "$current_team" ] && [ "$current_team" != "not set" ] && [ "$current_team" != "$app_team" ]; then
        echo "iris-drive updater: downloaded app signing team differs from the installed app"
        return 1
    fi
}

cleanup_stage() {
    rm -rf "$stage_app" 2>/dev/null || true
}
trap cleanup_stage EXIT HUP INT TERM

echo "iris-drive updater: staging $new_app beside $current_app"
rm -rf "$stage_app" "$backup_app"
/usr/bin/ditto "$new_app" "$stage_app"
validate_file_provider_app "$stage_app"

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
