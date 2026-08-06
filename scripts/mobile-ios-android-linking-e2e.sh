#!/usr/bin/env bash
set -Eeuo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
MODE="${IRIS_DRIVE_MOBILE_PHYSICAL_LINKING:-auto}"
WAIT_SECS="${IRIS_DRIVE_MOBILE_LINK_WAIT_SECS:-15}"
CAMERA_WAIT_SECS="${IRIS_DRIVE_MOBILE_CAMERA_WAIT_SECS:-30}"
POST_LINK_WAIT_SECS="${IRIS_DRIVE_MOBILE_POST_LINK_WAIT_SECS:-45}"
REUSE_ANDROID_ARTIFACTS="${IRIS_DRIVE_MOBILE_REUSE_ANDROID_ARTIFACTS:-0}"
ISOLATED_DEVICE="${IRIS_DRIVE_MOBILE_LINK_ISOLATED_DEVICE:-0}"
ANDROID_PACKAGE="${IRIS_DRIVE_ANDROID_PHYSICAL_LINK_PACKAGE:-to.iris.drive.uitest}"
ANDROID_ACTIVITY="${IRIS_DRIVE_ANDROID_PHYSICAL_LINK_ACTIVITY:-$ANDROID_PACKAGE/to.iris.drive.app.MainActivity}"
ANDROID_APK="${IRIS_DRIVE_ANDROID_PHYSICAL_LINK_APK:-$ROOT/android/app/build/outputs/apk/uiTest/app-uiTest.apk}"
ANDROID_TEST_APK="${IRIS_DRIVE_ANDROID_PHYSICAL_LINK_TEST_APK:-$ROOT/android/app/build/outputs/apk/androidTest/uiTest/app-uiTest-androidTest.apk}"
ANDROID_TEST_RUNNER="${IRIS_DRIVE_ANDROID_PHYSICAL_LINK_TEST_RUNNER:-$ANDROID_PACKAGE.test/androidx.test.runner.AndroidJUnitRunner}"
DEBUG_ACTION_EXTRA="${IRIS_DRIVE_ANDROID_DEBUG_ACTION_EXTRA:-to.iris.drive.DEBUG_ACTION}"
IOS_PROJECT="$ROOT/ios/IrisDriveIOS.xcodeproj"
IOS_SCHEME="IrisDriveIOS"
IOS_CONFIGURATION="${IRIS_DRIVE_IOS_XCODE_CONFIGURATION:-Debug}"
IOS_BUNDLE_ID="${IRIS_DRIVE_IOS_BUNDLE_ID:-fi.siriusbusiness.drive}"
IOS_APP_GROUP_ID="${IRIS_DRIVE_IOS_APP_GROUP_ID:-${IRIS_DRIVE_IOS_APP_GROUP_IDENTIFIER:-group.fi.siriusbusiness.drive}}"
IOS_SHARE_SOURCE_BUNDLE_ID="${IRIS_DRIVE_IOS_SHARE_SOURCE_BUNDLE_ID:-$IOS_BUNDLE_ID.ShareSource}"
IOS_RUNNER_BUNDLE_ID="${IRIS_DRIVE_IOS_UI_TEST_RUNNER_BUNDLE_ID:-$IOS_BUNDLE_ID.UITests.xctrunner}"
IOS_DERIVED_DATA="${IRIS_DRIVE_IOS_PHYSICAL_LINK_DERIVED_DATA:-$ROOT/ios/.build/PhysicalLinkDerivedData}"
TARGET_DIR="${CARGO_TARGET_DIR:-$(cargo metadata --format-version 1 --no-deps | python3 -c 'import json,sys; print(json.load(sys.stdin)["target_directory"])')}"
IDRIVE="${IRIS_DRIVE_IDRIVE_BIN:-$TARGET_DIR/debug/idrive}"
RUST_IOS_TARGET="${IRIS_DRIVE_IOS_RUST_TARGET:-aarch64-apple-ios}"
RUST_LIB_DIR="$TARGET_DIR/$RUST_IOS_TARGET/debug"
RUST_STATIC_LIB="$RUST_LIB_DIR/libiris_drive_app_core.a"
RESULT_DIR="${IRIS_DRIVE_MOBILE_LINK_RESULT_DIR:-$ROOT/artifacts/mobile-linking-e2e}"
RUN_ID="$(date +%s)-$$-$RANDOM"
IOS_MARKER_NAME="iris-drive-physical-link-markers.log"
IOS_MARKERS=""
IOS_TEST_PID=""
IOS_TEST_LOG=""
IOS_SIGNAL_PREFIX=""
ANDROID_SERIAL_SELECTED=""
IOS_DEVICE_SELECTED=""
ADB=""
XCTESTRUN=""
TMP=""
IOS_TO_ANDROID_MS=""
ANDROID_TO_IOS_MS=""
IOS_TO_ANDROID_STARTED_MS=""
IOS_TO_ANDROID_FINISHED_MS=""
ANDROID_TO_IOS_STARTED_MS=""
ANDROID_TO_IOS_FINISHED_MS=""
IOS_TO_ANDROID_MANUAL_MS=""
ANDROID_TO_IOS_MANUAL_MS=""
IOS_TO_ANDROID_MANUAL_STARTED_MS=""
IOS_TO_ANDROID_MANUAL_FINISHED_MS=""
ANDROID_TO_IOS_MANUAL_STARTED_MS=""
ANDROID_TO_IOS_MANUAL_FINISHED_MS=""
IOS_OWNER_FILE="ios-owner-$RUN_ID.txt"
IOS_JOINER_FILE="ios-joiner-$RUN_ID.txt"
ANDROID_OWNER_FILE="android-owner-$RUN_ID.txt"
ANDROID_JOINER_FILE="android-joiner-$RUN_ID.txt"
IOS_MANUAL_OWNER_FILE="ios-manual-owner-$RUN_ID.txt"
IOS_MANUAL_JOINER_FILE="ios-manual-joiner-$RUN_ID.txt"
ANDROID_MANUAL_OWNER_FILE="android-manual-owner-$RUN_ID.txt"
ANDROID_MANUAL_JOINER_FILE="android-manual-joiner-$RUN_ID.txt"
IOS_OWNER_CONTENT="physical iOS owner post-link provider write $RUN_ID"
IOS_JOINER_CONTENT="physical iOS joiner post-link provider write $RUN_ID"
ANDROID_OWNER_CONTENT="physical Android owner post-link provider write $RUN_ID"
ANDROID_JOINER_CONTENT="physical Android joiner post-link provider write $RUN_ID"
IOS_MANUAL_OWNER_CONTENT="physical iOS manual owner post-link provider write $RUN_ID"
IOS_MANUAL_JOINER_CONTENT="physical iOS manual joiner post-link provider write $RUN_ID"
ANDROID_MANUAL_OWNER_CONTENT="physical Android manual owner post-link provider write $RUN_ID"
ANDROID_MANUAL_JOINER_CONTENT="physical Android manual joiner post-link provider write $RUN_ID"

