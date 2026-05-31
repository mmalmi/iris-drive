#!/usr/bin/env bash

set -Eeuo pipefail

case "$(uname -s)" in
  Darwin) ;;
  *)
    echo "iOS simulator smoke is Darwin-only; skipping on $(uname -s)"
    exit 0
    ;;
esac

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
source "$ROOT/scripts/ios-simulator-signing.sh"
PROJECT="$ROOT/ios/IrisDriveIOS.xcodeproj"
SCHEME="IrisDriveIOS"
CONFIGURATION="${IRIS_DRIVE_IOS_XCODE_CONFIGURATION:-Debug}"
DERIVED_DATA="$ROOT/ios/.build/DerivedData"
BUILD_LOG="${IRIS_DRIVE_IOS_BUILD_LOG:-/tmp/iris-drive-ios-build.log}"
BUNDLE_ID="to.iris.drive.ios"
DEVICE_NAME="${IRIS_DRIVE_IOS_SIMULATOR_DEVICE:-}"
APP_GROUP_ID="group.to.iris.drive"
TARGET_DIR="${CARGO_TARGET_DIR:-$(cargo metadata --format-version 1 --no-deps | python3 -c 'import json,sys; print(json.load(sys.stdin)["target_directory"])')}"
IDRIVE="${IRIS_DRIVE_IDRIVE_BIN:-$TARGET_DIR/debug/idrive}"
RUST_IOS_TARGET="${IRIS_DRIVE_IOS_RUST_TARGET:-aarch64-apple-ios-sim}"
RUST_LIB_DIR="$TARGET_DIR/$RUST_IOS_TARGET/debug"
RUST_STATIC_LIB="$RUST_LIB_DIR/libiris_drive_app_core.a"
OWNER_CONFIG="$(mktemp -d -t iris-drive-ios-gui-owner)"

usage() {
  cat <<'USAGE'
Usage:
  scripts/ios-simulator-smoke.sh [--build-only]

Environment:
  IRIS_DRIVE_IOS_SIMULATOR_DEVICE  Optional simulator device name.
  IRIS_DRIVE_IOS_BUILD_LOG         Build log path.
USAGE
}

cleanup() {
  rm -rf "$OWNER_CONFIG"
}
trap cleanup EXIT

BUILD_ONLY=0
while [[ $# -gt 0 ]]; do
  case "$1" in
    --build-only)
      BUILD_ONLY=1
      shift
      ;;
    -h|--help)
      usage
      exit 0
      ;;
    *)
      echo "unknown argument: $1" >&2
      usage >&2
      exit 2
      ;;
  esac
done

select_simulator() {
  local devices_json
  devices_json="$(mktemp -t iris-drive-ios-simulators.XXXXXX.json)"
  xcrun simctl list devices available --json >"$devices_json"
  python3 - "$DEVICE_NAME" "$devices_json" <<'PY'
import json
import sys

preferred = sys.argv[1]
with open(sys.argv[2], "r", encoding="utf-8") as handle:
    data = json.load(handle)
booted = []
available = []
for runtime, devices in data.get("devices", {}).items():
    if "iOS" not in runtime:
        continue
    for device in devices:
        if not device.get("isAvailable"):
            continue
        if preferred and device.get("name") != preferred:
            continue
        if "iPhone" not in device.get("name", ""):
            continue
        if device.get("state") == "Booted":
            booted.append(device)
        available.append(device)

choices = booted or available
if not choices:
    raise SystemExit("no available iPhone simulator found")
print(choices[0]["udid"])
PY
  rm -f "$devices_json"
}

resolve_app_path() {
  find "$DERIVED_DATA/Build/Products" \
    -path "*/$CONFIGURATION-iphonesimulator/Iris Drive.app" \
    -type d \
    -print \
    -quit 2>/dev/null
}

assert_static_app_core_linkage() {
  local app_path="$1"
  local offenders

  offenders="$(
    find "$app_path" -type f -perm -111 -print 2>/dev/null |
      while IFS= read -r binary; do
        if otool -L "$binary" 2>/dev/null | grep -F "libiris_drive_app_core.dylib" >/dev/null; then
          printf '%s\n' "$binary"
        fi
      done
  )"
  if [[ -n "$offenders" ]]; then
    echo "FAIL: iOS app links iris-drive app-core dynamically; use the static archive instead." >&2
    echo "$offenders" >&2
    exit 1
  fi
}

app_group_container() {
  xcrun simctl get_app_container "$DEVICE_UDID" "$BUNDLE_ID" group.to.iris.drive 2>/dev/null
}

app_data_container() {
  xcrun simctl get_app_container "$DEVICE_UDID" "$BUNDLE_ID" data 2>/dev/null
}

safe_remove_sim_container() {
  local container="$1"

  if [[ -z "$container" ]]; then
    return 0
  fi
  if [[ "$container" != *"/CoreSimulator/Devices/$DEVICE_UDID/"* ]]; then
    echo "FAIL: refusing to remove unexpected simulator container path: $container" >&2
    exit 1
  fi
  rm -rf "$container"
}

wait_for_debug_state() {
  local state_file="$1"
  local jq_expr="$2"
  local seconds="$3"
  for _ in $(seq 1 "$((seconds * 5))"); do
    if [[ -f "$state_file" ]] && python3 -c "$jq_expr" <"$state_file" >/dev/null 2>&1; then
      return 0
    fi
    sleep 0.2
  done
  return 1
}

