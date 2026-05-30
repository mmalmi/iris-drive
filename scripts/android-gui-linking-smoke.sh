#!/usr/bin/env bash
set -Eeuo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
PACKAGE_NAME="${IRIS_DRIVE_ANDROID_PACKAGE:-to.iris.drive.debug}"
MAIN_ACTIVITY="${IRIS_DRIVE_ANDROID_ACTIVITY:-to.iris.drive.debug/to.iris.drive.app.MainActivity}"
DEBUG_ACTION_EXTRA="${IRIS_DRIVE_ANDROID_DEBUG_ACTION_EXTRA:-to.iris.drive.DEBUG_ACTION}"
DEBUG_OWNER_EXTRA="${IRIS_DRIVE_ANDROID_DEBUG_OWNER_EXTRA:-to.iris.drive.DEBUG_OWNER}"
APK_PATH="${IRIS_DRIVE_ANDROID_APK:-$ROOT/android/app/build/outputs/apk/debug/app-debug.apk}"
TARGET_DIR="${CARGO_TARGET_DIR:-$(cargo metadata --format-version 1 --no-deps | python3 -c 'import json,sys; print(json.load(sys.stdin)["target_directory"])')}"
IDRIVE="${IRIS_DRIVE_IDRIVE_BIN:-$TARGET_DIR/debug/idrive}"
OWNER_CONFIG="$(mktemp -d -t iris-drive-android-gui-owner)"
serial="${IRIS_DRIVE_ANDROID_SERIAL:-${ANDROID_SERIAL:-}}"

cleanup() {
  rm -rf "$OWNER_CONFIG"
}
trap cleanup EXIT

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
  command -v adb
}

select_serial() {
  local adb="$1"
  if [[ -n "$serial" ]]; then
    printf '%s\n' "$serial"
    return
  fi
  "$adb" devices | awk 'NR > 1 && $2 == "device" { print $1; exit }'
}

wait_for_debug_state() {
  local jq_expr="$1"
  local seconds="$2"
  for _ in $(seq 1 "$((seconds * 5))"); do
    if "$ADB" -s "$serial" exec-out run-as "$PACKAGE_NAME" cat files/debug-state.json 2>/dev/null \
      | python3 -c "$jq_expr" >/dev/null 2>&1; then
      return 0
    fi
    sleep 0.2
  done
  return 1
}

ADB="$(resolve_adb)"
serial="$(select_serial "$ADB")"
if [[ -z "$serial" ]]; then
  echo "FAIL: no online Android device or emulator found" >&2
  exit 1
fi

"$ADB" -s "$serial" wait-for-device
(
  cd "$ROOT"
  ANDROID_SERIAL="$serial" ./tools/run-android :app:connectedDebugAndroidTest \
    -Pandroid.testInstrumentationRunnerArguments.class=to.iris.drive.app.IrisDriveAndroidGuiFlowTest
)

if [[ ! -x "$IDRIVE" ]]; then
  cargo build -p idrive
fi
if [[ ! -f "$APK_PATH" ]]; then
  echo "FAIL: Debug APK not found at $APK_PATH" >&2
  exit 1
fi

"$ADB" -s "$serial" install -r "$APK_PATH" >/dev/null
"$ADB" -s "$serial" shell pm clear "$PACKAGE_NAME" >/dev/null
"$ADB" -s "$serial" shell am start -S -n "$MAIN_ACTIVITY" \
  --es "$DEBUG_ACTION_EXTRA" create-profile >/dev/null

if ! wait_for_debug_state \
  'import json,sys; s=json.load(sys.stdin); a=s.get("ui",{}).get("account") or {}; raise SystemExit(0 if a.get("authorization_state") == "authorized" and a.get("has_owner_signing_authority") else 1)' \
  15; then
  echo "FAIL: Android did not create a real owner profile after the GUI create-profile test." >&2
  "$ADB" -s "$serial" exec-out run-as "$PACKAGE_NAME" cat files/debug-state.json >&2 || true
  exit 1
fi

owner_json="$("$IDRIVE" --config-dir "$OWNER_CONFIG" init --force --label "CLI owner")"
owner_invite="$(python3 -c 'import json,sys; print(json.load(sys.stdin)["device_link_invite"]["url"])' <<<"$owner_json")"

"$ADB" -s "$serial" shell pm clear "$PACKAGE_NAME" >/dev/null
"$ADB" -s "$serial" shell am start -S -n "$MAIN_ACTIVITY" \
  --es "$DEBUG_ACTION_EXTRA" link-device \
  --es "$DEBUG_OWNER_EXTRA" "$owner_invite" >/dev/null

if ! wait_for_debug_state \
  'import json,sys; s=json.load(sys.stdin); a=s.get("ui",{}).get("account") or {}; raise SystemExit(0 if a.get("authorization_state") == "awaiting_approval" and a.get("device_link_request") else 1)' \
  15; then
  echo "FAIL: Android did not create a real awaiting linked-device profile after the GUI link-this-device test." >&2
  "$ADB" -s "$serial" exec-out run-as "$PACKAGE_NAME" cat files/debug-state.json >&2 || true
  exit 1
fi

request_url="$("$ADB" -s "$serial" exec-out run-as "$PACKAGE_NAME" cat files/debug-state.json \
  | python3 -c 'import json,sys; print(json.load(sys.stdin)["ui"]["account"]["device_link_request"])')"
approved_json="$("$IDRIVE" --config-dir "$OWNER_CONFIG" approve "$request_url" --label "Android GUI")"
roster_size="$(python3 -c 'import json,sys; print(json.load(sys.stdin)["roster_size"])' <<<"$approved_json")"
if [[ "$roster_size" != "2" ]]; then
  echo "FAIL: CLI owner did not approve the Android GUI request." >&2
  echo "$approved_json" >&2
  exit 1
fi

echo "ANDROID_GUI_LINKING_SMOKE_OK"
echo "serial=$serial"
echo "owner_config=$OWNER_CONFIG"