fail() {
  printf 'physical iOS/Android linking e2e failed: %s\n' "$*" >&2
  exit 1
}

bool_true() {
  case "${1:-}" in
    1 | true | TRUE | True | yes | YES | Yes | on | ON | On | required) return 0 ;;
    *) return 1 ;;
  esac
}

monotonic_milliseconds() {
  perl -MTime::HiRes=clock_gettime,CLOCK_MONOTONIC \
    -e 'printf "%.0f\n", clock_gettime(CLOCK_MONOTONIC) * 1000'
}

sha256_file() {
  python3 - "$1" <<'PY'
import hashlib
import sys
with open(sys.argv[1], "rb") as handle:
    print(hashlib.sha256(handle.read()).hexdigest())
PY
}

sha256_text() {
  python3 -c 'import hashlib,sys; print(hashlib.sha256(sys.stdin.buffer.read()).hexdigest())'
}

base64_url() {
  python3 -c 'import base64,sys; print(base64.urlsafe_b64encode(sys.stdin.buffer.read()).decode().rstrip("="))'
}

emit_skip() {
  local reason="$1"
  mkdir -p "$RESULT_DIR"
  local summary="$RESULT_DIR/mobile-linking-$RUN_ID-skipped.json"
  python3 - "$MODE" "$reason" >"$summary" <<'PY'
import json
import sys
print(json.dumps({"ok": False, "skipped": True, "mode": sys.argv[1], "reason": sys.argv[2]}, sort_keys=True))
PY
  echo "MOBILE_PHYSICAL_LINKING_SKIPPED mode=$MODE reason=$reason summary=$summary"
}

cleanup() {
  local status=$?
  if [[ -n "$IOS_TEST_PID" ]] && kill -0 "$IOS_TEST_PID" >/dev/null 2>&1; then
    kill "$IOS_TEST_PID" >/dev/null 2>&1 || true
    wait "$IOS_TEST_PID" >/dev/null 2>&1 || true
  fi
  if [[ -n "$ADB" && -n "$ANDROID_SERIAL_SELECTED" ]]; then
    "$ADB" -s "$ANDROID_SERIAL_SELECTED" shell am force-stop "$ANDROID_PACKAGE" >/dev/null 2>&1 || true
    if [[ "${IRIS_DRIVE_MOBILE_LINK_KEEP_ANDROID_APP:-0}" != "1" ]]; then
      "$ADB" -s "$ANDROID_SERIAL_SELECTED" shell pm clear "$ANDROID_PACKAGE" >/dev/null 2>&1 || true
    fi
  fi
  if [[ -n "$TMP" && -d "$TMP" ]]; then
    rm -rf "$TMP"
  fi
  return "$status"
}
trap cleanup EXIT

case "$MODE" in
  0 | false | FALSE | False | no | NO | No | off | OFF | Off)
    MODE=disabled
    emit_skip explicit
    exit 0
    ;;
  auto | AUTO | Auto | "") MODE=auto ;;
  1 | true | TRUE | True | yes | YES | Yes | on | ON | On | required) MODE=required ;;
  *) fail "unsupported IRIS_DRIVE_MOBILE_PHYSICAL_LINKING=$MODE" ;;
esac

for timeout_name in WAIT_SECS CAMERA_WAIT_SECS POST_LINK_WAIT_SECS; do
  [[ "${!timeout_name}" =~ ^[1-9][0-9]*$ ]] \
    || fail "$timeout_name must be a positive integer"
done
if ((WAIT_SECS > 15)); then
  fail "IRIS_DRIVE_MOBILE_LINK_WAIT_SECS must be at most 15"
fi

resolve_adb() {
  local sdk="${ANDROID_HOME:-${ANDROID_SDK_ROOT:-}}"
  if [[ -z "$sdk" && -f "$ROOT/android/local.properties" ]]; then
    sdk="$(sed -n 's/^sdk\.dir=//p' "$ROOT/android/local.properties" | head -n 1)"
  fi
  if [[ -z "$sdk" && -d "$HOME/Library/Android/sdk" ]]; then
    sdk="$HOME/Library/Android/sdk"
  fi
  if [[ -n "$sdk" && -x "$sdk/platform-tools/adb" ]]; then
    printf '%s\n' "$sdk/platform-tools/adb"
  else
    command -v adb
  fi
}

