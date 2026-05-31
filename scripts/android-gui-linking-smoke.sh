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
OWNER_DAEMON_LOG="$(mktemp -t iris-drive-android-gui-owner-daemon.XXXXXX)"
OWNER_DAEMON_PID=""
OWNER_FIPS_PORT=""
OWNER_HOST_ADDR="${IRIS_DRIVE_ANDROID_HOST_ADDR:-}"
serial="${IRIS_DRIVE_ANDROID_SERIAL:-${ANDROID_SERIAL:-}}"

cleanup() {
  if [[ -n "$OWNER_DAEMON_PID" ]]; then
    kill "$OWNER_DAEMON_PID" >/dev/null 2>&1 || true
    wait "$OWNER_DAEMON_PID" 2>/dev/null || true
  fi
  rm -rf "$OWNER_CONFIG"
  rm -f "$OWNER_DAEMON_LOG"
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

unused_loopback_port() {
  python3 - <<'PY'
import socket

with socket.socket(socket.AF_INET, socket.SOCK_STREAM) as sock:
    sock.bind(("127.0.0.1", 0))
    print(sock.getsockname()[1])
PY
}

android_host_addr() {
  if [[ -n "$OWNER_HOST_ADDR" ]]; then
    printf '%s\n' "$OWNER_HOST_ADDR"
    return
  fi

  if [[ "$("$ADB" -s "$serial" shell getprop ro.kernel.qemu 2>/dev/null | tr -d '\r')" == "1" ]]; then
    printf '10.0.2.2\n'
    return
  fi

  local route_iface
  route_iface="$(route -n get default 2>/dev/null | awk '/interface:/{print $2; exit}' || true)"
  if [[ -n "$route_iface" ]]; then
    OWNER_HOST_ADDR="$(ipconfig getifaddr "$route_iface" 2>/dev/null || true)"
  fi

  if [[ -z "$OWNER_HOST_ADDR" ]]; then
    OWNER_HOST_ADDR="$(python3 - <<'PY'
import socket

try:
    with socket.socket(socket.AF_INET, socket.SOCK_DGRAM) as sock:
        sock.connect(("8.8.8.8", 80))
        print(sock.getsockname()[0])
except OSError:
    pass
PY
)"
  fi

  if [[ -z "$OWNER_HOST_ADDR" ]]; then
    echo "FAIL: could not determine a host IP reachable from Android; set IRIS_DRIVE_ANDROID_HOST_ADDR." >&2
    exit 1
  fi
  printf '%s\n' "$OWNER_HOST_ADDR"
}

wait_for_owner_fips() {
  local seconds="$1"
  for _ in $(seq 1 "$((seconds * 5))"); do
    if "$IDRIVE" --config-dir "$OWNER_CONFIG" status 2>/dev/null \
      | python3 -c 'import json,sys; s=json.load(sys.stdin); f=((s.get("network") or {}).get("fips") or {}); raise SystemExit(0 if f.get("running") and f.get("endpoint_npub") else 1)' >/dev/null 2>&1; then
      return 0
    fi
    sleep 0.2
  done
  return 1
}

wait_for_owner_inbound_request() {
  local expected_device="$1"
  local seconds="$2"
  for _ in $(seq 1 "$((seconds * 5))"); do
    if "$IDRIVE" --config-dir "$OWNER_CONFIG" status 2>/dev/null \
      | python3 -c 'import json,sys; s=json.load(sys.stdin); expected=sys.argv[1]; reqs=((s.get("account") or {}).get("inbound_device_link_requests") or []); raise SystemExit(0 if any(r.get("device_npub") == expected and r.get("url") for r in reqs) else 1)' "$expected_device" >/dev/null 2>&1; then
      return 0
    fi
    sleep 0.2
  done
  return 1
}

