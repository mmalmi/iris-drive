#!/usr/bin/env bash

set -Eeuo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
INSTALL_HELPER="$ROOT/macos/Resources/iris-drive-install-update.sh"

if [[ "$(uname -s)" != "Darwin" ]]; then
  printf 'MACOS_UPDATE_INSTALL_TEST_SKIPPED\n'
  exit 0
fi

[[ -f "$INSTALL_HELPER" ]] || {
  printf 'missing macOS update install helper: %s\n' "$INSTALL_HELPER" >&2
  exit 1
}

WORK_DIR="$(mktemp -d -t iris-drive-update-install-test.XXXXXX)"
trap 'rm -rf "$WORK_DIR"' EXIT

CURRENT_APP="$WORK_DIR/Applications/Iris Drive.app"
NEW_APP="$WORK_DIR/download/Iris Drive.app"
LOG_PATH="$WORK_DIR/install.log"

write_app() {
  local app="$1"
  local marker="$2"
  local extension="$app/Contents/PlugIns/IrisDriveFileProvider.appex"
  mkdir -p "$app/Contents/MacOS" "$app/Contents/Resources" "$extension/Contents/MacOS"
  printf '%s\n' "$marker" >"$app/Contents/Resources/version-marker"
  printf '%s\n' \
    '<?xml version="1.0" encoding="UTF-8"?>' \
    '<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">' \
    '<plist version="1.0"><dict>' \
    '<key>CFBundleIdentifier</key><string>to.iris.drive.macos</string>' \
    '<key>CFBundleExecutable</key><string>Iris Drive</string>' \
    '<key>CFBundlePackageType</key><string>APPL</string>' \
    '</dict></plist>' >"$app/Contents/Info.plist"
  printf '%s\n' \
    '<?xml version="1.0" encoding="UTF-8"?>' \
    '<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">' \
    '<plist version="1.0"><dict>' \
    '<key>CFBundleIdentifier</key><string>to.iris.drive.macos.FileProvider</string>' \
    '<key>CFBundleExecutable</key><string>IrisDriveFileProvider</string>' \
    '<key>CFBundlePackageType</key><string>XPC!</string>' \
    '</dict></plist>' >"$extension/Contents/Info.plist"
  cp /usr/bin/true "$app/Contents/MacOS/Iris Drive"
  cp /usr/bin/true "$extension/Contents/MacOS/IrisDriveFileProvider"
  chmod 755 "$app/Contents/MacOS/Iris Drive"
  chmod 755 "$extension/Contents/MacOS/IrisDriveFileProvider"
}

sign_app() {
  local app="$1"
  local identity="$2"
  local include_entitlements="$3"
  local extension="$app/Contents/PlugIns/IrisDriveFileProvider.appex"
  local sign_args=(--force --timestamp=none --options runtime --sign "$identity")

  codesign "${sign_args[@]}" "$extension" >/dev/null 2>&1
  codesign "${sign_args[@]}" "$app" >/dev/null 2>&1
  if [[ "$include_entitlements" != "yes" ]]; then
    return
  fi

  local team
  team="$(codesign -dv --verbose=4 "$app" 2>&1 \
    | sed -n 's/^TeamIdentifier=//p' \
    | head -n 1)"
  [[ -n "$team" && "$team" != "not set" ]] || {
    printf 'test signing identity did not produce a TeamIdentifier\n' >&2
    exit 1
  }
  local entitlements="$WORK_DIR/entitlements.plist"
  printf '%s\n' \
    '<?xml version="1.0" encoding="UTF-8"?>' \
    '<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">' \
    "<plist version=\"1.0\"><dict><key>com.apple.security.app-sandbox</key><true/><key>com.apple.security.application-groups</key><array><string>$team.to.iris.drive</string></array></dict></plist>" \
    >"$entitlements"
  codesign "${sign_args[@]}" --entitlements "$entitlements" "$extension" >/dev/null 2>&1
  codesign "${sign_args[@]}" --entitlements "$entitlements" "$app" >/dev/null 2>&1
}