select_physical_android() {
  local requested="${IRIS_DRIVE_ANDROID_SERIAL:-${ANDROID_SERIAL:-}}"
  local serial candidates="" devices
  devices="$("$ADB" devices)" || return 5
  while read -r serial _; do
    [[ -n "$serial" && "$serial" != "List" ]] || continue
    if [[ "$("$ADB" -s "$serial" get-state 2>/dev/null || true)" != "device" ]]; then
      continue
    fi
    if [[ "$("$ADB" -s "$serial" shell getprop ro.kernel.qemu 2>/dev/null | tr -d '\r')" == "1" ]]; then
      continue
    fi
    candidates+="${candidates:+$'\n'}$serial"
  done <<<"$devices"

  if [[ -n "$requested" ]]; then
    if printf '%s\n' "$candidates" | grep -Fxq "$requested"; then
      printf '%s\n' "$requested"
      return 0
    fi
    return 4
  fi
  local count
  count="$(printf '%s\n' "$candidates" | sed '/^$/d' | wc -l | tr -d ' ')"
  case "$count" in
    0) return 3 ;;
    1) printf '%s\n' "$candidates" ;;
    *) return 4 ;;
  esac
}

select_physical_ios() {
  local requested="${IRIS_DRIVE_IOS_DEVICE:-}"
  local devices_json xctrace_output
  devices_json="$TMP/ios-devices.json"
  xctrace_output="$TMP/ios-xctrace.txt"
  xcrun devicectl list devices --json-output "$devices_json" >/dev/null 2>&1 || return 5
  xcrun xctrace list devices >"$xctrace_output" 2>/dev/null || return 5
  python3 - "$requested" "$devices_json" "$xctrace_output" <<'PY'
import json
import sys

requested, devices_path, xctrace_path = sys.argv[1:]
with open(devices_path, "r", encoding="utf-8") as handle:
    devices = json.load(handle).get("result", {}).get("devices", [])
with open(xctrace_path, "r", encoding="utf-8") as handle:
    xctrace = handle.read()

online = xctrace.split("== Devices ==", 1)[-1].split("== Devices Offline ==", 1)[0]

def names(device):
    props = device.get("deviceProperties", {})
    connection = device.get("connectionProperties", {})
    values = [device.get("identifier"), props.get("name")]
    values.extend(connection.get("potentialHostnames", []))
    return {value for value in values if value}

available = []
for device in devices:
    product_type = device.get("hardwareProperties", {}).get("productType", "")
    connection = device.get("connectionProperties", {})
    name = device.get("deviceProperties", {}).get("name", "")
    if not product_type.startswith(("iPhone", "iPad")):
        continue
    if connection.get("pairingState") != "paired" or connection.get("tunnelState") == "unavailable":
        continue
    if not name or name not in online:
        continue
    available.append(device)

if requested:
    matches = [device for device in available if requested in names(device)]
    if len(matches) != 1:
        raise SystemExit(4)
    print(matches[0]["identifier"])
elif len(available) == 1:
    print(available[0]["identifier"])
elif not available:
    raise SystemExit(3)
else:
    raise SystemExit(4)
PY
}

redact_physical_identifiers() {
  python3 -u -c 'import re, sys
for text in sys.stdin:
    for value in sys.argv[1:]: text=text.replace(value, "<physical-device>")
    for key in ("allocation", "host", "name"):
        text=re.sub(r"(\\\"" + key + r"\\\"\\s*:\\s*)\\\"(?:\\\\.|[^\\\"])*\\\"", r"\\1\\\"<redacted>\\\"", text)
    sys.stdout.write(text)' "$@"
}

reserve_physical_devices() {
  bool_true "${IRIS_DRIVE_NATIVE_MOBILE_RESERVED:-0}" && return
  if [[ "${IRIS_DRIVE_LAB_ALLOCATED_ANDROID:-}" == "$ANDROID_SERIAL_SELECTED" \
    && "${IRIS_DRIVE_LAB_ALLOCATED_IOS:-}" == "$IOS_DEVICE_SELECTED" ]]; then
    return
  fi

  set +e
  python3 "$ROOT/scripts/native_lab.py" run \
    --resource iris-drive-mobile-ios-android-linking \
    --health "android:$ANDROID_SERIAL_SELECTED" \
    --health "ios-device:$IOS_DEVICE_SELECTED" \
    --allocation-env android=IRIS_DRIVE_LAB_ALLOCATED_ANDROID \
    --allocation-env ios-device=IRIS_DRIVE_LAB_ALLOCATED_IOS \
    -- env IRIS_DRIVE_NATIVE_MOBILE_RESERVED=1 "$0" "$@" 2>&1 \
    | redact_physical_identifiers "$ANDROID_SERIAL_SELECTED" "$IOS_DEVICE_SELECTED"
  local status="${PIPESTATUS[0]}"
  set -e
  exit "$status"
}

if [[ "$(uname -s)" != "Darwin" ]] || ! command -v xcrun >/dev/null 2>&1; then
  if [[ "$MODE" == "auto" ]]; then
    emit_skip macos_unavailable
    exit 0
  fi
  fail "physical linking requires macOS with Xcode command-line tools"
fi
if ! ADB="$(resolve_adb 2>/dev/null)"; then
  if [[ "$MODE" == "auto" ]]; then
    emit_skip adb_unavailable
    exit 0
  fi
  fail "physical linking requires adb"
fi

TMP="$(mktemp -d -t iris-drive-mobile-linking)"
mkdir -p "$RESULT_DIR"
IOS_MARKERS="$TMP/ios-markers.log"

set +e
ANDROID_SERIAL_SELECTED="$(select_physical_android)"
android_status=$?
set -e
if [[ "$android_status" -ne 0 ]]; then
  if [[ "$MODE" == "auto" && "$android_status" -eq 3 ]]; then
    emit_skip physical_android_unavailable
    exit 0
  fi
  fail "select exactly one online non-emulated Android device"
fi

set +e
IOS_DEVICE_SELECTED="$(select_physical_ios)"
ios_status=$?
set -e
if [[ "$ios_status" -ne 0 ]]; then
  if [[ "$MODE" == "auto" && "$ios_status" -eq 3 ]]; then
    emit_skip physical_ios_unavailable
    exit 0
  fi
  fail "select exactly one paired, online physical iOS device"
