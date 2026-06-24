#!/usr/bin/env bash

set -Eeuo pipefail

case "$(uname -s)" in
  Darwin) ;;
  *)
    echo "iOS device smoke is Darwin-only; skipping on $(uname -s)"
    exit 0
    ;;
esac

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
PROJECT="$ROOT/ios/IrisDriveIOS.xcodeproj"
SCHEME="IrisDriveIOS"
CONFIGURATION="${IRIS_DRIVE_IOS_XCODE_CONFIGURATION:-Debug}"
DERIVED_DATA="$ROOT/ios/.build/DeviceDerivedData"
BUNDLE_ID="${IRIS_DRIVE_IOS_BUNDLE_ID:-to.iris.drive.ios}"
DEVICE_SELECTOR="${IRIS_DRIVE_IOS_DEVICE:-virus.exe}"
DEVELOPMENT_TEAM="${IRIS_DRIVE_IOS_DEVELOPMENT_TEAM:-J8PPJKD7TA}"
CODE_SIGN_IDENTITY="${IRIS_DRIVE_IOS_CODE_SIGN_IDENTITY:-Apple Development}"
LAUNCH_WAIT_SECONDS="${IRIS_DRIVE_IOS_LAUNCH_WAIT_SECONDS:-3}"
ALLOW_SCREEN_OFF="${IRIS_DRIVE_IOS_ALLOW_SCREEN_OFF:-0}"
RUN_SHARE_EXTENSION_TESTS="${IRIS_DRIVE_IOS_RUN_SHARE_EXTENSION_TESTS:-1}"
TARGET_DIR="${CARGO_TARGET_DIR:-$(cargo metadata --format-version 1 --no-deps | python3 -c 'import json,sys; print(json.load(sys.stdin)["target_directory"])')}"
RUST_IOS_TARGET="${IRIS_DRIVE_IOS_RUST_TARGET:-aarch64-apple-ios}"
RUST_LIB_DIR="$TARGET_DIR/$RUST_IOS_TARGET/debug"
RUST_STATIC_LIB="$RUST_LIB_DIR/libiris_drive_app_core.a"

select_device() {
  local devices_json
  devices_json="$(mktemp -t iris-drive-ios-devices.XXXXXX.json)"
  xcrun devicectl list devices --json-output "$devices_json" >/dev/null
  python3 - "$DEVICE_SELECTOR" "$devices_json" <<'PY'
import json
import sys

preferred = sys.argv[1].strip()
with open(sys.argv[2], "r", encoding="utf-8") as handle:
    devices = json.load(handle)["result"]["devices"]

def is_usable_iphone(device):
    product_type = device.get("hardwareProperties", {}).get("productType", "")
    connection = device.get("connectionProperties", {})
    return (
        product_type.startswith("iPhone")
        and connection.get("pairingState") == "paired"
        and connection.get("tunnelState") != "unavailable"
    )

def names(device):
    props = device.get("deviceProperties", {})
    connection = device.get("connectionProperties", {})
    values = [device.get("identifier"), props.get("name")]
    values.extend(connection.get("potentialHostnames", []))
    return {value for value in values if value}

if preferred:
    for device in devices:
        if preferred in names(device):
            if not is_usable_iphone(device):
                raise SystemExit(f"selected iOS device is not usable for install: {preferred}")
            print(device["identifier"])
            raise SystemExit(0)
    raise SystemExit(f"iOS device not found: {preferred}")

for device in devices:
    if is_usable_iphone(device):
        print(device["identifier"])
        raise SystemExit(0)
raise SystemExit("no paired available iPhone found")
PY
  rm -f "$devices_json"
}

assert_device_awake_for_launch() {
  local device_udid="$1"
  local display_json
  local backlight_state

  display_json="$(mktemp -t iris-drive-ios-display.XXXXXX.json)"
  if ! xcrun devicectl device info displays \
    --device "$device_udid" \
    --json-output "$display_json" >/dev/null; then
    rm -f "$display_json"
    echo "FAIL: could not read iOS device display state before launch." >&2
    exit 1
  fi

  backlight_state="$(python3 - "$display_json" <<'PY'
import json
import sys

with open(sys.argv[1], "r", encoding="utf-8") as handle:
    print((json.load(handle).get("result") or {}).get("backlightState") or "")
PY
)"
  rm -f "$display_json"

  if [[ "$ALLOW_SCREEN_OFF" != "1" && "$backlight_state" == "off" ]]; then
    echo "FAIL: selected iOS device screen is off; wake and unlock the device before running physical launch smoke." >&2
    echo "Set IRIS_DRIVE_IOS_ALLOW_SCREEN_OFF=1 to bypass this preflight." >&2
    exit 1
  fi
}