make_ad_hoc_app() {
  local app="$1"
  local marker="$2"
  write_app "$app" "$marker"
  sign_app "$app" - no
}

make_developer_id_app() {
  local app="$1"
  local marker="$2"
  local identity="$3"
  local include_entitlements="$4"
  write_app "$app" "$marker"
  sign_app "$app" "$identity" "$include_entitlements"
}

make_ad_hoc_app "$CURRENT_APP" "old"
make_ad_hoc_app "$NEW_APP" "ad-hoc"

if /bin/sh "$INSTALL_HELPER" "$CURRENT_APP" "$NEW_APP" "$LOG_PATH"; then
  printf 'installer unexpectedly accepted an ad-hoc app without File Provider entitlements\n' >&2
  exit 1
fi

[[ "$(cat "$CURRENT_APP/Contents/Resources/version-marker")" == "old" ]]
if find "$WORK_DIR/Applications" -maxdepth 1 \
    \( -name '.iris-drive-update-*.app' -o -name '.iris-drive-update-backup-*.app' \) \
    -print -quit | grep -q .; then
  printf 'installer left a staging or backup bundle behind\n' >&2
  exit 1
fi
grep -Fq "iris-drive updater: downloaded app is ad-hoc signed" "$LOG_PATH"

SIGNING_IDENTITY="${IRIS_DRIVE_MACOS_TEST_SIGNING_IDENTITY:-$(
  security find-identity -v -p codesigning 2>/dev/null \
    | sed -n 's/.*"\(Developer ID Application:[^"]*\)"/\1/p' \
    | head -n 1
)}"
if [[ -n "$SIGNING_IDENTITY" ]]; then
  rm -rf "$NEW_APP"
  make_developer_id_app "$NEW_APP" "missing-entitlements" "$SIGNING_IDENTITY" no
  if /bin/sh "$INSTALL_HELPER" "$CURRENT_APP" "$NEW_APP" "$LOG_PATH"; then
    printf 'installer unexpectedly accepted an app without File Provider entitlements\n' >&2
    exit 1
  fi
  [[ "$(cat "$CURRENT_APP/Contents/Resources/version-marker")" == "old" ]]
  grep -Fq "iris-drive updater: downloaded app is missing its sandbox entitlement" "$LOG_PATH"

  rm -rf "$NEW_APP"
  make_developer_id_app "$NEW_APP" "new" "$SIGNING_IDENTITY" yes
  /bin/sh "$INSTALL_HELPER" "$CURRENT_APP" "$NEW_APP" "$LOG_PATH"
  [[ "$(cat "$CURRENT_APP/Contents/Resources/version-marker")" == "new" ]]
  grep -Fq "iris-drive updater: installed $CURRENT_APP" "$LOG_PATH"
else
  printf 'MACOS_UPDATE_INSTALL_SIGNED_PATH_SKIPPED (no Developer ID identity)\n'
fi

if /bin/sh "$INSTALL_HELPER" \
    "$CURRENT_APP" \
    "$WORK_DIR/missing/Iris Drive.app" \
    "$LOG_PATH"; then
  printf 'installer unexpectedly accepted a missing update bundle\n' >&2
  exit 1
fi

expected_marker="$([[ -n "$SIGNING_IDENTITY" ]] && printf new || printf old)"
[[ "$(cat "$CURRENT_APP/Contents/Resources/version-marker")" == "$expected_marker" ]]

if [[ -n "$SIGNING_IDENTITY" ]]; then
  chmod 555 "$WORK_DIR/Applications"
  if /bin/sh "$INSTALL_HELPER" "$CURRENT_APP" "$NEW_APP" "$LOG_PATH"; then
    chmod 755 "$WORK_DIR/Applications"
    printf 'installer unexpectedly replaced an app in a read-only directory\n' >&2
    exit 1
  fi
  chmod 755 "$WORK_DIR/Applications"
  [[ "$(cat "$CURRENT_APP/Contents/Resources/version-marker")" == "new" ]]
fi

printf 'MACOS_UPDATE_INSTALL_TEST_OK\n'
