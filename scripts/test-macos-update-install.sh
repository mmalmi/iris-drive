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

make_app() {
  local app="$1"
  local marker="$2"
  mkdir -p "$app/Contents/MacOS"
  printf '%s\n' "$marker" >"$app/Contents/version-marker"
  printf '%s\n' \
    '<?xml version="1.0" encoding="UTF-8"?>' \
    '<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">' \
    '<plist version="1.0"><dict>' \
    '<key>CFBundleIdentifier</key><string>to.iris.drive.update-test</string>' \
    '<key>CFBundleExecutable</key><string>Iris Drive</string>' \
    '<key>CFBundlePackageType</key><string>APPL</string>' \
    '</dict></plist>' >"$app/Contents/Info.plist"
  printf '#!/bin/sh\nexit 0\n' >"$app/Contents/MacOS/Iris Drive"
  chmod 755 "$app/Contents/MacOS/Iris Drive"
  codesign --force --deep --sign - "$app"
}

make_app "$CURRENT_APP" "old"
make_app "$NEW_APP" "new"

/bin/sh "$INSTALL_HELPER" "$CURRENT_APP" "$NEW_APP" "$LOG_PATH"

[[ "$(cat "$CURRENT_APP/Contents/version-marker")" == "new" ]]
if find "$WORK_DIR/Applications" -maxdepth 1 \
    \( -name '.iris-drive-update-*.app' -o -name '.iris-drive-update-backup-*.app' \) \
    -print -quit | grep -q .; then
  printf 'installer left a staging or backup bundle behind\n' >&2
  exit 1
fi
grep -Fq "iris-drive updater: installed $CURRENT_APP" "$LOG_PATH"

rm -rf "$CURRENT_APP"
make_app "$CURRENT_APP" "preserved"

if /bin/sh "$INSTALL_HELPER" \
    "$CURRENT_APP" \
    "$WORK_DIR/missing/Iris Drive.app" \
    "$LOG_PATH"; then
  printf 'installer unexpectedly accepted a missing update bundle\n' >&2
  exit 1
fi

[[ "$(cat "$CURRENT_APP/Contents/version-marker")" == "preserved" ]]

rm -rf "$NEW_APP"
make_app "$NEW_APP" "blocked"
chmod 555 "$WORK_DIR/Applications"
if /bin/sh "$INSTALL_HELPER" "$CURRENT_APP" "$NEW_APP" "$LOG_PATH"; then
  chmod 755 "$WORK_DIR/Applications"
  printf 'installer unexpectedly replaced an app in a read-only directory\n' >&2
  exit 1
fi
chmod 755 "$WORK_DIR/Applications"

[[ "$(cat "$CURRENT_APP/Contents/version-marker")" == "preserved" ]]
printf 'MACOS_UPDATE_INSTALL_TEST_OK\n'