fi

if ! bool_true "$ISOLATED_DEVICE"; then
  if [[ "$MODE" == "auto" ]]; then
    emit_skip isolated_device_opt_in_required
    exit 0
  fi
  fail "set IRIS_DRIVE_MOBILE_LINK_ISOLATED_DEVICE=1 only for a reserved device; the real iOS app-group provider state is reset"
fi
reserve_physical_devices "$@"

assert_android_unlocked() {
  local window
  window="$("$ADB" -s "$ANDROID_SERIAL_SELECTED" shell dumpsys window 2>/dev/null || true)"
  if grep -Eq 'mDreamingLockscreen=true|isStatusBarKeyguard=true' <<<"$window"; then
    fail "unlock the Android device before running physical linking"
  fi
}

assert_ios_unlocked() {
  local display_json="$TMP/ios-display.json"
  xcrun devicectl device info displays \
    --device "$IOS_DEVICE_SELECTED" --json-output "$display_json" >/dev/null \
    || fail "could not inspect the physical iOS display"
  if python3 - "$display_json" <<'PY'
import json
import sys
with open(sys.argv[1], "r", encoding="utf-8") as handle:
    state = (json.load(handle).get("result") or {}).get("backlightState")
raise SystemExit(0 if state == "off" else 1)
PY
  then
    fail "wake and unlock the iOS device before running physical linking"
  fi
}

assert_android_unlocked
assert_ios_unlocked

android_artifacts_valid() {
  python3 - "$ANDROID_APK" "$ANDROID_TEST_APK" <<'PY'
from pathlib import Path
import sys
import zipfile

requirements = (
    (Path(sys.argv[1]), {"AndroidManifest.xml", "classes.dex", "lib/arm64-v8a/libiris_drive_app_core.so"}),
    (Path(sys.argv[2]), {"AndroidManifest.xml", "classes.dex"}),
)
for artifact, required in requirements:
    if not artifact.is_file() or artifact.stat().st_size == 0:
        raise SystemExit(1)
    try:
        with zipfile.ZipFile(artifact) as archive:
            if archive.testzip() is not None or not required.issubset(archive.namelist()):
                raise SystemExit(1)
    except (OSError, zipfile.BadZipFile):
        raise SystemExit(1)
PY
}

build_apps() {
  echo "[mobile-link] building physical Android and iOS test artifacts" >&2
  if bool_true "$REUSE_ANDROID_ARTIFACTS" && android_artifacts_valid; then
    echo "[mobile-link] reusing validated Android artifacts from the completed functional gate" >&2
  else
    "$ROOT/tools/run-android" :app:assembleUiTest :app:assembleUiTestAndroidTest
  fi
  android_artifacts_valid || fail "Android physical-link APKs are missing, corrupt, or incomplete"

  cargo build -p idrive
  [[ -x "$IDRIVE" ]] || fail "idrive was not built for exact provider-content verification"
  cargo build -p iris-drive-app-core --target "$RUST_IOS_TARGET"
  [[ -f "$RUST_STATIC_LIB" ]] || fail "iOS app-core static library was not built"
  xcodebuild \
    -project "$IOS_PROJECT" \
    -scheme "$IOS_SCHEME" \
    -configuration "$IOS_CONFIGURATION" \
    -derivedDataPath "$IOS_DERIVED_DATA" \
    -destination "platform=iOS,id=$IOS_DEVICE_SELECTED" \
    SWIFT_ACTIVE_COMPILATION_CONDITIONS=DEBUG \
    LIBRARY_SEARCH_PATHS="$RUST_LIB_DIR" \
    OTHER_LDFLAGS="$RUST_STATIC_LIB" \
    -allowProvisioningUpdates \
    -allowProvisioningDeviceRegistration \
    build-for-testing >"$RESULT_DIR/ios-physical-link-build.log"
  XCTESTRUN="$(find "$IOS_DERIVED_DATA/Build/Products" -name '*.xctestrun' -type f -print -quit)"
  [[ -n "$XCTESTRUN" && -f "$XCTESTRUN" ]] || fail "iOS physical-link xctestrun was not built"
}

copy_ios_markers() {
  rm -f "$IOS_MARKERS"
  xcrun devicectl device copy from \
    --device "$IOS_DEVICE_SELECTED" \
    --domain-type appDataContainer \
    --domain-identifier "$IOS_RUNNER_BUNDLE_ID" \
    --source "Documents/$IOS_MARKER_NAME" \
    --destination "$IOS_MARKERS" >/dev/null 2>&1
}

wait_for_ios_marker() {
  local marker="$1"
  local seconds="$2"
  local deadline=$((SECONDS + seconds))
  while (( SECONDS < deadline )); do
    if copy_ios_markers \
      && grep -Fxq "IRIS_XCUITEST_RUN_ID=$RUN_ID" "$IOS_MARKERS" \
      && grep -Fq "$marker" "$IOS_MARKERS"; then
      return 0
    fi
    if [[ -n "$IOS_TEST_PID" ]] && ! kill -0 "$IOS_TEST_PID" >/dev/null 2>&1; then
      return 1
    fi
    sleep 0.25
  done
  return 1
}

assert_ios_provider_write_marker() {
  local expected
  expected="$(printf '%s' "$1" | sha256_text)"
  wait_for_ios_marker "IRIS_IOS_PROVIDER_WRITE_SHA256=$expected" 10
}

