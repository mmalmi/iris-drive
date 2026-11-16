#!/usr/bin/env bash

set -Eeuo pipefail

case "$(uname -s)" in
  Darwin) ;;
  *)
    echo "iOS device Iris Apps smoke is Darwin-only; skipping on $(uname -s)"
    exit 0
    ;;
esac

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
PROJECT="$ROOT/ios/IrisDriveIOS.xcodeproj"
SCHEME="IrisDriveIOS"
CONFIGURATION="${IRIS_DRIVE_IOS_XCODE_CONFIGURATION:-Release}"
DERIVED_DATA="$ROOT/ios/.build/DeviceIrisAppsDerivedData"
BUNDLE_ID="${IRIS_DRIVE_IOS_BUNDLE_ID:-fi.siriusbusiness.drive}"
DEVICE_SELECTOR="${IRIS_DRIVE_IOS_DEVICE:-}"
DEVELOPMENT_TEAM="${IRIS_DRIVE_IOS_DEVELOPMENT_TEAM:-8G9P8AN75D}"
CODE_SIGN_IDENTITY="${IRIS_DRIVE_IOS_CODE_SIGN_IDENTITY:-Apple Development}"
ALLOW_SCREEN_OFF="${IRIS_DRIVE_IOS_ALLOW_SCREEN_OFF:-0}"
TARGET_DIR="${CARGO_TARGET_DIR:-$(cargo metadata --format-version 1 --no-deps | python3 -c 'import json,sys; print(json.load(sys.stdin)["target_directory"])')}"
RUST_IOS_TARGET="${IRIS_DRIVE_IOS_RUST_TARGET:-aarch64-apple-ios}"
RUST_LIB_DIR="$TARGET_DIR/$RUST_IOS_TARGET/debug"
RUST_STATIC_LIB="$RUST_LIB_DIR/libiris_drive_app_core.a"
BUILD_LOG="${IRIS_DRIVE_IOS_DEVICE_IRIS_APPS_BUILD_LOG:-/tmp/iris-drive-ios-device-iris-apps-build.log}"
PROBE_TIMEOUT_MS="${IRIS_DRIVE_IOS_DEVICE_IRIS_APPS_PROBE_TIMEOUT_MS:-30000}"
MAX_OPEN_MS="${IRIS_DRIVE_IOS_DEVICE_IRIS_APPS_MAX_OPEN_MS:-40000}"
DEBUG_REFRESH_DELAY_MS="${IRIS_DRIVE_IOS_DEVICE_IRIS_APPS_REFRESH_DELAY_MS:-1500}"
WEBVIEW_SETTLE_MS="${IRIS_DRIVE_DEBUG_WEBVIEW_SETTLE_MS:-10000}"
DEBUG_PROBE_HTTP="${IRIS_DRIVE_DEBUG_PROBE_HTTP:-1}"
PROBE_ID="iris-apps-device-$(date +%s)-$$"
PROBE_BASE_DIR="iris-drive-device-iris-apps-smoke-$PROBE_ID"