if [[ ! -x "$IDRIVE" ]]; then
  cargo build -p idrive
fi

cargo build -p iris-drive-app-core --target "$RUST_IOS_TARGET"
if [[ ! -f "$RUST_STATIC_LIB" ]]; then
  echo "FAIL: static app-core library not found at $RUST_STATIC_LIB" >&2
  exit 1
fi

if command -v xcodegen >/dev/null 2>&1; then
  (cd "$ROOT/ios" && xcodegen generate)
elif [[ ! -d "$PROJECT" ]]; then
  echo "FAIL: $PROJECT is missing and xcodegen is not installed" >&2
  exit 1
fi

DEVICE_UDID="$(select_simulator)"
DESTINATION="platform=iOS Simulator,id=$DEVICE_UDID"

xcodebuild \
  -project "$PROJECT" \
  -scheme "$SCHEME" \
  -configuration "$CONFIGURATION" \
  -derivedDataPath "$DERIVED_DATA" \
  -destination "$DESTINATION" \
  CODE_SIGNING_ALLOWED=YES \
  CODE_SIGNING_REQUIRED=YES \
  CODE_SIGN_IDENTITY="${IRIS_DRIVE_IOS_CODE_SIGN_IDENTITY:--}" \
  PROVISIONING_PROFILE_SPECIFIER= \
  LIBRARY_SEARCH_PATHS="$RUST_LIB_DIR" \
  OTHER_LDFLAGS="$RUST_STATIC_LIB" \
  build >"$BUILD_LOG"

APP_PATH="$(resolve_app_path)"
if [[ -z "$APP_PATH" || ! -d "$APP_PATH" ]]; then
  echo "FAIL: built iOS app not found. Build log: $BUILD_LOG" >&2
  exit 1
fi
assert_static_app_core_linkage "$APP_PATH"
iris_drive_ios_assert_simulator_entitlements "$DERIVED_DATA" "$CONFIGURATION"

if [[ "$BUILD_ONLY" == "1" ]]; then
  echo "IOS_BUILD_OK"
  echo "$APP_PATH"
  exit 0
fi

xcrun simctl boot "$DEVICE_UDID" >/dev/null 2>&1 || true
xcrun simctl bootstatus "$DEVICE_UDID" -b >/dev/null

xcrun simctl uninstall "$DEVICE_UDID" "$BUNDLE_ID" >/dev/null 2>&1 || true
owner_json="$("$IDRIVE" --config-dir "$OWNER_CONFIG" init --force --label "CLI owner")"
owner_invite="$(python3 -c 'import json,sys; print(json.load(sys.stdin)["device_link_invite"]["url"])' <<<"$owner_json")"

xcrun simctl install "$DEVICE_UDID" "$APP_PATH"

DATA_CONTAINER="$(app_data_container || true)"
GROUP_CONTAINER="$(app_group_container || true)"
if [[ -z "$DATA_CONTAINER" || ! -d "$DATA_CONTAINER" ]]; then
  echo "FAIL: iOS app container unavailable after install" >&2
  exit 1
fi
if [[ -z "$GROUP_CONTAINER" || ! -d "$GROUP_CONTAINER" ]]; then
  echo "FAIL: iOS app group container unavailable after install" >&2
  exit 1
fi
safe_remove_sim_container "$DATA_CONTAINER/Library/Application Support/IrisDrive"
SIM_APP_BASE_DIR="$GROUP_CONTAINER/IrisDrive"
safe_remove_sim_container "$SIM_APP_BASE_DIR"
mkdir -p "$SIM_APP_BASE_DIR"

SIMCTL_CHILD_IRIS_DRIVE_DEBUG_ACTION=link-device \
  SIMCTL_CHILD_IRIS_DRIVE_DEBUG_OWNER="$owner_invite" \
  SIMCTL_CHILD_IRIS_DRIVE_UI_TEST_BASE_DIR="$SIM_APP_BASE_DIR" \
  xcrun simctl launch --terminate-running-process "$DEVICE_UDID" "$BUNDLE_ID" >/dev/null

STATE_FILE="$SIM_APP_BASE_DIR/debug-state.json"
if ! wait_for_debug_state \
  "$STATE_FILE" \
  'import json,sys; s=json.load(sys.stdin); a=s.get("ui",{}).get("account") or {}; raise SystemExit(0 if a.get("authorization_state") == "awaiting_approval" and a.get("device_link_request") else 1)' \
  15; then
  echo "FAIL: iOS GUI did not create a real awaiting linked-device profile." >&2
  [[ -f "$STATE_FILE" ]] && cat "$STATE_FILE" >&2
  exit 1
fi

request_url="$(python3 -c 'import json,sys; print(json.load(sys.stdin)["ui"]["account"]["device_link_request"])' <"$STATE_FILE")"
approved_json="$("$IDRIVE" --config-dir "$OWNER_CONFIG" approve "$request_url" --label "iOS GUI")"
roster_size="$(python3 -c 'import json,sys; print(json.load(sys.stdin)["roster_size"])' <<<"$approved_json")"
if [[ "$roster_size" != "2" ]]; then
  echo "FAIL: CLI owner did not approve the iOS GUI request." >&2
  echo "$approved_json" >&2
  exit 1
fi

echo "IOS_SIMULATOR_SMOKE_OK"
echo "device=$DEVICE_UDID"
echo "app=$APP_PATH"
echo "owner_config=$OWNER_CONFIG"