resolve_app_path() {
  find "$DERIVED_DATA/Build/Products" \
    -path "*/$CONFIGURATION-iphoneos/Iris Drive.app" \
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
    echo "FAIL: iOS app links iris-drive app-core dynamically; physical devices cannot load host build paths." >&2
    echo "$offenders" >&2
    exit 1
  fi
}

assert_app_running() {
  local device_udid="$1"
  local processes

  processes="$(xcrun devicectl device info processes --device "$device_udid")"
  if ! grep -F "Iris Drive.app/Iris Drive" <<<"$processes" >/dev/null; then
    echo "FAIL: Iris Drive was not running $LAUNCH_WAIT_SECONDS seconds after launch." >&2
    xcrun devicectl device process launch \
      --device "$device_udid" \
      --terminate-existing \
      --console \
      --timeout 10 \
      "$BUNDLE_ID" >&2 || true
    exit 1
  fi
}

run_share_extension_tests() {
  local device_udid="$1"

  case "$RUN_SHARE_EXTENSION_TESTS" in
    1|true|TRUE|True|yes|YES|Yes|on|ON|On) ;;
    *) return 0 ;;
  esac

  xcodebuild \
    -project "$PROJECT" \
    -scheme "$SCHEME" \
    -configuration "$CONFIGURATION" \
    -derivedDataPath "$DERIVED_DATA" \
    -destination "platform=iOS,id=$device_udid" \
    DEVELOPMENT_TEAM="$DEVELOPMENT_TEAM" \
    CODE_SIGN_STYLE=Automatic \
    CODE_SIGN_IDENTITY="$CODE_SIGN_IDENTITY" \
    LIBRARY_SEARCH_PATHS="$RUST_LIB_DIR" \
    OTHER_LDFLAGS="$RUST_STATIC_LIB" \
    -allowProvisioningUpdates \
    -allowProvisioningDeviceRegistration \
    -only-testing:IrisDriveIOSShareExtensionTests \
    test

  echo "IOS_DEVICE_SHARE_EXTENSION_TESTS_OK"
}

DEVICE_UDID="$(select_device)"
assert_device_awake_for_launch "$DEVICE_UDID"

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

xcodebuild \
  -project "$PROJECT" \
  -scheme "$SCHEME" \
  -configuration "$CONFIGURATION" \
  -derivedDataPath "$DERIVED_DATA" \
  -destination "platform=iOS,id=$DEVICE_UDID" \
  DEVELOPMENT_TEAM="$DEVELOPMENT_TEAM" \
  CODE_SIGN_STYLE=Automatic \
  CODE_SIGN_IDENTITY="$CODE_SIGN_IDENTITY" \
  LIBRARY_SEARCH_PATHS="$RUST_LIB_DIR" \
  OTHER_LDFLAGS="$RUST_STATIC_LIB" \
  -allowProvisioningUpdates \
  -allowProvisioningDeviceRegistration \
  build

run_share_extension_tests "$DEVICE_UDID"

APP_PATH="$(resolve_app_path)"
if [[ -z "$APP_PATH" || ! -d "$APP_PATH" ]]; then
  echo "FAIL: built iOS app not found under $DERIVED_DATA" >&2
  exit 1
fi
assert_static_app_core_linkage "$APP_PATH"

xcrun devicectl device install app --device "$DEVICE_UDID" "$APP_PATH" >/dev/null
xcrun devicectl device process launch \
  --device "$DEVICE_UDID" \
  --terminate-existing \
  "$BUNDLE_ID" >/dev/null
sleep "$LAUNCH_WAIT_SECONDS"
assert_app_running "$DEVICE_UDID"

echo "IOS_DEVICE_SMOKE_OK"
echo "device=$DEVICE_UDID"
echo "app=$APP_PATH"