select_device() {
  local devices_json
  local status
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
    values = [device.get("identifier"), device.get("hardwareProperties", {}).get("udid"), props.get("name")]
    values.extend(connection.get("potentialHostnames", []))
    return {value for value in values if value}

for device in devices:
    if preferred and preferred not in names(device):
        continue
    if not is_usable_iphone(device):
        if preferred:
            raise SystemExit(f"selected iOS device is not usable for install: {preferred}")
        continue
    udid = device.get("hardwareProperties", {}).get("udid")
    if not isinstance(udid, str) or not udid.strip():
        raise SystemExit("selected iOS device has no hardware UDID for Xcode")
    print(udid.strip())
    raise SystemExit(0)
raise SystemExit(f"iOS device not found: {preferred}" if preferred else "no paired available iPhone found")
PY
  status=$?
  rm -f "$devices_json"
  return "$status"
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
    echo "FAIL: selected iOS device screen is off; wake and unlock the device before running physical Iris Apps smoke." >&2
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

poll_probe_result() {
  local device_udid="$1"
  local destination="$2"
  local deadline
  deadline=$((SECONDS + (PROBE_TIMEOUT_MS / 1000) + 10))
  while (( SECONDS < deadline )); do
    rm -f "$destination"
    if xcrun devicectl device copy from \
      --device "$device_udid" \
      --domain-type appDataContainer \
      --domain-identifier "$BUNDLE_ID" \
      --source Documents/debug-iris-apps-probe.json \
      --destination "$destination" >/dev/null 2>&1; then
      if python3 - "$destination" "$PROBE_ID" <<'PY'
import json
import sys
from urllib.parse import urlparse

with open(sys.argv[1], "r", encoding="utf-8") as handle:
    data = json.load(handle)
raise SystemExit(0 if data.get("probe_id") == sys.argv[2] else 1)
PY
      then
        return 0
      fi
    fi
    sleep 0.5
  done
  return 1
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

if ! xcodebuild \
  -project "$PROJECT" \
  -scheme "$SCHEME" \
  -configuration "$CONFIGURATION" \
  -derivedDataPath "$DERIVED_DATA" \
  -destination "generic/platform=iOS" \
  DEVELOPMENT_TEAM="$DEVELOPMENT_TEAM" \
  CODE_SIGN_STYLE=Automatic \
  CODE_SIGN_IDENTITY="$CODE_SIGN_IDENTITY" \
  SWIFT_ACTIVE_COMPILATION_CONDITIONS="${IRIS_DRIVE_IOS_DEVICE_IRIS_APPS_SWIFT_CONDITIONS:-DEBUG}" \
  LIBRARY_SEARCH_PATHS="$RUST_LIB_DIR" \
  OTHER_LDFLAGS="$RUST_STATIC_LIB" \
  -allowProvisioningUpdates \
  -allowProvisioningDeviceRegistration \
  build >"$BUILD_LOG"; then
  echo "FAIL: iOS device Iris Apps app build failed. Build log: $BUILD_LOG" >&2
  tail -n 120 "$BUILD_LOG" >&2 || true
  exit 1
fi

APP_PATH="$(resolve_app_path)"
if [[ -z "$APP_PATH" || ! -d "$APP_PATH" ]]; then
  echo "FAIL: built iOS app not found under $DERIVED_DATA" >&2
  exit 1
fi

xcrun devicectl device install app --device "$DEVICE_UDID" "$APP_PATH" >/dev/null
probe_dir="$(mktemp -d "${TMPDIR:-/tmp}/iris-apps-probe.XXXXXX")"
probe_launch_json="$probe_dir/launch.json"
probe_json="$probe_dir/probe.json"
echo "probe_launch_receipt=$probe_launch_json"
echo "probe_result=$probe_json"

launch_probe_console() {
  exec xcrun devicectl device process launch \
    --device "$DEVICE_UDID" \
    --console --json-output "$probe_launch_json" --timeout "$((PROBE_TIMEOUT_MS / 1000 + 120))" \
    --terminate-existing \
    --environment-variables "{\"IRIS_DRIVE_UI_TEST_BASE_DIR\":\"__TMP__/$PROBE_BASE_DIR\",\"IRIS_DRIVE_DEBUG_ACTION\":\"probe-iris-apps\",\"IRIS_DRIVE_DEBUG_REFRESH_DELAY_MS\":\"$DEBUG_REFRESH_DELAY_MS\",\"IRIS_DRIVE_DEBUG_PROBE_TIMEOUT_MS\":\"$PROBE_TIMEOUT_MS\",\"IRIS_DRIVE_DEBUG_PROBE_ID\":\"$PROBE_ID\",\"IRIS_DRIVE_DEBUG_WEBVIEW_SETTLE_MS\":\"$WEBVIEW_SETTLE_MS\",\"IRIS_DRIVE_DEBUG_PROBE_HTTP\":\"$DEBUG_PROBE_HTTP\"}" \
    "$BUNDLE_ID"
}

finish_probe() {
  local status="$1" console_status=0 sent_term=0
  trap - EXIT
  trap '' HUP INT TERM
  if [[ -z "$probe_console_pid" ]]; then
    probe_console_pid="$(jobs -p %launch_probe_console 2>/dev/null || true)"
    [[ -n "$probe_console_pid" ]] || exit "$status"
  fi
  # devicectl forwards signals to the attached app. A completed shell job can
  # never select a replacement app, even if iOS has recycled its PID.
  if kill -TERM %launch_probe_console 2>/dev/null; then sent_term=1; fi
  wait "$probe_console_pid" || console_status=$?
  if (( console_status != 0 && !(sent_term && console_status == 143) )); then
    echo "FAIL: device probe console exited with status $console_status (see $probe_dir/console.log)" >&2
    if (( status == 0 )); then status=1; fi
  fi
  if ! python3 - "$probe_launch_json" "$PROBE_ID" <<'PYRECEIPT'
import json
import sys

with open(sys.argv[1], encoding="utf-8") as handle:
    receipt = json.load(handle)
if (receipt.get("info") or {}).get("outcome") != "success":
    raise SystemExit("FAIL: devicectl did not complete the probe console session")
result = receipt.get("result") or {}
pid = (result.get("process") or {}).get("processIdentifier")
launch_env = (result.get("launchOptions") or {}).get("environmentVariables") or {}
if type(pid) is not int or pid <= 0 or launch_env.get("IRIS_DRIVE_DEBUG_PROBE_ID") != sys.argv[2]:
    raise SystemExit("FAIL: devicectl did not return the launched probe PID and matching probe ID")
print(f"probe_pid={pid}")
PYRECEIPT
  then
    if (( status == 0 )); then status=1; fi
  fi
  if (( status == 0 )); then
    echo "IOS_DEVICE_IRIS_APPS_SMOKE_OK"
    echo "build_log=$BUILD_LOG"
  fi
  exit "$status"
}

probe_console_pid=""
trap 'finish_probe "$?"' EXIT
trap 'exit 129' HUP
trap 'exit 130' INT
trap 'exit 143' TERM
launch_probe_console >"$probe_dir/console.log" 2>&1 &
probe_console_pid=$!

if ! poll_probe_result "$DEVICE_UDID" "$probe_json"; then
  echo "FAIL: device Iris Apps probe did not write a fresh result." >&2
  exit 1
fi
probe_screenshot_source="$(python3 - "$probe_json" <<'PY'
import json
import re
import sys
from pathlib import PurePosixPath

with open(sys.argv[1], "r", encoding="utf-8") as handle:
    data = json.load(handle)
name = PurePosixPath(data.get("webview_screenshot_path") or "").name
if not re.fullmatch(r"debug-iris-apps-webview-[0-9A-Fa-f-]{36}\.png", name):
    raise SystemExit(f"FAIL: device Iris Apps probe did not capture a fresh screenshot: {data}")
print(f"Documents/{name}")
PY
)"
probe_screenshot="$probe_dir/webview.png"
if xcrun devicectl device copy from \
  --device "$DEVICE_UDID" \
  --domain-type appDataContainer \
  --domain-identifier "$BUNDLE_ID" \
  --source "$probe_screenshot_source" \
  --destination "$probe_screenshot" >/dev/null 2>&1; then
  echo "webview_screenshot=$probe_screenshot"
else
  rm -f "$probe_screenshot"
  echo "FAIL: device Iris Apps WebView screenshot was not copied." >&2
  exit 1
fi

python3 - "$probe_json" "$MAX_OPEN_MS" <<'PY'
import json
import sys
from urllib.parse import urlparse

with open(sys.argv[1], "r", encoding="utf-8") as handle:
    data = json.load(handle)
max_open_ms = int(sys.argv[2])
if not data.get("opened"):
    raise SystemExit(f"FAIL: device Iris Apps probe did not open: {data}")
if not data.get("webview_loaded"):
    raise SystemExit(f"FAIL: device Iris Apps WebView did not load: {data}")
if data.get("webview_ready_state") != "complete":
    raise SystemExit(f"FAIL: device Iris Apps WebView document is not complete: {data}")
if data.get("webview_title") != "Iris: The Freedom Toolkit":
    raise SystemExit(f"FAIL: device Iris Apps WebView title is not the launcher: {data}")
if (data.get("webview_error") or "") or (data.get("webview_error_domain") or "") or int(data.get("webview_error_code") or 0):
    raise SystemExit(f"FAIL: device Iris Apps WebView retained a navigation error: {data}")
if not (data.get("webview_screenshot_path") or "") or (data.get("webview_screenshot_error") or ""):
    raise SystemExit(f"FAIL: device Iris Apps WebView did not persist a clean screenshot: {data}")
required_http_200 = [
    ("route_http_ok", "route_http_status_code", "route_http"),
    ("route_proxy_http_ok", "route_proxy_http_status_code", "route_proxy_http"),
    ("embedded_route_http_ok", "embedded_route_http_status_code", "embedded_route_http"),
    ("embedded_resolve_http_ok", "embedded_resolve_http_status_code", "embedded_resolve_http"),
    ("embedded_cached_resolve_http_ok", "embedded_cached_resolve_http_status_code", "embedded_cached_resolve_http"),
    ("hashtree_status_ok", "hashtree_status_code", "hashtree_status"),
]
for ok_key, status_key, label in required_http_200:
    if not data.get(ok_key) or int(data.get(status_key) or 0) != 200:
        raise SystemExit(f"FAIL: device Iris Apps {label} probe was not HTTP 200: {data}")
if not data.get("gateway_status_present") or not data.get("gateway_status_running"):
    raise SystemExit(f"FAIL: device Iris Apps gateway status is not running: {data}")
if data.get("gateway_status_state") != "running" or (data.get("gateway_status_error") or ""):
    raise SystemExit(f"FAIL: device Iris Apps gateway retained an error state: {data}")
body_text = (data.get("webview_body_text") or "").strip()
body_lower = body_text.lower()
failure_markers = [
    "resolution timeout",
    "not found",
    "failed to load",
    "iris apps failed",
    "no connection to server",
]
for marker in failure_markers:
    if marker in body_lower:
        raise SystemExit(f"FAIL: device Iris Apps WebView rendered failure text {marker!r}: {data}")
route_host = (data.get("route_host") or "").lower()
final_host = (urlparse(data.get("webview_final_url") or "").hostname or "").lower()
for label, host in (("route", route_host), ("final WebView", final_host)):
    if not (
        host.endswith(".iris.localhost")
        or host.endswith(".hash.localhost")
    ):
        raise SystemExit(f"FAIL: {label} host is not an isolated Iris-local subdomain: {data}")
    if host in {"iris.localhost", "nhash.iris.localhost", "localhost", "127.0.0.1", "::1"}:
        raise SystemExit(f"FAIL: {label} host collapsed to a shared loopback origin: {data}")
elapsed = int(data.get("user_visible_elapsed_ms") or data.get("elapsed_ms") or 0)
if elapsed > max_open_ms:
    raise SystemExit(
        f"FAIL: device Iris Apps probe opened too slowly: {elapsed}ms > {max_open_ms}ms"
    )
webview_elapsed = int(data.get("webview_elapsed_ms") or 0)
html_length = int(data.get("webview_html_length") or 0)
if html_length < 1000:
    raise SystemExit(f"FAIL: device Iris Apps WebView rendered too little HTML to be the launcher: {data}")
html_prefix = (data.get("webview_html_prefix") or "").lower()
if 'id="app"' not in html_prefix and "id='app'" not in html_prefix:
    raise SystemExit(f"FAIL: device Iris Apps WebView did not materialize the app root: {data}")
launcher_markers = ["drive", "chat", "contacts"]
missing_markers = [marker for marker in launcher_markers if marker not in body_lower]
if missing_markers:
    raise SystemExit(
        f"FAIL: device Iris Apps WebView did not render launcher markers {missing_markers}: {data}"
    )
print(f"opened_ms={elapsed} webview_ms={webview_elapsed}")
PY
