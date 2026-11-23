#!/usr/bin/env bash

sha256_file() {
  python3 - "$1" <<'PY'
import hashlib
import sys
with open(sys.argv[1], "rb") as handle:
    print(hashlib.sha256(handle.read()).hexdigest())
PY
}

capture_android_request_frame() {
  android_dump_ui
  local geometry height width top left
  geometry="$(python3 - "$TMP/android-ui.xml" "$ANDROID_PACKAGE" <<'PY'
import re
import sys
import xml.etree.ElementTree as ET

nodes = [node for node in ET.parse(sys.argv[1]).iter("node")
         if node.get("package") == sys.argv[2] and node.get("content-desc") == "Device approval request QR"]
if len(nodes) != 1:
    raise SystemExit("Expected one visible request QR in the test app")
match = re.fullmatch(r"\[(\d+),(\d+)\]\[(\d+),(\d+)\]", nodes[0].get("bounds", ""))
if not match:
    raise SystemExit("Missing QR image bounds")
x, y, right, bottom = map(int, match.groups())
if not (0 < right - x <= 4096 and 0 < bottom - y <= 4096):
    raise SystemExit("Invalid QR image bounds")
print(bottom - y, right - x, y, x)
PY
)" || return 1
  read -r height width top left <<< "$geometry"
  "$ADB" -s "$ANDROID_SERIAL_SELECTED" exec-out screencap -p >"$TMP/android-screen.png"
  sips -c "$height" "$width" --cropOffset "$top" "$left" "$TMP/android-screen.png" \
    --out "$TMP/android-request.png" >/dev/null
  rm -f "$TMP/android-screen.png"
}

send_frame_to_ios() {
  xcrun devicectl device copy to --device "$IOS_DEVICE_SELECTED" --domain-type appDataContainer \
    --domain-identifier "$IOS_BUNDLE_ID" --source "$TMP/android-request.png" \
    --destination "Documents/qr-frame-$QR_FRAME_SHA256.png" --quiet
}

copy_ios_request_frame() {
  xcrun devicectl device copy from --device "$IOS_DEVICE_SELECTED" --domain-type appDataContainer \
    --domain-identifier "$IOS_RUNNER_BUNDLE_ID" --source "Documents/qr-request-$RUN_ID.png" \
    --destination "$TMP/ios-request.png" --quiet
  [[ -s "$TMP/ios-request.png" ]]
}

arm_android_frame_reader() {
  printf '%s\n' "$1" | "$ADB" -s "$ANDROID_SERIAL_SELECTED" shell -T run-as "$ANDROID_PACKAGE" \
    tee files/qr-frame.expected >/dev/null
}

send_frame_to_android() {
  "$ADB" -s "$ANDROID_SERIAL_SELECTED" shell -T run-as "$ANDROID_PACKAGE" \
    tee "files/qr-frame-$1.png" <"$TMP/ios-request.png" >/dev/null
}


android_scanner_surface_ready() {
  python3 - "$TMP/android-ui.xml" "$ANDROID_PACKAGE" <<'PYTHON'
import re
import sys
import xml.etree.ElementTree as ET

nodes = [item for item in ET.parse(sys.argv[1]).iter("node") if item.get("package") == sys.argv[2]]
labels = {item.get(key) for item in nodes for key in ("text", "content-desc")}
for item in nodes:
    if item.get("class") not in {"android.view.SurfaceView", "android.view.TextureView"}:
        continue
    bounds = [int(n) for n in re.findall(r"\d+", item.get("bounds", ""))]
    if len(bounds) == 4 and bounds[2] > bounds[0] and bounds[3] > bounds[1] and {"Scan", "Close"} <= labels:
        raise SystemExit(0)
raise SystemExit(1)
PYTHON
}