start_ios_test() {
  local test_name="$1"
  local post_file="$2"
  local post_content="$3"
  local peer_file="$4"
  local manual_request="${5:-}"
  local run_file="$TMP/$test_name.xctestrun"
  IOS_SIGNAL_PREFIX="$test_name"
  cp "$XCTESTRUN" "$run_file"
  python3 - "$run_file" \
    "IRIS_XCUITEST_PHYSICAL_LINK_GATE=1" \
    "IRIS_XCUITEST_ISOLATED_DEVICE=1" \
    "IRIS_XCUITEST_RUN_ID=$RUN_ID" \
    "IRIS_XCUITEST_SIGNAL_PREFIX=$IOS_SIGNAL_PREFIX" \
    "IRIS_XCUITEST_DELIVERY_WAIT_SECS=$WAIT_SECS" \
    "IRIS_XCUITEST_CAMERA_WAIT_SECS=$CAMERA_WAIT_SECS" \
    "IRIS_XCUITEST_SHARE_SOURCE_BUNDLE_ID=$IOS_SHARE_SOURCE_BUNDLE_ID" \
    "IRIS_XCUITEST_POST_LINK_FILE=$post_file" \
    "IRIS_XCUITEST_POST_LINK_CONTENT_B64=$(printf '%s' "$post_content" | base64_url)" \
    "IRIS_XCUITEST_PEER_FILE=$peer_file" \
    "IRIS_XCUITEST_MANUAL_REQUEST_B64=$(printf '%s' "$manual_request" | base64_url)" <<'PY'
import plistlib
import sys

path = sys.argv[1]
updates = dict(value.split("=", 1) for value in sys.argv[2:])
with open(path, "rb") as handle:
    data = plistlib.load(handle)
matched = 0
for target in data.values():
    if not isinstance(target, dict) or not target.get("IsUITestBundle"):
        continue
    matched += 1
    for key in (
        "EnvironmentVariables",
        "TestingEnvironmentVariables",
        "UITargetAppEnvironmentVariables",
    ):
        target.setdefault(key, {}).update(updates)
if matched != 1:
    raise SystemExit(f"expected one UI test target, found {matched}")
with open(path, "wb") as handle:
    plistlib.dump(data, handle)
PY

  IOS_TEST_LOG="$RESULT_DIR/ios-$test_name.log"
  rm -f "$IOS_MARKERS"
  xcodebuild \
    -xctestrun "$run_file" \
    -destination "platform=iOS,id=$IOS_DEVICE_SELECTED" \
    -only-testing:"IrisDriveIOSUITests/IrisDrivePhysicalLinkingUITests/$test_name" \
    test-without-building >"$IOS_TEST_LOG" 2>&1 &
  IOS_TEST_PID="$!"
}

signal_ios_test() {
  [[ -n "$IOS_SIGNAL_PREFIX" ]] || fail "cannot signal iOS without an active XCTest"
  local name="$IOS_SIGNAL_PREFIX-$1"
  local source="$TMP/$name.signal"
  printf 'ready\n' >"$source"
  xcrun devicectl device copy to \
    --device "$IOS_DEVICE_SELECTED" \
    --domain-type appDataContainer \
    --domain-identifier "$IOS_RUNNER_BUNDLE_ID" \
    --source "$source" \
    --destination "Documents/iris-drive-physical-link-$RUN_ID-$name.signal" \
    --quiet >/dev/null
}

finish_ios_test() {
  local test_name="$1"
  local status=0
  wait "$IOS_TEST_PID" || status=$?
  IOS_TEST_PID=""
  IOS_SIGNAL_PREFIX=""
  if [[ "$status" -ne 0 ]]; then
    tail -n 120 "$IOS_TEST_LOG" >&2 || true
    fail "iOS physical XCTest $test_name failed"
  fi
  copy_ios_markers || fail "could not copy iOS physical XCTest markers"
  grep -Fxq "IRIS_XCUITEST_RUN_ID=$RUN_ID" "$IOS_MARKERS" \
    || fail "iOS physical XCTest marker belongs to another run"
  grep -Fxq "IRIS_XCUITEST_FINISHED=$test_name" "$IOS_MARKERS" \
    || fail "iOS physical XCTest $test_name executed zero matching tests"
  grep -Eq 'Executed 1 test' "$IOS_TEST_LOG" \
    || fail "iOS physical XCTest $test_name did not report Executed 1 test"
}

android_dump_ui() {
  "$ADB" -s "$ANDROID_SERIAL_SELECTED" shell uiautomator dump /sdcard/iris-physical-link.xml >/dev/null
  "$ADB" -s "$ANDROID_SERIAL_SELECTED" exec-out cat /sdcard/iris-physical-link.xml >"$TMP/android-ui.xml"
}

android_ui_point() {
  local kind="$1"
  local value="$2"
  android_dump_ui || return 1
  python3 "$ROOT/scripts/lib/android-ui-point.py" "$TMP/android-ui.xml" "$kind" "$value"
}

android_ui_has() {
  local kind="$1"
  local value="$2"
  android_dump_ui || return 1
  python3 - "$TMP/android-ui.xml" "$kind" "$value" <<'PY'
import sys
import xml.etree.ElementTree as ET
root = ET.parse(sys.argv[1]).getroot()
attribute = "text" if sys.argv[2] == "text" else "content-desc"
raise SystemExit(0 if any(node.attrib.get(attribute) == sys.argv[3] for node in root.iter("node")) else 1)
PY
}

wait_for_android_ui() {
  local kind="$1"
  local value="$2"
  local seconds="$3"
  local deadline=$((SECONDS + seconds))
  while (( SECONDS < deadline )); do
    if android_ui_has "$kind" "$value"; then
      return 0
    fi
    sleep 0.25
  done
  return 1
}

tap_android_ui() {
  local coordinates x y
  coordinates="$(android_ui_point "$1" "$2")" || return 1
  read -r x y <<<"$coordinates"
  [[ -n "$x" && -n "$y" ]] || return 1
  "$ADB" -s "$ANDROID_SERIAL_SELECTED" shell input tap "$x" "$y" >/dev/null
}