owner_inbound_request_url() {
  local expected_device="$1"
  "$IDRIVE" --config-dir "$OWNER_CONFIG" status \
    | python3 -c 'import json,sys; s=json.load(sys.stdin); expected=sys.argv[1]; reqs=((s.get("account") or {}).get("inbound_device_link_requests") or []); print(next(r["url"] for r in reqs if r.get("device_npub") == expected and r.get("url"))) ' "$expected_device"
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
owner_device_npub="$(python3 -c 'import json,sys; print(json.load(sys.stdin)["device_npub"])' <<<"$owner_json")"
OWNER_FIPS_PORT="$(unused_loopback_port)"
owner_host_addr="$(android_host_addr)"
owner_fips_peer="$owner_device_npub=$owner_host_addr:$OWNER_FIPS_PORT"
IRIS_DRIVE_FIPS_UDP_BIND_ADDR="0.0.0.0:$OWNER_FIPS_PORT" \
  IRIS_DRIVE_FIPS_UDP_EXTERNAL_ADDR="$owner_host_addr:$OWNER_FIPS_PORT" \
  IRIS_DRIVE_FIPS_UDP_PUBLIC=false \
  IRIS_DRIVE_FIPS_ENABLE_BOOTSTRAP=false \
  IRIS_DRIVE_FIPS_ENABLE_WEBRTC=false \
  "$IDRIVE" --config-dir "$OWNER_CONFIG" daemon --watch-interval 0 --no-gateway \
  >"$OWNER_DAEMON_LOG" 2>&1 &
OWNER_DAEMON_PID="$!"
if ! wait_for_owner_fips 20; then
  echo "FAIL: owner daemon did not start FIPS for Android GUI link delivery." >&2
  cat "$OWNER_DAEMON_LOG" >&2 || true
  exit 1
fi

"$ADB" -s "$serial" shell pm clear "$PACKAGE_NAME" >/dev/null
"$ADB" -s "$serial" shell am start -S -n "$MAIN_ACTIVITY" \
  --es "$DEBUG_ACTION_EXTRA" link-device \
  --es "$DEBUG_OWNER_EXTRA" "$owner_invite" \
  --es IRIS_DRIVE_FIPS_STATIC_PEERS "$owner_fips_peer" \
  --es IRIS_DRIVE_FIPS_ENABLE_BOOTSTRAP false \
  --es IRIS_DRIVE_FIPS_ENABLE_WEBRTC false \
  --es IRIS_DRIVE_FIPS_UDP_BIND_ADDR "0.0.0.0:0" \
  --es IRIS_DRIVE_FIPS_UDP_EXTERNAL_ADDR "" >/dev/null

if ! wait_for_debug_state \
  'import json,sys; s=json.load(sys.stdin); a=s.get("ui",{}).get("account") or {}; raise SystemExit(0 if a.get("authorization_state") == "awaiting_approval" and a.get("device_link_request") else 1)' \
  15; then
  echo "FAIL: Android did not create a real awaiting linked-device profile after the GUI link-this-device test." >&2
  "$ADB" -s "$serial" exec-out run-as "$PACKAGE_NAME" cat files/debug-state.json >&2 || true
  exit 1
fi

linked_device="$("$ADB" -s "$serial" exec-out run-as "$PACKAGE_NAME" cat files/debug-state.json \
  | python3 -c 'import json,sys; print(json.load(sys.stdin)["ui"]["account"]["device_pubkey"])')"
if ! wait_for_owner_inbound_request "$linked_device" 30; then
  echo "FAIL: owner did not receive the Android GUI device-link request over FIPS." >&2
  "$IDRIVE" --config-dir "$OWNER_CONFIG" status >&2 || true
  cat "$OWNER_DAEMON_LOG" >&2 || true
  exit 1
fi

request_url="$(owner_inbound_request_url "$linked_device")"
approved_json="$("$IDRIVE" --config-dir "$OWNER_CONFIG" approve "$request_url" --label "Android GUI")"
roster_size="$(python3 -c 'import json,sys; print(json.load(sys.stdin)["roster_size"])' <<<"$approved_json")"
if [[ "$roster_size" != "2" ]]; then
  echo "FAIL: CLI owner did not approve the inbound Android GUI request." >&2
  echo "$approved_json" >&2
  exit 1
fi

echo "ANDROID_GUI_LINKING_SMOKE_OK"
echo "serial=$serial"
echo "owner_config=$OWNER_CONFIG"
echo "owner_fips_addr=$owner_host_addr:$OWNER_FIPS_PORT"
