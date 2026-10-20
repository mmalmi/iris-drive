#!/usr/bin/env bash

set -Eeuo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
source "$ROOT/scripts/ios-simulator-signing.sh"

PROJECT="$ROOT/ios/IrisDriveIOS.xcodeproj"
SCHEME="IrisDriveIOS"
TEST_TARGET="IrisDriveIOSUITests/IrisDriveAppStoreScreenshotTests/testCaptureAppStoreScreenshots"
DERIVED_DATA="${IRIS_DRIVE_IOS_SCREENSHOT_DERIVED_DATA:-$ROOT/ios/.build/AppStoreScreenshots}"
OUTPUT_ROOT="${IRIS_DRIVE_IOS_SCREENSHOT_DIR:-$ROOT/dist/ios-screenshots/en-US}"
TARGET_DIR="${CARGO_TARGET_DIR:-$(cargo metadata --format-version 1 --no-deps | python3 -c 'import json,sys; print(json.load(sys.stdin)["target_directory"])')}"
RUST_TARGET="${IRIS_DRIVE_IOS_SCREENSHOT_RUST_TARGET:-aarch64-apple-ios-sim}"
RUST_IOS_DEPLOYMENT_TARGET="${IRIS_DRIVE_IOS_DEPLOYMENT_TARGET:-17.0}"
RUST_LIB_DIR="$TARGET_DIR/$RUST_TARGET/debug"
RUST_STATIC_LIB="$RUST_LIB_DIR/libiris_drive_app_core.a"

IPHONE_69="${IRIS_DRIVE_IOS_SCREENSHOT_IPHONE:-iPhone 17 Pro Max}"
IPAD_PRO_13="${IRIS_DRIVE_IOS_SCREENSHOT_IPAD:-iPad Pro 13-inch (M5)}"
DEVICES=("$IPHONE_69" "$IPAD_PRO_13")

if [[ "${1:-}" == "--list" ]]; then
  xcrun simctl list devices available
  exit 0
fi
if [[ $# -gt 0 ]]; then
  DEVICES=("$@")
fi

resolve_udid() {
  local name="$1"
  xcrun simctl list devices available --json | python3 -c '
import json
import sys

name = sys.argv[1]
data = json.load(sys.stdin)
for runtime, devices in sorted(data.get("devices", {}).items(), reverse=True):
    if "iOS" not in runtime:
        continue
    for device in devices:
        if device.get("name") == name and device.get("isAvailable", True):
            print(device["udid"])
            raise SystemExit(0)
raise SystemExit(f"available simulator not found: {name}")
' "$name"
}

slugify() {
  printf '%s' "$1" | python3 -c '
import re
import sys

print(re.sub(r"[^a-z0-9]+", "-", sys.stdin.read().strip().lower()).strip("-"))
'
}

extract_screenshots() {
  local result_path="$1"
  local output_dir="$2"
  local export_dir="$3"

  mkdir -p "$export_dir" "$output_dir"
  xcrun xcresulttool export attachments \
    --path "$result_path" \
    --output-path "$export_dir" >/dev/null
  python3 - "$export_dir" "$output_dir" <<'PY'
import json
from pathlib import Path
import re
import shutil
import sys

export_dir = Path(sys.argv[1])
output_dir = Path(sys.argv[2])
manifest = json.loads((export_dir / "manifest.json").read_text(encoding="utf-8"))
found = 0
for test in manifest:
    for attachment in test.get("attachments", []):
        name = attachment.get("suggestedHumanReadableName", "")
        if not name.startswith("screenshot-"):
            continue
        source = export_dir / attachment["exportedFileName"]
        slug = name.removeprefix("screenshot-")
        slug = re.sub(r"_\d+_[A-F0-9-]+\.png$", "", slug)
        target = output_dir / f"{slug}.png"
        shutil.copyfile(source, target)
        print(f"  {target}")
        found += 1
if found == 0:
    raise SystemExit("no App Store screenshot attachments were exported")
PY
}

CARGO_TARGET_DIR="$TARGET_DIR" \
  IPHONEOS_DEPLOYMENT_TARGET="$RUST_IOS_DEPLOYMENT_TARGET" \
  cargo build -p iris-drive-app-core --target "$RUST_TARGET"
test -f "$RUST_STATIC_LIB"

if command -v xcodegen >/dev/null 2>&1; then
  (cd "$ROOT/ios" && xcodegen generate)
fi

FIRST_UDID="$(resolve_udid "${DEVICES[0]}")"
xcodebuild \
  -project "$PROJECT" \
  -scheme "$SCHEME" \
  -configuration Debug \
  -derivedDataPath "$DERIVED_DATA" \
  -destination "platform=iOS Simulator,id=$FIRST_UDID" \
  CODE_SIGNING_ALLOWED=YES \
  CODE_SIGNING_REQUIRED=YES \
  CODE_SIGN_IDENTITY="${IRIS_DRIVE_IOS_CODE_SIGN_IDENTITY:--}" \
  PROVISIONING_PROFILE_SPECIFIER= \
  LIBRARY_SEARCH_PATHS="$RUST_LIB_DIR" \
  OTHER_LDFLAGS="$RUST_STATIC_LIB" \
  build-for-testing

for device in "${DEVICES[@]}"; do
  udid="$(resolve_udid "$device")"
  slug="$(slugify "$device")"
  result_path="$DERIVED_DATA/results/$slug.xcresult"
  export_dir="$DERIVED_DATA/attachments/$slug"
  output_dir="$OUTPUT_ROOT/$(
    [[ "$device" == "$IPHONE_69" ]] && printf 'APP_IPHONE_69' || printf 'APP_IPAD_PRO_3GEN_129'
  )"

  rm -rf "$result_path" "$export_dir" "$output_dir"
  mkdir -p "$(dirname "$result_path")"
  xcrun simctl boot "$udid" >/dev/null 2>&1 || true
  xcrun simctl bootstatus "$udid" -b
  xcrun simctl ui "$udid" appearance light
  xcodebuild \
    -project "$PROJECT" \
    -scheme "$SCHEME" \
    -configuration Debug \
    -derivedDataPath "$DERIVED_DATA" \
    -destination "platform=iOS Simulator,id=$udid" \
    -resultBundlePath "$result_path" \
    -only-testing:"$TEST_TARGET" \
    CODE_SIGNING_ALLOWED=YES \
    CODE_SIGNING_REQUIRED=YES \
    CODE_SIGN_IDENTITY="${IRIS_DRIVE_IOS_CODE_SIGN_IDENTITY:--}" \
    PROVISIONING_PROFILE_SPECIFIER= \
    LIBRARY_SEARCH_PATHS="$RUST_LIB_DIR" \
    OTHER_LDFLAGS="$RUST_STATIC_LIB" \
    test-without-building
  extract_screenshots "$result_path" "$output_dir" "$export_dir"
done

echo "IOS_APP_STORE_SCREENSHOTS_OK"