wait_and_tap_android_text() {
  local value="$1"
  local seconds="$2"
  wait_for_android_ui text "$value" "$seconds" || return 1
  tap_android_ui text "$value"
}

start_android_app() {
  "$ADB" -s "$ANDROID_SERIAL_SELECTED" shell am start -W -n "$ANDROID_ACTIVITY" >/dev/null
}

android_debug_action() {
  "$ADB" -s "$ANDROID_SERIAL_SELECTED" shell am start -W -n "$ANDROID_ACTIVITY" \
    --es "$DEBUG_ACTION_EXTRA" "$1" >/dev/null
}

prepare_android_fresh() {
  "$ADB" -s "$ANDROID_SERIAL_SELECTED" shell am force-stop "$ANDROID_PACKAGE" >/dev/null 2>&1 || true
  "$ADB" -s "$ANDROID_SERIAL_SELECTED" shell pm clear "$ANDROID_PACKAGE" >/dev/null
  start_android_app
}

create_android_owner_through_ui() {
  prepare_android_fresh
  wait_and_tap_android_text "Create profile" 15 || fail "Android Create profile UI was unavailable"
  wait_and_tap_android_text "Create profile" 10 || fail "Android profile form did not submit"
  wait_for_android_ui text "My Drive" 20 || fail "Android owner profile did not reach My Drive"
  android_debug_action start-sync
}

display_android_join_request_qr() {
  prepare_android_fresh
  wait_and_tap_android_text "Sign in" 15 || fail "Android Sign in UI was unavailable"
  wait_for_android_ui desc "Device approval request QR" 15 \
    || fail "Android shipped UI did not display its compact approval request QR"
  "$ADB" -s "$ANDROID_SERIAL_SELECTED" shell input keyevent KEYCODE_HOME >/dev/null
  start_android_app
  wait_for_android_ui desc "Device approval request QR" 10 \
    || fail "Android approval request QR did not survive background and resume"
  echo "IRIS_ANDROID_REQUEST_QR_RESUMED=1"
}

open_android_approval_camera() {
  open_android_manual_approval
  wait_and_tap_android_text "Scan QR" 10 || fail "Android shipped QR scanner action was unavailable"
  wait_for_android_ui desc "QR scanner camera" 10 \
    || fail "Android shipped QR camera did not become ready"
}

restart_android_and_assert_authorized() {
  "$ADB" -s "$ANDROID_SERIAL_SELECTED" shell am force-stop "$ANDROID_PACKAGE" >/dev/null
  start_android_app
  wait_for_android_authorized 15 || fail "Android authorization did not survive app restart"
  echo "IRIS_ANDROID_LIFECYCLE_RESUMED=1"
  android_debug_action start-sync
}

wait_for_android_authorized() {
  local seconds="$1"
  local deadline=$((SECONDS + seconds))
  while (( SECONDS < deadline )); do
    if android_ui_has text "My Drive" && ! android_ui_has text "Waiting for approval"; then
      return 0
    fi
    sleep 0.25
  done
  return 1
}

wait_for_android_provider_entry() {
  local expected="$1"
  local seconds="$2"
  local deadline=$((SECONDS + seconds))
  while (( SECONDS < deadline )); do
    android_debug_action start-sync
    sleep 0.5
    android_debug_action dump-provider-list
    if "$ADB" -s "$ANDROID_SERIAL_SELECTED" exec-out run-as "$ANDROID_PACKAGE" \
      cat files/debug-provider-list.json 2>/dev/null \
      | python3 -c 'import json,sys; expected=sys.argv[1]; entries=(json.load(sys.stdin).get("entries") or []); raise SystemExit(0 if any(e.get("path") == expected for e in entries) else 1)' \
        "$expected" >/dev/null 2>&1; then
      return 0
    fi
    sleep 0.5
  done
  return 1
}

copy_android_config() {
  "$ADB" -s "$ANDROID_SERIAL_SELECTED" exec-out run-as "$ANDROID_PACKAGE" \
    cat files/config.toml >"$1" 2>/dev/null
}

copy_ios_config() {
  local destination="$1"
  xcrun devicectl device copy from \
    --device "$IOS_DEVICE_SELECTED" \
    --domain-type appGroupDataContainer \
    --domain-identifier "$IOS_APP_GROUP_ID" \
    --source "IrisDrive/config.toml" \
    --destination "$destination" >/dev/null 2>&1
}

assert_pending_receipts_cleared() {
  python3 - "$1" <<'PY'
import sys
import tomllib
with open(sys.argv[1], "rb") as handle:
    config = tomllib.load(handle)
receipts = (config.get("profile") or {}).get("pending_device_approval_receipts") or []
raise SystemExit(0 if not receipts else 1)
PY
}

assert_authorized_config() {
  python3 - "$1" <<'PY'
import sys
import tomllib
with open(sys.argv[1], "rb") as handle:
    config = tomllib.load(handle)
profile = config.get("profile") or {}
raise SystemExit(0 if profile.get("authorization_state") == "authorized" else 1)
PY
}

wait_for_empty_receipts() {
  local platform="$1"
  local seconds="$2"
  local config="$TMP/$platform-owner-config.toml"
  local deadline=$((SECONDS + seconds))
  while (( SECONDS < deadline )); do
    rm -f "$config"
    if [[ "$platform" == "ios" ]]; then
      copy_ios_config "$config" || true
    else
      copy_android_config "$config" || true
    fi
    if [[ -s "$config" ]] && assert_pending_receipts_cleared "$config"; then
      return 0
    fi
    sleep 0.25
  done
  return 1
}

