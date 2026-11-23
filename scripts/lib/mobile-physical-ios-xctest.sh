#!/usr/bin/env bash
# Physical-device XCUITest marker and runner readiness helpers.

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

wait_for_ios_runner_or_marker() {
  local marker="$1"
  local seconds="$2"
  local processes="$TMP/ios-processes.json"
  local deadline=$((SECONDS + seconds))
  while (( SECONDS < deadline )); do
    if copy_ios_markers \
      && grep -Fxq "IRIS_XCUITEST_RUN_ID=$RUN_ID" "$IOS_MARKERS" \
      && grep -Fq "$marker" "$IOS_MARKERS"; then
      return 0
    fi
    if xcrun devicectl device info processes \
      --device "$IOS_DEVICE_SELECTED" --json-output "$processes" >/dev/null 2>&1 \
      && python3 - "$processes" <<'PY'
import json
import sys

with open(sys.argv[1], "r", encoding="utf-8") as handle:
    result = json.load(handle).get("result") or {}
processes = result.get("runningProcesses") or result.get("processes") or []
running = any(
    str(process.get("executable") or "").endswith("/IrisDriveIOSUITests-Runner")
    for process in processes
)
raise SystemExit(0 if running else 1)
PY
    then
      return 0
    fi
    if [[ -n "$IOS_TEST_PID" ]] && ! kill -0 "$IOS_TEST_PID" >/dev/null 2>&1; then
      return 1
    fi
    sleep 0.5
  done
  return 1
}

configure_physical_xctestrun() {
  local run_file="$1" post_file="$2" post_content="$3" peer_file="$4" manual_request="$5"
  python3 - "$run_file" \
    "IRIS_XCUITEST_PHYSICAL_LINK_GATE=1" \
    "IRIS_XCUITEST_ISOLATED_DEVICE=1" \
    "IRIS_XCUITEST_RUN_ID=$RUN_ID" \
    "IRIS_XCUITEST_FRAME_SHA256=${QR_FRAME_SHA256:-}" \
    "IRIS_XCUITEST_SIGNAL_PREFIX=$IOS_SIGNAL_PREFIX" \
    "IRIS_XCUITEST_DELIVERY_WAIT_SECS=$WAIT_SECS" \
    "IRIS_XCUITEST_CAMERA_WAIT_SECS=$CAMERA_WAIT_SECS" \
    "IRIS_XCUITEST_SHARE_SOURCE_BUNDLE_ID=$IOS_SHARE_SOURCE_BUNDLE_ID" \
    "IRIS_XCUITEST_POST_LINK_FILE=$post_file" \
    "IRIS_XCUITEST_POST_LINK_CONTENT_B64=$(printf '%s' "$post_content" | base64_url)" \
    "IRIS_XCUITEST_PEER_FILE=$peer_file" \
    "IRIS_XCUITEST_MANUAL_REQUEST_B64=$(printf '%s' "$manual_request" | base64_url)" <<'PY'
import os
from pathlib import Path
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
    app_dir = Path(target["UITargetAppPath"].replace("__TESTROOT__", str(Path(path).parent)))
    with (app_dir / "Info.plist").open("rb") as handle:
        info = plistlib.load(handle)
    share_extensions = []
    for extension_info in app_dir.glob("PlugIns/*.appex/Info.plist"):
        with extension_info.open("rb") as handle:
            extension = plistlib.load(handle)
        if extension["NSExtension"]["NSExtensionPointIdentifier"] == "com.apple.share-services":
            share_extensions.append(extension["CFBundleIdentifier"])
    extension_id, = share_extensions
    updates["IRIS_XCUITEST_SHARE_EXTENSION_BUNDLE_ID"] = extension_id
    updates["IRIS_XCUITEST_APP_DISPLAY_NAME"] = info["CFBundleDisplayName"]
    if os.environ.get("IRIS_DRIVE_MOBILE_LINK_TRANSPORT") == "usb":
        target.setdefault("EnvironmentVariables", {}).update({
            key: os.environ[key] for key in ("IRIS_DRIVE_USB_FIXTURE_PORTS", "IRIS_DRIVE_USB_FIXTURE_TOKEN")
        })
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
}


read_ios_link_profile() {
  local field="$1" file="$TMP/ios-link-state.json"
  xcrun devicectl device copy from --quiet --timeout 8 --device "$IOS_DEVICE_SELECTED" \
    --domain-type appDataContainer --domain-identifier "$IOS_BUNDLE_ID" \
    --source Documents/debug-state.json --destination "$file" >/dev/null 2>&1 || return 1
  python3 - "$file" "$field" <<'PYTHON'
import json
import sys

from urllib.parse import urlsplit

with open(sys.argv[1]) as stream:
    profile = json.load(stream)["ui"].get("profile") or {}
if sys.argv[2] == "request":
    request = profile.get("app_key_link_request")
    if not isinstance(request, str):
        raise SystemExit(1)
    link = urlsplit(request)
    if link.scheme != "https" or link.netloc != "drive.iris.to" or not link.path.startswith("/approve-device/"):
        raise SystemExit(1)
    print(request)
elif sys.argv[2] == "acknowledged":
    raise SystemExit(0 if profile.get("authorization_state") == "authorized"
                     and profile.get("pending_device_approval_receipt_count") == 0 else 1)
else:
    raise SystemExit(1)
PYTHON
}


prepare_physical_share_source() {
  local share_app
  share_app="$(python3 - "$XCTESTRUN" "$IOS_SHARE_SOURCE_BUNDLE_ID" <<'PYTHON'
from pathlib import Path
import plistlib
import sys

root = Path(sys.argv[1]).resolve().parent
apps = []
for info in root.glob("*-iphoneos/*.app/Info.plist"):
    with info.open("rb") as stream:
        if plistlib.load(stream).get("CFBundleIdentifier") == sys.argv[2]:
            apps.append(info.parent)
app, = apps
print(app)
PYTHON
)" || fail "The physical share source did not match the selected test app"
  codesign --verify --deep --strict "$share_app" || fail "The physical share source signature is invalid"
  xcrun devicectl device install app --device "$IOS_DEVICE_SELECTED" --quiet "$share_app" \
    || fail "Could not install the test share source on the selected device"
}
