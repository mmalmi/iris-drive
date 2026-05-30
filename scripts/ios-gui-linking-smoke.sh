#!/usr/bin/env bash

set -Eeuo pipefail

case "$(uname -s)" in
  Darwin) ;;
  *)
    echo "iOS GUI smoke is Darwin-only; skipping on $(uname -s)"
    exit 0
    ;;
esac

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
PROJECT="$ROOT/ios/IrisDriveIOS.xcodeproj"
SCHEME="IrisDriveIOS"
CONFIGURATION="${IRIS_DRIVE_IOS_XCODE_CONFIGURATION:-Debug}"
DERIVED_DATA="$ROOT/ios/.build/DerivedData"
BUILD_LOG="${IRIS_DRIVE_IOS_UI_BUILD_LOG:-/tmp/iris-drive-ios-ui-tests.log}"
BUNDLE_ID="to.iris.drive.ios"
DEVICE_NAME="${IRIS_DRIVE_IOS_SIMULATOR_DEVICE:-}"
TARGET_DIR="${CARGO_TARGET_DIR:-$(cargo metadata --format-version 1 --no-deps | python3 -c 'import json,sys; print(json.load(sys.stdin)["target_directory"])')}"
IDRIVE="${IRIS_DRIVE_IDRIVE_BIN:-$TARGET_DIR/debug/idrive}"
RUST_IOS_TARGET="${IRIS_DRIVE_IOS_RUST_TARGET:-aarch64-apple-ios-sim}"
RUST_LIB_DIR="$TARGET_DIR/$RUST_IOS_TARGET/debug"
OWNER_CONFIG="$(mktemp -d -t iris-drive-ios-ui-owner)"
LINKED_CONFIG="$(mktemp -d -t iris-drive-ios-ui-linked)"
XCTESTRUN=""

cleanup() {
  rm -rf "$OWNER_CONFIG" "$LINKED_CONFIG"
}
trap cleanup EXIT

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

