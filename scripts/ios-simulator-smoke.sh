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
PROJECT="$ROOT/ios/IrisDriveIOS.xcodeproj"
SCHEME="IrisDriveIOS"
CONFIGURATION="${IRIS_DRIVE_IOS_XCODE_CONFIGURATION:-Debug}"
DERIVED_DATA="$ROOT/ios/.build/DerivedData"
BUILD_LOG="${IRIS_DRIVE_IOS_BUILD_LOG:-/tmp/iris-drive-ios-build.log}"
BUNDLE_ID="to.iris.drive.ios"
DEVICE_NAME="${IRIS_DRIVE_IOS_SIMULATOR_DEVICE:-}"

usage() {
  cat <<'USAGE'
Usage:
  scripts/ios-simulator-smoke.sh [--build-only]

Environment:
  IRIS_DRIVE_IOS_SIMULATOR_DEVICE  Optional simulator device name.
  IRIS_DRIVE_IOS_BUILD_LOG         Build log path.
USAGE
}

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

if [[ ! -d "$PROJECT" ]]; then
  (cd "$ROOT/ios" && xcodegen generate)
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
  build >"$BUILD_LOG"

APP_PATH="$(resolve_app_path)"
if [[ -z "$APP_PATH" || ! -d "$APP_PATH" ]]; then
  echo "FAIL: built iOS app not found. Build log: $BUILD_LOG" >&2
  exit 1
fi

if [[ "$BUILD_ONLY" == "1" ]]; then
  echo "IOS_BUILD_OK"
  echo "$APP_PATH"
  exit 0
fi

xcrun simctl boot "$DEVICE_UDID" >/dev/null 2>&1 || true
xcrun simctl bootstatus "$DEVICE_UDID" -b >/dev/null
xcrun simctl install "$DEVICE_UDID" "$APP_PATH"
xcrun simctl launch "$DEVICE_UDID" "$BUNDLE_ID" >/dev/null

if ! xcrun simctl get_app_container "$DEVICE_UDID" "$BUNDLE_ID" data >/dev/null; then
  echo "FAIL: iOS app container unavailable after launch" >&2
  exit 1
fi

echo "IOS_SIMULATOR_SMOKE_OK"
echo "device=$DEVICE_UDID"
echo "app=$APP_PATH"