assert_ios_provider_content() {
  local expected_name="$1"
  local expected_content="$2"
  local copied="$TMP/ios-provider-$expected_name"
  local output="$TMP/ios-provider-read-$expected_name"
  local expected="$TMP/ios-provider-expected-$expected_name"
  rm -rf "$copied"
  mkdir -p "$copied"
  xcrun devicectl device copy from \
    --device "$IOS_DEVICE_SELECTED" \
    --domain-type appGroupDataContainer \
    --domain-identifier "$IOS_APP_GROUP_ID" \
    --source IrisDrive \
    --destination "$copied" \
    --quiet >/dev/null \
    || return 1
  local config
  config="$(find "$copied" -name config.toml -type f -print -quit)"
  [[ -n "$config" ]] || return 1
  printf '%s' "$expected_content" >"$expected"
  "$IDRIVE" --config-dir "$(dirname "$config")" provider read "$expected_name" "$output" \
    >/dev/null 2>&1 \
    || return 1
  cmp -s "$expected" "$output" || return 1
  echo "ios_provider_read_sha256=$(sha256_file "$output")" >&2
}

run_android_provider_test() {
  local method="$1"
  local name="$2"
  local content="$3"
  local log="$RESULT_DIR/android-$method-$name.log"
  "$ADB" -s "$ANDROID_SERIAL_SELECTED" shell am instrument -w -r \
    -e class "to.iris.drive.app.provider.IrisDrivePhysicalLinkingProviderTest#$method" \
    -e file_name "$name" \
    -e content_b64 "$(printf '%s' "$content" | base64_url)" \
    -e wait_millis "$((POST_LINK_WAIT_SECS * 1000))" \
    "$ANDROID_TEST_RUNNER" >"$log" 2>&1 \
    || { tail -n 120 "$log" >&2; return 1; }
  grep -Eq 'OK \(1 test\)|IRIS_ANDROID_PROVIDER_(WRITE|READ)_SHA256=' "$log"
}

write_android_provider_file() {
  run_android_provider_test writePostLinkFileThroughDocumentsProvider "$1" "$2" \
    || fail "Android DocumentsProvider could not write $1"
  start_android_app
  android_debug_action start-sync
}

assert_android_provider_content() {
  run_android_provider_test readPostLinkFileThroughDocumentsProvider "$1" "$2" \
    || fail "Android DocumentsProvider content did not match for $1"
}

exchange_post_link_provider_files() {
  local ios_file="$1"
  local ios_content="$2"
  local android_file="$3"
  local android_content="$4"
  signal_ios_test peer-authorized \
    || fail "could not coordinate iOS provider write after observed authorization"
  wait_for_ios_marker "IRIS_IOS_HOST_AUTHORIZATION_OBSERVED=1" 10 \
    || fail "iOS did not observe the peer authorization checkpoint"
  wait_for_ios_marker "IRIS_IOS_POST_LINK_FILE_READY=1" 25 \
    || fail "iOS did not complete its post-link provider write"
  assert_ios_provider_write_marker "$ios_content" \
    || fail "iOS provider write hash evidence did not match the source content"
  wait_for_android_provider_entry "$ios_file" "$POST_LINK_WAIT_SECS" \
    || fail "Android provider did not receive iOS post-link file $ios_file"
  assert_android_provider_content "$ios_file" "$ios_content"

  write_android_provider_file "$android_file" "$android_content"
  signal_ios_test peer-file-written \
    || fail "could not tell iOS to verify the Android provider write"
  wait_for_ios_marker "IRIS_IOS_PEER_FILE_VISIBLE=1" "$POST_LINK_WAIT_SECS" \
    || fail "iOS Files provider did not expose Android post-link file $android_file"
}

assert_delivery_bound() {
  local label="$1"
  local started="$2"
  local finished="$3"
  local elapsed=$((finished - started))
  if [[ "$elapsed" -lt 0 || "$elapsed" -gt $((WAIT_SECS * 1000)) ]]; then
    fail "$label approval delivery took ${elapsed}ms; maximum is $((WAIT_SECS * 1000))ms"
  fi
  printf '%s\n' "$elapsed"
}

run_ios_owner_android_joiner() {
  echo "[mobile-link] direction iOS owner -> Android joiner" >&2
  display_android_join_request_qr
  start_ios_test \
    testIosOwnerApprovesAndroidThroughPhysicalCamera \
    "$IOS_OWNER_FILE" \
    "$IOS_OWNER_CONTENT" \
    "$ANDROID_JOINER_FILE"
  wait_for_ios_marker "IRIS_IOS_CAMERA_READY=1" 30 \
    || fail "iOS shipped camera did not become ready"
  wait_for_ios_marker "IRIS_IOS_MANUAL_ENTRY_READY=1" 2 \
    || fail "iOS shipped manual approval entry was not covered before camera approval"
  wait_for_ios_marker "IRIS_IOS_LIFECYCLE_RESUMED=1" 2 \
    || fail "iOS owner state did not survive background/restart coverage"
  echo "Aim the unlocked iOS camera at the Android request QR (up to ${CAMERA_WAIT_SECS}s)." >&2
  wait_for_ios_marker "IRIS_IOS_APPROVAL_CONFIRMATION_READY=1" "$((CAMERA_WAIT_SECS + 5))" \
    || fail "iOS camera did not scan Android's compact request"
  local android_config="$TMP/android-joiner-config.toml"
  IOS_TO_ANDROID_STARTED_MS="$(monotonic_milliseconds)"
  signal_ios_test submit-camera-approval \
    || fail "could not release iOS camera approval after starting the host timer"
  wait_for_android_authorized "$WAIT_SECS" \
    || fail "Android stayed on Waiting for approval after iOS confirmed the scan"
  IOS_TO_ANDROID_FINISHED_MS="$(monotonic_milliseconds)"
  IOS_TO_ANDROID_MS="$(assert_delivery_bound \
    "iOS-to-Android" "$IOS_TO_ANDROID_STARTED_MS" "$IOS_TO_ANDROID_FINISHED_MS")"
  restart_android_and_assert_authorized
  exchange_post_link_provider_files \
    "$IOS_OWNER_FILE" "$IOS_OWNER_CONTENT" \
    "$ANDROID_JOINER_FILE" "$ANDROID_JOINER_CONTENT"
  wait_for_empty_receipts ios "$WAIT_SECS" \
    || fail "iOS owner retained its durable approval receipt after Android applied it"
  finish_ios_test testIosOwnerApprovesAndroidThroughPhysicalCamera
  copy_android_config "$android_config" || fail "could not read Android joiner config"
  assert_authorized_config "$android_config" || fail "Android did not persist authorized state"
  assert_ios_provider_content "$ANDROID_JOINER_FILE" "$ANDROID_JOINER_CONTENT" \
    || fail "iOS provider content did not match Android's post-link write"
}