app_container() {
  xcrun simctl get_app_container "$DEVICE_UDID" "$BUNDLE_ID" data 2>/dev/null
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

resolve_xctestrun() {
  find "$DERIVED_DATA/Build/Products" \
    -maxdepth 1 \
    -name "${SCHEME}_*.xctestrun" \
    -type f \
    -print \
    -quit 2>/dev/null
}

run_ui_test() {
  local only_testing="$1"
  shift
  local run_stem
  local run_file
  run_stem="$(mktemp "$DERIVED_DATA/Build/Products/IrisDriveIOS-ui.XXXXXX")"
  run_file="$run_stem.xctestrun"
  mv "$run_stem" "$run_file"
  cp "$XCTESTRUN" "$run_file"

  python3 - "$run_file" "$@" <<'PY'
import plistlib
import sys

path = sys.argv[1]
updates = {}
for assignment in sys.argv[2:]:
    key, separator, value = assignment.partition("=")
    if not separator:
        raise SystemExit(f"invalid environment assignment: {assignment}")
    updates[key] = value

with open(path, "rb") as handle:
    data = plistlib.load(handle)

for target in data.values():
    if not isinstance(target, dict) or not target.get("IsUITestBundle"):
        continue
    for env_key in (
        "EnvironmentVariables",
        "TestingEnvironmentVariables",
        "UITargetAppEnvironmentVariables",
    ):
        target.setdefault(env_key, {}).update(updates)

with open(path, "wb") as handle:
    plistlib.dump(data, handle)
PY

  local status=0
  xcodebuild \
    -xctestrun "$run_file" \
    -destination "$DESTINATION" \
    -only-testing:"$only_testing" \
    test-without-building >>"$BUILD_LOG" || status=$?
  rm -f "$run_file"
  return "$status"
}

if [[ ! -x "$IDRIVE" ]]; then
  cargo build -p idrive
fi

cargo build -p iris-drive-app-core --target "$RUST_IOS_TARGET"

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
  CODE_SIGNING_ALLOWED=NO \
  LIBRARY_SEARCH_PATHS="$RUST_LIB_DIR" \
  OTHER_LDFLAGS="-liris_drive_app_core" \
  build-for-testing >"$BUILD_LOG"

XCTESTRUN="$(resolve_xctestrun)"
if [[ -z "$XCTESTRUN" || ! -f "$XCTESTRUN" ]]; then
  echo "FAIL: iOS UI test run file not found. Build log: $BUILD_LOG" >&2
  exit 1
fi

xcrun simctl boot "$DEVICE_UDID" >/dev/null 2>&1 || true
xcrun simctl bootstatus "$DEVICE_UDID" -b >/dev/null

xcrun simctl uninstall "$DEVICE_UDID" "$BUNDLE_ID" >/dev/null 2>&1 || true
run_ui_test "IrisDriveIOSUITests/IrisDriveIOSUITests/testWelcomeRoutesWithoutSetupTitle"

owner_json="$("$IDRIVE" --config-dir "$OWNER_CONFIG" init --force --label "CLI owner")"
owner_invite="$(python3 -c 'import json,sys; print(json.load(sys.stdin)["device_link_invite"]["url"])' <<<"$owner_json")"

xcrun simctl uninstall "$DEVICE_UDID" "$BUNDLE_ID" >/dev/null 2>&1 || true
run_ui_test \
  "IrisDriveIOSUITests/IrisDriveIOSUITests/testLinkThisDeviceFromWelcome" \
  "IRIS_DRIVE_UI_TEST_OWNER_INVITE=$owner_invite"

CONTAINER="$(app_container)"
STATE_FILE="$CONTAINER/Library/Application Support/Iris Drive/debug-state.json"
if ! wait_for_debug_state \
  "$STATE_FILE" \
  'import json,sys; s=json.load(sys.stdin); a=s.get("ui",{}).get("account") or {}; raise SystemExit(0 if a.get("authorization_state") == "awaiting_approval" and a.get("device_link_request") else 1)' \
  15; then
  echo "FAIL: iOS Link this device UI did not create an awaiting linked-device profile." >&2
  [[ -f "$STATE_FILE" ]] && cat "$STATE_FILE" >&2
  exit 1
fi
request_url="$(python3 -c 'import json,sys; print(json.load(sys.stdin)["ui"]["account"]["device_link_request"])' <"$STATE_FILE")"
approved_json="$("$IDRIVE" --config-dir "$OWNER_CONFIG" approve "$request_url" --label "iOS UI linked")"
roster_size="$(python3 -c 'import json,sys; print(json.load(sys.stdin)["roster_size"])' <<<"$approved_json")"
if [[ "$roster_size" != "2" ]]; then
  echo "FAIL: CLI owner did not approve the iOS UI link request." >&2
  echo "$approved_json" >&2
  exit 1
fi

xcrun simctl uninstall "$DEVICE_UDID" "$BUNDLE_ID" >/dev/null 2>&1 || true
run_ui_test "IrisDriveIOSUITests/IrisDriveIOSUITests/testCreateProfileFromWelcome"

CONTAINER="$(app_container)"
STATE_FILE="$CONTAINER/Library/Application Support/Iris Drive/debug-state.json"
if ! wait_for_debug_state \
  "$STATE_FILE" \
  'import json,sys; s=json.load(sys.stdin); a=s.get("ui",{}).get("account") or {}; raise SystemExit(0 if a.get("authorization_state") == "authorized" and a.get("device_link_invite") else 1)' \
  15; then
  echo "FAIL: iOS Create profile UI did not initialize an owner profile." >&2
  [[ -f "$STATE_FILE" ]] && cat "$STATE_FILE" >&2
  exit 1
fi
app_invite="$(python3 -c 'import json,sys; print(json.load(sys.stdin)["ui"]["account"]["device_link_invite"])' <"$STATE_FILE")"
linked_json="$("$IDRIVE" --config-dir "$LINKED_CONFIG" link "$app_invite" --label "iOS UI linked")"
linked_device="$(python3 -c 'import json,sys; print(json.load(sys.stdin)["device_npub"])' <<<"$linked_json")"

run_ui_test \
  "IrisDriveIOSUITests/IrisDriveIOSUITests/testAddLinkedDeviceFromDevices" \
  "IRIS_DRIVE_UI_TEST_LINKED_DEVICE=$linked_device" \
  "IRIS_DRIVE_UI_TEST_LINKED_DEVICE_LABEL=iOS UI linked"

CONTAINER="$(app_container)"
STATE_FILE="$CONTAINER/Library/Application Support/Iris Drive/debug-state.json"
if ! wait_for_debug_state \
  "$STATE_FILE" \
  'import json,sys; s=json.load(sys.stdin); devices=s.get("ui",{}).get("devices") or []; raise SystemExit(0 if any(d.get("label") == "iOS UI linked" for d in devices) and len(devices) >= 2 else 1)' \
  15; then
  echo "FAIL: iOS Add Device UI did not add the linked device." >&2
  [[ -f "$STATE_FILE" ]] && cat "$STATE_FILE" >&2
  exit 1
fi

echo "IOS_GUI_LINKING_SMOKE_OK"
echo "device=$DEVICE_UDID"
echo "build_log=$BUILD_LOG"
