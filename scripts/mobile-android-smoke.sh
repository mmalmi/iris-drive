#!/usr/bin/env bash
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
PACKAGE_NAME="${IRIS_DRIVE_ANDROID_PACKAGE:-to.iris.drive.debug}"
MAIN_ACTIVITY="${IRIS_DRIVE_ANDROID_ACTIVITY:-to.iris.drive.debug/to.iris.drive.app.MainActivity}"
PROVIDER_AUTHORITY="${IRIS_DRIVE_ANDROID_PROVIDER_AUTHORITY:-to.iris.drive.documents}"
DEBUG_ACTION_EXTRA="${IRIS_DRIVE_ANDROID_DEBUG_ACTION_EXTRA:-to.iris.drive.DEBUG_ACTION}"
APK_PATH="${IRIS_DRIVE_ANDROID_APK:-$ROOT/android/app/build/outputs/apk/debug/app-debug.apk}"

build=1
clear_state=0
add_debug_root=0
serial="${IRIS_DRIVE_ANDROID_SERIAL:-${ANDROID_SERIAL:-}}"

usage() {
  cat >&2 <<'USAGE'
Usage: scripts/mobile-android-smoke.sh [--no-build] [--clear] [--add-debug-root] [--serial SERIAL]

Builds and installs the debug APK, launches the Android app through adb, and
verifies that the SAF DocumentsProvider authority is registered. Pass device
identifiers through env or CLI only; do not commit them.
USAGE
}

while [[ $# -gt 0 ]]; do
  case "$1" in
    --no-build)
      build=0
      ;;
    --clear)
      clear_state=1
      ;;
    --add-debug-root)
      add_debug_root=1
      ;;
    --serial)
      if [[ $# -lt 2 ]]; then
        echo "--serial requires a value" >&2
        exit 2
      fi
      serial="$2"
      shift
      ;;
    -h|--help)
      usage
      exit 0
      ;;
    *)
      usage
      exit 2
      ;;
  esac
  shift
done

sdk_from_local_properties() {
  local file="$ROOT/android/local.properties"
  if [[ -f "$file" ]]; then
    sed -n 's/^sdk\.dir=//p' "$file" | head -n 1
  fi
}

resolve_adb() {
  local sdk="${ANDROID_HOME:-${ANDROID_SDK_ROOT:-}}"
  if [[ -z "$sdk" ]]; then
    sdk="$(sdk_from_local_properties)"
  fi
  if [[ -z "$sdk" && -d "$HOME/Library/Android/sdk" ]]; then
    sdk="$HOME/Library/Android/sdk"
  fi
  if [[ -n "$sdk" && -x "$sdk/platform-tools/adb" ]]; then
    printf '%s\n' "$sdk/platform-tools/adb"
    return
  fi
  if command -v adb >/dev/null 2>&1; then
    command -v adb
    return
  fi
  echo "adb not found; set ANDROID_HOME/ANDROID_SDK_ROOT or add adb to PATH" >&2
  exit 1
}

select_serial() {
  local adb="$1"
  if [[ -n "$serial" ]]; then
    printf '%s\n' "$serial"
    return
  fi
  "$adb" devices | awk 'NR > 1 && $2 == "device" { print $1; exit }'
}

ADB="$(resolve_adb)"

if [[ "$build" -eq 1 ]]; then
  "$ROOT/tools/run-android" build
fi

if [[ ! -f "$APK_PATH" ]]; then
  echo "Debug APK not found at $APK_PATH; run just android-build first" >&2
  exit 1
fi

serial="$(select_serial "$ADB")"
if [[ -z "$serial" ]]; then
  echo "No online Android device or emulator found; set IRIS_DRIVE_ANDROID_SERIAL or start an emulator" >&2
  exit 1
fi

"$ADB" -s "$serial" wait-for-device
"$ADB" -s "$serial" install -r "$APK_PATH" >/dev/null

if [[ "$clear_state" -eq 1 ]]; then
  "$ADB" -s "$serial" shell pm clear "$PACKAGE_NAME" >/dev/null
fi

"$ADB" -s "$serial" shell am start -n "$MAIN_ACTIVITY" >/dev/null
if [[ "$add_debug_root" -eq 1 ]]; then
  "$ADB" -s "$serial" shell am start -n "$MAIN_ACTIVITY" --es "$DEBUG_ACTION_EXTRA" create-profile >/dev/null
  "$ADB" -s "$serial" shell am start -n "$MAIN_ACTIVITY" --es "$DEBUG_ACTION_EXTRA" add-root >/dev/null
fi
"$ADB" -s "$serial" shell pm path "$PACKAGE_NAME" >/dev/null
"$ADB" -s "$serial" shell dumpsys package "$PACKAGE_NAME" | grep -F "$PROVIDER_AUTHORITY" >/dev/null
"$ADB" -s "$serial" shell dumpsys package "$PACKAGE_NAME" | grep -F "device-link" >/dev/null
"$ADB" -s "$serial" shell dumpsys package "$PACKAGE_NAME" | grep -F "invite" >/dev/null

echo "Android smoke passed on adb serial: $serial"