run_android_owner_ios_joiner() {
  echo "[mobile-link] direction Android owner -> iOS joiner" >&2
  create_android_owner_through_ui
  start_ios_test \
    testIosJoinerDisplaysPhysicalQrAndBecomesAuthorized \
    "$IOS_JOINER_FILE" \
    "$IOS_JOINER_CONTENT" \
    "$ANDROID_OWNER_FILE"
  wait_for_ios_marker "IRIS_IOS_REQUEST_QR_READY=1" 30 \
    || fail "iOS shipped UI did not display its compact request QR"
  wait_for_ios_marker "IRIS_IOS_LIFECYCLE_RESUMED=1" 2 \
    || fail "iOS request QR did not survive background and resume"
  open_android_approval_camera
  echo "Aim the unlocked Android camera at the iOS request QR (up to ${CAMERA_WAIT_SECS}s)." >&2
  wait_for_android_ui text "Approve this device?" "$CAMERA_WAIT_SECS" \
    || fail "Android camera did not scan iOS's compact request"
  local ios_config="$TMP/ios-joiner-config.toml"
  ANDROID_TO_IOS_STARTED_MS="$(monotonic_milliseconds)"
  tap_android_ui text "Approve" || fail "Android approval confirmation could not be tapped"
  wait_for_ios_marker "IRIS_IOS_AUTHORIZED=1" "$WAIT_SECS" \
    || fail "iOS stayed on Waiting for approval after Android confirmed the scan"
  ANDROID_TO_IOS_FINISHED_MS="$(monotonic_milliseconds)"
  ANDROID_TO_IOS_MS="$(assert_delivery_bound \
    "Android-to-iOS" "$ANDROID_TO_IOS_STARTED_MS" "$ANDROID_TO_IOS_FINISHED_MS")"
  restart_android_and_assert_authorized
  exchange_post_link_provider_files \
    "$IOS_JOINER_FILE" "$IOS_JOINER_CONTENT" \
    "$ANDROID_OWNER_FILE" "$ANDROID_OWNER_CONTENT"
  wait_for_empty_receipts android "$WAIT_SECS" \
    || fail "Android owner retained its durable approval receipt after iOS applied it"
  finish_ios_test testIosJoinerDisplaysPhysicalQrAndBecomesAuthorized
  copy_ios_config "$ios_config" || fail "could not read iOS joiner config"
  assert_authorized_config "$ios_config" || fail "iOS did not persist authorized state"
  assert_ios_provider_content "$ANDROID_OWNER_FILE" "$ANDROID_OWNER_CONTENT" \
    || fail "iOS provider content did not match Android's post-link write"
}

source "$ROOT/scripts/lib/mobile-ios-android-manual-linking.sh"

build_apps
"$ADB" -s "$ANDROID_SERIAL_SELECTED" install -r -g "$ANDROID_APK" >/dev/null
"$ADB" -s "$ANDROID_SERIAL_SELECTED" install -r -t "$ANDROID_TEST_APK" >/dev/null

start_ios_test testPhysicalEnvironmentBridgeIsReady preflight.txt preflight none
wait_for_ios_marker "IRIS_XCUITEST_ENVIRONMENT_READY=1" 30 \
  || fail "enable UI Automation on the unlocked physical iOS device"
finish_ios_test testPhysicalEnvironmentBridgeIsReady

run_ios_owner_android_joiner
run_android_owner_ios_joiner
run_ios_owner_android_joiner_manual
run_android_owner_ios_joiner_manual

SUMMARY="$RESULT_DIR/mobile-linking-$RUN_ID.json"
write_mobile_linking_summary "$SUMMARY"

echo "MOBILE_PHYSICAL_LINKING_OK"
echo "delivery_measurement_clock=host_monotonic_ms"
echo "ios_owner_to_android_joiner_ms=$IOS_TO_ANDROID_MS"
echo "ios_owner_to_android_joiner_interval=${IOS_TO_ANDROID_STARTED_MS}..${IOS_TO_ANDROID_FINISHED_MS} start=host_released_approval_submission finish=host_observed_authorization"
echo "android_owner_to_ios_joiner_ms=$ANDROID_TO_IOS_MS"
echo "android_owner_to_ios_joiner_interval=${ANDROID_TO_IOS_STARTED_MS}..${ANDROID_TO_IOS_FINISHED_MS} start=host_began_approval_submission finish=host_observed_authorization"
echo "ios_owner_to_android_joiner_manual_ms=$IOS_TO_ANDROID_MANUAL_MS"
echo "ios_owner_to_android_joiner_manual_interval=${IOS_TO_ANDROID_MANUAL_STARTED_MS}..${IOS_TO_ANDROID_MANUAL_FINISHED_MS} start=host_released_approval_submission finish=host_observed_authorization"
echo "android_owner_to_ios_joiner_manual_ms=$ANDROID_TO_IOS_MANUAL_MS"
echo "android_owner_to_ios_joiner_manual_interval=${ANDROID_TO_IOS_MANUAL_STARTED_MS}..${ANDROID_TO_IOS_MANUAL_FINISHED_MS} start=host_began_approval_submission finish=host_observed_authorization"
echo "summary=$SUMMARY"
