#!/usr/bin/env bash
set -Eeuo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
PACKAGE_NAME="${IRIS_DRIVE_ANDROID_PACKAGE:-to.iris.drive.uitest}"
MAIN_ACTIVITY="${IRIS_DRIVE_ANDROID_ACTIVITY:-to.iris.drive.uitest/to.iris.drive.app.MainActivity}"
DEBUG_ACTION_EXTRA="${IRIS_DRIVE_ANDROID_DEBUG_ACTION_EXTRA:-to.iris.drive.DEBUG_ACTION}"
DEBUG_OWNER_EXTRA="${IRIS_DRIVE_ANDROID_DEBUG_OWNER_EXTRA:-to.iris.drive.DEBUG_OWNER}"
DEBUG_RELAY_EXTRA="${IRIS_DRIVE_ANDROID_DEBUG_RELAY_EXTRA:-to.iris.drive.DEBUG_RELAY}"
DEBUG_NETWORK_HOST_EXTRA="${IRIS_DRIVE_ANDROID_DEBUG_NETWORK_HOST_EXTRA:-to.iris.drive.DEBUG_NETWORK_HOST}"
DEBUG_NETWORK_PORT_EXTRA="${IRIS_DRIVE_ANDROID_DEBUG_NETWORK_PORT_EXTRA:-to.iris.drive.DEBUG_NETWORK_PORT}"
APK_PATH="${IRIS_DRIVE_ANDROID_APK:-$ROOT/android/app/build/outputs/apk/uiTest/app-uiTest.apk}"
TARGET_DIR="${CARGO_TARGET_DIR:-$(cargo metadata --format-version 1 --no-deps | python3 -c 'import json,sys; print(json.load(sys.stdin)["target_directory"])')}"
IDRIVE="${IRIS_DRIVE_IDRIVE_BIN:-$TARGET_DIR/debug/idrive}"
OWNER_CONFIG="$(mktemp -d "${TMPDIR:-/tmp}/iris-drive-android-gui-owner.XXXXXX")"
export HTREE_DATA_DIR="$OWNER_CONFIG/hashtree-data"
OWNER_SOURCE_DIR="$(mktemp -d "${TMPDIR:-/tmp}/iris-drive-android-gui-owner-files.XXXXXX")"
OWNER_DAEMON_LOG="$(mktemp "${TMPDIR:-/tmp}/iris-drive-android-gui-owner-daemon.XXXXXX")"
OWNER_DAEMON_PID=""
OWNER_FIPS_PORT=""
OWNER_HOST_ADDR="${IRIS_DRIVE_ANDROID_HOST_ADDR:-}"
LOCAL_RELAY_READY="$(mktemp "${TMPDIR:-/tmp}/iris-drive-android-gui-relay.XXXXXX")"
LOCAL_RELAY_LOG="$(mktemp "${TMPDIR:-/tmp}/iris-drive-android-gui-relay.log.XXXXXX")"
LOCAL_RELAY_PID=""
LOCAL_RELAY_URL=""
LOCAL_BLOSSOM_READY="$(mktemp "${TMPDIR:-/tmp}/iris-drive-android-gui-blossom.XXXXXX")"
LOCAL_BLOSSOM_LOG="$(mktemp "${TMPDIR:-/tmp}/iris-drive-android-gui-blossom.log.XXXXXX")"
LOCAL_BLOSSOM_STORAGE="$(mktemp -d "${TMPDIR:-/tmp}/iris-drive-android-gui-blossom-storage.XXXXXX")"
LOCAL_BLOSSOM_PID=""
LOCAL_BLOSSOM_PORT=""
LOCAL_BLOSSOM_DEVICE_URL=""
USE_DIRECT_STATIC_PEER="${IRIS_DRIVE_ANDROID_USE_DIRECT_STATIC_PEER:-true}"
OWNER_FIPS_OPEN_DISCOVERY_MAX_PENDING="${IRIS_DRIVE_ANDROID_FIPS_OPEN_DISCOVERY_MAX_PENDING:-8}"
ANDROID_FIPS_PORT="${IRIS_DRIVE_ANDROID_FIPS_PORT:-59011}"
LINK_REQUEST_TIMEOUT_SECS="${IRIS_DRIVE_ANDROID_LINK_REQUEST_TIMEOUT_SECS:-${IRIS_DRIVE_ANDROID_LINK_TIMEOUT_SECS:-90}}"
AUTHORIZATION_TIMEOUT_SECS="${IRIS_DRIVE_ANDROID_AUTHORIZATION_TIMEOUT_SECS:-15}"
PROVIDER_SYNC_TIMEOUT_SECS="${IRIS_DRIVE_ANDROID_PROVIDER_SYNC_TIMEOUT_SECS:-${IRIS_DRIVE_ANDROID_LINK_TIMEOUT_SECS:-90}}"
PUBLISH_TIMEOUT_SECS="${IRIS_DRIVE_ANDROID_PUBLISH_TIMEOUT_SECS:-3}"
NETWORK_PROBE_HOST="${IRIS_DRIVE_ANDROID_NETWORK_PROBE_HOST:-1.1.1.1}"
NETWORK_PROBE_PORT="${IRIS_DRIVE_ANDROID_NETWORK_PROBE_PORT:-443}"
serial="${IRIS_DRIVE_ANDROID_SERIAL:-${ANDROID_SERIAL:-}}"

cleanup() {
  local status=$?
  trap - EXIT
  set +e
  if [[ -n "$LOCAL_BLOSSOM_PORT" && -n "${ADB:-}" && -n "$serial" ]]; then
    "$ADB" -s "$serial" reverse --remove "tcp:$LOCAL_BLOSSOM_PORT" >/dev/null 2>&1 || true
  fi
  if [[ -n "$LOCAL_BLOSSOM_PID" ]]; then
    kill "$LOCAL_BLOSSOM_PID" >/dev/null 2>&1 || true
    wait "$LOCAL_BLOSSOM_PID" 2>/dev/null || true
  fi
  if [[ -n "$OWNER_DAEMON_PID" ]]; then
    kill "$OWNER_DAEMON_PID" >/dev/null 2>&1 || true
    wait "$OWNER_DAEMON_PID" 2>/dev/null || true
  fi
  if [[ -n "$LOCAL_RELAY_PID" ]]; then
    kill "$LOCAL_RELAY_PID" >/dev/null 2>&1 || true
    wait "$LOCAL_RELAY_PID" 2>/dev/null || true
  fi
  if [[ "${IRIS_DRIVE_ANDROID_KEEP_TEST_APP:-false}" != "true" && -n "${ADB:-}" && -n "$serial" ]]; then
    "$ADB" -s "$serial" uninstall "$PACKAGE_NAME" >/dev/null 2>&1 || true
    "$ADB" -s "$serial" uninstall "$PACKAGE_NAME.test" >/dev/null 2>&1 || true
  fi
  rm -rf "$OWNER_CONFIG"
  rm -rf "$OWNER_SOURCE_DIR"
  rm -rf "$LOCAL_BLOSSOM_STORAGE"
  rm -f \
    "$OWNER_DAEMON_LOG" \
    "$LOCAL_RELAY_READY" \
    "$LOCAL_RELAY_LOG" \
    "$LOCAL_BLOSSOM_READY" \
    "$LOCAL_BLOSSOM_LOG"
  exit "$status"
}
trap cleanup EXIT

for timeout_name in \
  LINK_REQUEST_TIMEOUT_SECS \
  AUTHORIZATION_TIMEOUT_SECS \
  PROVIDER_SYNC_TIMEOUT_SECS \
  PUBLISH_TIMEOUT_SECS
do
  [[ "${!timeout_name}" =~ ^[1-9][0-9]*$ ]] \
    || { echo "FAIL: $timeout_name must be a positive integer" >&2; exit 2; }
done
if ((AUTHORIZATION_TIMEOUT_SECS > 15)); then
  echo "FAIL: IRIS_DRIVE_ANDROID_AUTHORIZATION_TIMEOUT_SECS must be at most 15" >&2
  exit 2
fi

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
  shift 2
  for _ in $(seq 1 "$((seconds * 5))"); do
    if "$ADB" -s "$serial" exec-out run-as "$PACKAGE_NAME" cat files/debug-state.json 2>/dev/null \
      | python3 -c "$jq_expr" "$@" >/dev/null 2>&1; then
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

monotonic_milliseconds() {
  perl -MTime::HiRes=clock_gettime,CLOCK_MONOTONIC \
    -e 'printf "%.0f\n", clock_gettime(CLOCK_MONOTONIC) * 1000'
}

bool_true() {
  case "$1" in
    1 | true | TRUE | True | yes | YES | Yes | on | ON | On) return 0 ;;
    *) return 1 ;;
  esac
}

shell_quote() {
  printf "'"
  printf '%s' "$1" | sed "s/'/'\\\\''/g"
  printf "'"
}

adb_am_start() {
  local command="am start"
  local arg
  for arg in "$@"; do
    command+=" $(shell_quote "$arg")"
  done
  "$ADB" -s "$serial" shell "$command"
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

android_device_addr() {
  local addr
  addr="$("$ADB" -s "$serial" shell 'ip route get 8.8.8.8 2>/dev/null | sed -n "s/.* src \([0-9.]*\).*/\1/p" | head -n 1' | tr -d '\r' || true)"
  if [[ -z "$addr" ]]; then
    addr="$("$ADB" -s "$serial" shell 'ip -4 addr show wlan0 2>/dev/null | sed -n "s/.*inet \([0-9.]*\)\/.*/\1/p" | head -n 1' | tr -d '\r' || true)"
  fi
  if [[ -z "$addr" ]]; then
    echo "FAIL: could not determine an Android device IP reachable from the owner host." >&2
    exit 1
  fi
  printf '%s\n' "$addr"
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

start_local_relay() {
  if [[ -n "$LOCAL_RELAY_URL" ]]; then
    return 0
  fi
  python3 "$ROOT/scripts/local-nostr-relay.py" --ready-file "$LOCAL_RELAY_READY" \
    >"$LOCAL_RELAY_LOG" 2>&1 &
  LOCAL_RELAY_PID="$!"
  for _ in $(seq 1 100); do
    if [[ -s "$LOCAL_RELAY_READY" ]]; then
      LOCAL_RELAY_URL="$(cat "$LOCAL_RELAY_READY")"
      return 0
    fi
    if ! kill -0 "$LOCAL_RELAY_PID" >/dev/null 2>&1; then
      echo "FAIL: local Nostr relay exited before becoming ready" >&2
      cat "$LOCAL_RELAY_LOG" >&2 || true
      exit 1
    fi
    sleep 0.1
  done
  echo "FAIL: local Nostr relay did not become ready" >&2
  cat "$LOCAL_RELAY_LOG" >&2 || true
  exit 1
}

start_local_blossom() {
  local host_url
  if [[ -n "$LOCAL_BLOSSOM_DEVICE_URL" ]]; then
    return 0
  fi

  python3 "$ROOT/scripts/local-blossom-server.py" \
    --host 127.0.0.1 \
    --port 0 \
    --storage-dir "$LOCAL_BLOSSOM_STORAGE" \
    --ready-file "$LOCAL_BLOSSOM_READY" \
    >"$LOCAL_BLOSSOM_LOG" 2>&1 &
  LOCAL_BLOSSOM_PID="$!"
  for _ in $(seq 1 100); do
    if [[ -s "$LOCAL_BLOSSOM_READY" ]]; then
      host_url="$(cat "$LOCAL_BLOSSOM_READY")"
      LOCAL_BLOSSOM_PORT="$(python3 -c 'import sys,urllib.parse; parsed=urllib.parse.urlsplit(sys.argv[1]); print(parsed.port or "")' "$host_url")"
      if [[ ! "$LOCAL_BLOSSOM_PORT" =~ ^[1-9][0-9]*$ ]]; then
        echo "FAIL: local Blossom fixture returned an invalid URL" >&2
        cat "$LOCAL_BLOSSOM_LOG" >&2 || true
        exit 1
      fi
      if ! "$ADB" -s "$serial" reverse \
        "tcp:$LOCAL_BLOSSOM_PORT" "tcp:$LOCAL_BLOSSOM_PORT" >/dev/null; then
        echo "FAIL: could not expose the local Blossom fixture to Android via adb reverse" >&2
        exit 1
      fi
      LOCAL_BLOSSOM_DEVICE_URL="http://127.0.0.1:$LOCAL_BLOSSOM_PORT"
      return 0
    fi
    if ! kill -0 "$LOCAL_BLOSSOM_PID" >/dev/null 2>&1; then
      echo "FAIL: local Blossom fixture exited before becoming ready" >&2
      cat "$LOCAL_BLOSSOM_LOG" >&2 || true
      exit 1
    fi
    sleep 0.1
  done
  echo "FAIL: local Blossom fixture did not become ready" >&2
  cat "$LOCAL_BLOSSOM_LOG" >&2 || true
  exit 1
}

verify_local_blossom_uploads() {
  if (( $(local_blossom_blob_count) == 0 )); then
    echo "FAIL: Android approval tests did not upload any block to the local Blossom fixture" >&2
    cat "$LOCAL_BLOSSOM_LOG" >&2 || true
    return 1
  fi
}

local_blossom_blob_count() {
  find "$LOCAL_BLOSSOM_STORAGE" -type f -name '*.bin' -print \
    | awk 'END { print NR + 0 }'
}

configure_owner_local_blossom() {
  local configured
  "$IDRIVE" --config-dir "$OWNER_CONFIG" \
    blossom-servers remove https://upload.iris.to >/dev/null
  "$IDRIVE" --config-dir "$OWNER_CONFIG" \
    blossom-servers add "$LOCAL_BLOSSOM_DEVICE_URL" >/dev/null
  configured="$("$IDRIVE" --config-dir "$OWNER_CONFIG" blossom-servers list)"
  if ! python3 -c 'import json,sys; servers=json.load(sys.stdin); expected=sys.argv[1]; raise SystemExit(0 if servers == [expected] else 1)' \
    "$LOCAL_BLOSSOM_DEVICE_URL" <<<"$configured"; then
    echo "FAIL: CLI owner must use only the deterministic local Blossom fixture" >&2
    echo "$configured" >&2
    return 1
  fi
}

assert_local_blossom_approval_handoff() {
  local approval_json="$1"
  if ! python3 -c '
import json, sys
approval = json.load(sys.stdin)
upload = approval.get("blossom_upload") or {}
total = int(upload.get("total_hashes") or 0)
uploaded = int(upload.get("uploaded") or 0)
already_present = int(upload.get("already_present") or 0)
valid = (
    approval.get("approval_publish_error") is None
    and approval.get("published_drive_root") is True
    and int(approval.get("published_approval_events") or 0) > 0
    and bool(approval.get("root_cid"))
    and total > 0
    and uploaded >= 0
    and already_present >= 0
    and uploaded + already_present == total
)
raise SystemExit(0 if valid else 1)
' <<<"$approval_json"; then
    echo "FAIL: CLI owner approval did not publish a complete local Blossom root handoff" >&2
    return 1
  fi
}

configure_owner_local_relay() {
  start_local_relay
  "$IDRIVE" --config-dir "$OWNER_CONFIG" relays add "$LOCAL_RELAY_URL" >/dev/null
}

wait_for_owner_inbound_request() {
  local expected_device="$1"
  local seconds="$2"
  for _ in $(seq 1 "$((seconds * 5))"); do
    if "$IDRIVE" --config-dir "$OWNER_CONFIG" status 2>/dev/null \
      | python3 -c 'import json,sys; s=json.load(sys.stdin); expected=sys.argv[1]; prefix="https://drive.iris.to/approve-device/"; reqs=((s.get("profile") or {}).get("inbound_app_key_link_requests") or []); raise SystemExit(0 if any(r.get("app_key_npub") == expected and str(r.get("url") or "").startswith(prefix) for r in reqs) else 1)' "$expected_device" >/dev/null 2>&1; then
      return 0
    fi
    sleep 0.2
  done
  return 1
}

owner_inbound_request_url() {
  local expected_device="$1"
  "$IDRIVE" --config-dir "$OWNER_CONFIG" status \
    | python3 -c 'import json,sys; s=json.load(sys.stdin); expected=sys.argv[1]; prefix="https://drive.iris.to/approve-device/"; reqs=((s.get("profile") or {}).get("inbound_app_key_link_requests") or []); print(next(r["url"] for r in reqs if r.get("app_key_npub") == expected and str(r.get("url") or "").startswith(prefix)))' "$expected_device"
}

wait_for_android_authorized_until() {
  local expected_device="$1"
  local deadline_ms="$2"
  while (( $(monotonic_milliseconds) <= deadline_ms )); do
    adb_am_start -n "$MAIN_ACTIVITY" \
      --es "$DEBUG_ACTION_EXTRA" refresh >/dev/null
    if "$ADB" -s "$serial" exec-out run-as "$PACKAGE_NAME" cat files/debug-state.json 2>/dev/null \
      | python3 -c 'import json,sys; s=json.load(sys.stdin); expected=sys.argv[1]; ui=s.get("ui",{}); a=ui.get("profile") or {}; actors=ui.get("app_actors") or ui.get("devices") or []; ok=a.get("authorization_state") == "authorized" and any(row.get("pubkey") == expected and (row.get("is_current_app_key") or row.get("is_current_device")) for row in actors); raise SystemExit(0 if ok else 1)' "$expected_device" >/dev/null 2>&1; then
      return 0
    fi
    sleep 0.2
  done
  return 1
}

wait_for_android_provider_entry() {
  local expected_path="$1"
  local seconds="$2"
  for _ in $(seq 1 "$((seconds * 2))"); do
    adb_am_start -n "$MAIN_ACTIVITY" \
      --es "$DEBUG_ACTION_EXTRA" dump-provider-list >/dev/null
    if "$ADB" -s "$serial" exec-out run-as "$PACKAGE_NAME" cat files/debug-provider-list.json 2>/dev/null \
      | python3 -c 'import json,sys; s=json.load(sys.stdin); expected=sys.argv[1]; entries=s.get("entries") or []; raise SystemExit(0 if any(e.get("path") == expected for e in entries) else 1)' "$expected_path" >/dev/null 2>&1; then
      return 0
    fi
    sleep 0.5
  done
  return 1
}

dump_android_debug_files() {
  echo "--- Android debug-state.json ---" >&2
  "$ADB" -s "$serial" exec-out run-as "$PACKAGE_NAME" cat files/debug-state.json >&2 || true
  echo "--- Android native-fips-status.json ---" >&2
  "$ADB" -s "$serial" exec-out run-as "$PACKAGE_NAME" cat files/native-fips-status.json >&2 || true
  echo "--- Android debug-env.json ---" >&2
  "$ADB" -s "$serial" exec-out run-as "$PACKAGE_NAME" cat files/debug-env.json >&2 || true
  echo "--- Android debug-network-probe.json ---" >&2
  "$ADB" -s "$serial" exec-out run-as "$PACKAGE_NAME" cat files/debug-network-probe.json >&2 || true
  echo "--- Android debug-provider-list.json ---" >&2
  "$ADB" -s "$serial" exec-out run-as "$PACKAGE_NAME" cat files/debug-provider-list.json >&2 || true
}

run_android_network_probe() {
  local host="$1"
  local port="$2"
  local seconds="$3"
  local json classification
  "$ADB" -s "$serial" shell run-as "$PACKAGE_NAME" rm -f files/debug-network-probe.json >/dev/null 2>&1 || true
  adb_am_start -n "$MAIN_ACTIVITY" \
    --es "$DEBUG_ACTION_EXTRA" probe-network \
    --es "$DEBUG_NETWORK_HOST_EXTRA" "$host" \
    --es "$DEBUG_NETWORK_PORT_EXTRA" "$port" >/dev/null

  for _ in $(seq 1 "$((seconds * 5))"); do
    json="$("$ADB" -s "$serial" exec-out run-as "$PACKAGE_NAME" cat files/debug-network-probe.json 2>/dev/null || true)"
    classification="$(python3 -c '
import json, sys
try:
    status = json.loads(sys.stdin.read())
except Exception:
    raise SystemExit(2)
if status.get("ok"):
    print("ok")
    raise SystemExit(0)
error = str(status.get("error") or "")
if "EACCES" in error or "Permission denied" in error:
    print("permission-denied")
    raise SystemExit(0)
print("other")
' <<<"$json" 2>/dev/null || true)"
    case "$classification" in
      ok) return 0 ;;
      permission-denied) return 2 ;;
      other) return 1 ;;
    esac
    sleep 0.2
  done
  return 1
}

require_android_app_network_permission() {
  local probe_status=0
  local vpn_app vpn_lockdown vpn_whitelist
  run_android_network_probe "$NETWORK_PROBE_HOST" "$NETWORK_PROBE_PORT" 8 || probe_status="$?"
  case "$probe_status" in
    0) return 0 ;;
    2)
      echo "FAIL: Android app process cannot open network sockets; debug probe to $NETWORK_PROBE_HOST:$NETWORK_PROBE_PORT returned EACCES/Permission denied." >&2
      echo "Grant the OS-level Network/Internet permission for $PACKAGE_NAME on this device, then rerun the smoke." >&2
      vpn_app="$("$ADB" -s "$serial" shell settings get secure always_on_vpn_app 2>/dev/null | tr -d '\r' || true)"
      vpn_lockdown="$("$ADB" -s "$serial" shell settings get secure always_on_vpn_lockdown 2>/dev/null | tr -d '\r' || true)"
      vpn_whitelist="$("$ADB" -s "$serial" shell settings get secure always_on_vpn_lockdown_whitelist 2>/dev/null | tr -d '\r' || true)"
      if [[ "$vpn_lockdown" == "1" ]]; then
        echo "Android always-on VPN lockdown is enabled for ${vpn_app:-unknown VPN}; disable 'Block connections without VPN' or route/allow $PACKAGE_NAME before rerunning." >&2
        echo "VPN lockdown whitelist: ${vpn_whitelist:-<empty>}" >&2
      fi
      dump_android_debug_files
      return 1
      ;;
    *)
      echo "WARN: Android app network probe to $NETWORK_PROBE_HOST:$NETWORK_PROBE_PORT did not confirm connectivity; continuing to the FIPS smoke." >&2
      "$ADB" -s "$serial" exec-out run-as "$PACKAGE_NAME" cat files/debug-network-probe.json >&2 || true
      return 0
      ;;
  esac
}

run_android_gui_tests() {
  local class="to.iris.drive.app.IrisDriveAndroidGuiFlowTest"
  local approval_deep_link_class="to.iris.drive.app.MainActivityApprovalDeepLinkTest"
  local native_state_class="to.iris.drive.app.IrisDriveAndroidNativeStateTest"
  local share_api_class="to.iris.drive.app.ShareActivityInstrumentedTest"
  local mode="${IRIS_DRIVE_ANDROID_GUI_TEST_MODE:-smoke}"
  local blossom_argument="-Pandroid.testInstrumentationRunnerArguments.blossom_server=$LOCAL_BLOSSOM_DEVICE_URL"
  local smoke_tests=(
    createProfileFlowDoesNotRequireUsernameOrProfilePhoto
    linkThisDeviceFlowClicksThroughSignInUi
    authenticatedAppShowsBottomTabsAndSeparateDevicesView
    devicesViewUsesOnlineStatusDots
  )
  local tests=(
    devicesViewUsesOnlineStatusDots
    documentsProviderListsNativeProviderRoot
    authenticatedAppShowsBottomTabsAndSeparateDevicesView
    settingsViewUsesNativeRelayStatusRows
    createProfileFlowDoesNotRequireUsernameOrProfilePhoto
    createProfileFlowWithUsernameCanSkipProfilePhoto
    linkThisDeviceFlowClicksThroughSignInUi
    signInStartsJoinRequest
    addDeviceSectionRequiresCompleteNativeLinkInput
    addDeviceSectionDispatchesManualDeviceApproval
    inboundDeviceRequestApprovalKeepsInlineAddDevicePanelOpen
    deleteDeviceRequiresConfirmation
    acceptedLinkedDeviceThatIsNotOnlineShowsOfflineInGui
    syncPanelShowsOnlyTheAvailableAction
  )

  local test
  local filter=""
  case "$mode" in
    class)
      filter="$class,$approval_deep_link_class,$native_state_class,$share_api_class"
      ;;
    serial)
      for test in "${tests[@]}"; do
        (
          cd "$ROOT"
          ANDROID_SERIAL="$serial" ./tools/run-android :app:connectedUiTestAndroidTest \
            "$blossom_argument" \
            "-Pandroid.testInstrumentationRunnerArguments.class=$class#$test"
        )
      done
      filter="$approval_deep_link_class,$native_state_class,$share_api_class"
      ;;
    smoke)
      for test in "${smoke_tests[@]}"; do
        filter+="${filter:+,}$class#$test"
      done
      filter+=",$approval_deep_link_class,$native_state_class,$share_api_class"
      ;;
    *)
      echo "FAIL: unknown IRIS_DRIVE_ANDROID_GUI_TEST_MODE=$mode (expected smoke, serial, or class)" >&2
      return 1
      ;;
  esac
  (
    cd "$ROOT"
    ANDROID_SERIAL="$serial" ./tools/run-android :app:assembleDebug :app:connectedUiTestAndroidTest \
      "$blossom_argument" \
      "-Pandroid.testInstrumentationRunnerArguments.class=$filter"
  )
}

ADB="$(resolve_adb)"
serial="$(select_serial "$ADB")"
if [[ -z "$serial" ]]; then
  echo "FAIL: no online Android device or emulator found" >&2
  exit 1
fi

"$ADB" -s "$serial" wait-for-device
start_local_blossom
run_android_gui_tests
verify_local_blossom_uploads

if [[ ! -x "$IDRIVE" ]]; then
  cargo build -p idrive
fi
if [[ ! -f "$APK_PATH" ]]; then
  echo "FAIL: Debug APK not found at $APK_PATH" >&2
  exit 1
fi

"$ADB" -s "$serial" install -r -g "$APK_PATH" >/dev/null
"$ADB" -s "$serial" shell pm clear "$PACKAGE_NAME" >/dev/null
if ! require_android_app_network_permission; then
  exit 1
fi
"$ADB" -s "$serial" shell pm clear "$PACKAGE_NAME" >/dev/null
adb_am_start -S -n "$MAIN_ACTIVITY" \
  --es "$DEBUG_ACTION_EXTRA" create-profile >/dev/null

if ! wait_for_debug_state \
  'import json,sys; s=json.load(sys.stdin); a=s.get("ui",{}).get("profile") or {}; raise SystemExit(0 if a.get("authorization_state") == "authorized" and a.get("can_admin_profile") else 1)' \
  15; then
  echo "FAIL: Android did not create a real owner profile after the GUI create-profile test." >&2
  "$ADB" -s "$serial" exec-out run-as "$PACKAGE_NAME" cat files/debug-state.json >&2 || true
  exit 1
fi

owner_json="$("$IDRIVE" --config-dir "$OWNER_CONFIG" init --force --label "CLI owner")"
configure_owner_local_relay
configure_owner_local_blossom
owner_invite="$(python3 -c 'import json,sys; print(json.load(sys.stdin)["app_key_link_invite"]["url"])' <<<"$owner_json")"
owner_app_key_npub="$(python3 -c 'import json,sys; print(json.load(sys.stdin)["current_app_key_npub"])' <<<"$owner_json")"
printf 'hello from android gui sync smoke\n' >"$OWNER_SOURCE_DIR/android-smoke.txt"
"$IDRIVE" --config-dir "$OWNER_CONFIG" import "$OWNER_SOURCE_DIR" >/dev/null
owner_fips_addr="default-graph"
owner_daemon_env=(
  "IRIS_DRIVE_FIPS_OPEN_DISCOVERY_MAX_PENDING=$OWNER_FIPS_OPEN_DISCOVERY_MAX_PENDING"
)
android_fips_args=()
if bool_true "$USE_DIRECT_STATIC_PEER"; then
  OWNER_FIPS_PORT="$(unused_loopback_port)"
  owner_host_addr="$(android_host_addr)"
  android_addr="$(android_device_addr)"
  owner_fips_peer="$owner_app_key_npub=$owner_host_addr:$OWNER_FIPS_PORT"
  owner_fips_addr="$owner_host_addr:$OWNER_FIPS_PORT"
  owner_daemon_env+=(
    IRIS_DRIVE_FIPS_ENABLE_BOOTSTRAP=false
    IRIS_DRIVE_FIPS_ENABLE_WEBRTC=false
    "IRIS_DRIVE_FIPS_UDP_BIND_ADDR=0.0.0.0:$OWNER_FIPS_PORT"
    "IRIS_DRIVE_FIPS_UDP_EXTERNAL_ADDR=$owner_host_addr:$OWNER_FIPS_PORT"
    IRIS_DRIVE_FIPS_UDP_PUBLIC=false
  )
  android_fips_args+=(
    --es IRIS_DRIVE_FIPS_ENABLE_BOOTSTRAP false
    --es IRIS_DRIVE_FIPS_ENABLE_WEBRTC false
    --es IRIS_DRIVE_FIPS_STATIC_PEERS "$owner_fips_peer"
    --es IRIS_DRIVE_FIPS_UDP_BIND_ADDR "0.0.0.0:$ANDROID_FIPS_PORT"
    --es IRIS_DRIVE_FIPS_UDP_EXTERNAL_ADDR "$android_addr:$ANDROID_FIPS_PORT"
    --es IRIS_DRIVE_FIPS_UDP_PUBLIC false
  )
else
  owner_daemon_env+=(
    IRIS_DRIVE_FIPS_ENABLE_BOOTSTRAP=true
    IRIS_DRIVE_FIPS_ENABLE_WEBRTC=true
  )
  android_fips_args+=(
    --es IRIS_DRIVE_FIPS_ENABLE_BOOTSTRAP true
    --es IRIS_DRIVE_FIPS_ENABLE_WEBRTC true
  )
fi

"$ADB" -s "$serial" shell pm clear "$PACKAGE_NAME" >/dev/null
adb_am_start -S -n "$MAIN_ACTIVITY" \
  --es "$DEBUG_ACTION_EXTRA" add-relay \
  --es "$DEBUG_RELAY_EXTRA" "$LOCAL_RELAY_URL" >/dev/null

if ! wait_for_debug_state \
  'import json,sys; s=json.load(sys.stdin); relay=sys.argv[1]; relays=s.get("ui",{}).get("relays") or []; raise SystemExit(0 if relay in relays else 1)' \
  10 "$LOCAL_RELAY_URL"; then
  echo "FAIL: Android did not persist the local relay needed for deterministic FIPS link discovery." >&2
  dump_android_debug_files
  exit 1
fi
adb_am_start -S -n "$MAIN_ACTIVITY" \
  --es "$DEBUG_ACTION_EXTRA" link-device \
  --es "$DEBUG_OWNER_EXTRA" "$owner_invite" \
  "${android_fips_args[@]}" >/dev/null

if ! wait_for_debug_state \
  'import json,sys; s=json.load(sys.stdin); a=s.get("ui",{}).get("profile") or {}; raise SystemExit(0 if a.get("authorization_state") == "awaiting_approval" and a.get("app_key_link_request") else 1)' \
  15; then
  echo "FAIL: Android did not create a real awaiting linked-device profile after the GUI link-this-device test." >&2
  dump_android_debug_files
  exit 1
fi

linked_device="$("$ADB" -s "$serial" exec-out run-as "$PACKAGE_NAME" cat files/debug-state.json \
  | python3 -c 'import json,sys; print(json.load(sys.stdin)["ui"]["profile"]["current_app_key_npub"])')"
if bool_true "$USE_DIRECT_STATIC_PEER"; then
  owner_daemon_env+=(
    "IRIS_DRIVE_FIPS_STATIC_PEERS=$linked_device=$android_addr:$ANDROID_FIPS_PORT"
  )
fi
env "${owner_daemon_env[@]}" \
  "$IDRIVE" --config-dir "$OWNER_CONFIG" daemon --watch-interval 0 --no-gateway \
  >"$OWNER_DAEMON_LOG" 2>&1 &
OWNER_DAEMON_PID="$!"
if ! wait_for_owner_fips 20; then
  echo "FAIL: owner daemon did not start FIPS for Android GUI link delivery." >&2
  cat "$OWNER_DAEMON_LOG" >&2 || true
  exit 1
fi

if ! wait_for_owner_inbound_request "$linked_device" "$LINK_REQUEST_TIMEOUT_SECS"; then
  echo "FAIL: owner did not receive the Android GUI app-key-link request over FIPS." >&2
  dump_android_debug_files
  "$IDRIVE" --config-dir "$OWNER_CONFIG" status >&2 || true
  cat "$OWNER_DAEMON_LOG" >&2 || true
  exit 1
fi

request_url="$(owner_inbound_request_url "$linked_device")"
authorization_started_ms="$(monotonic_milliseconds)"
authorization_deadline_ms=$((authorization_started_ms + AUTHORIZATION_TIMEOUT_SECS * 1000))
approved_json="$("$IDRIVE" --config-dir "$OWNER_CONFIG" approve "$request_url" --label "Android GUI")"
assert_local_blossom_approval_handoff "$approved_json"
roster_size="$(python3 -c 'import json,sys; print(json.load(sys.stdin)["roster_size"])' <<<"$approved_json")"
if [[ "$roster_size" != "2" ]]; then
  echo "FAIL: CLI owner did not approve the inbound Android GUI request." >&2
  echo "$approved_json" >&2
  exit 1
fi

if ! wait_for_android_authorized_until "$linked_device" "$authorization_deadline_ms"; then
  echo "FAIL: Android did not become authorized within ${AUTHORIZATION_TIMEOUT_SECS}s of starting approval submission." >&2
  dump_android_debug_files
  "$IDRIVE" --config-dir "$OWNER_CONFIG" status >&2 || true
  cat "$OWNER_DAEMON_LOG" >&2 || true
  exit 1
fi
authorization_finished_ms="$(monotonic_milliseconds)"
authorization_delivery_ms=$((authorization_finished_ms - authorization_started_ms))
if ((authorization_finished_ms > authorization_deadline_ms)); then
  echo "FAIL: Android authorization was observed after the strict ${AUTHORIZATION_TIMEOUT_SECS}s deadline (${authorization_delivery_ms}ms)." >&2
  exit 1
fi

publish_json="$("$IDRIVE" --config-dir "$OWNER_CONFIG" publish --timeout "$PUBLISH_TIMEOUT_SECS")"
if ! python3 -c 'import json,sys; s=json.load(sys.stdin); raise SystemExit(0 if s.get("published_drive_root") and not s.get("drive_root_publish_error") else 1)' <<<"$publish_json"; then
  echo "WARN: CLI owner did not confirm relay drive-root publish before Android sync; continuing with direct FIPS sync." >&2
  echo "$publish_json" >&2
fi

adb_am_start -n "$MAIN_ACTIVITY" \
  --es "$DEBUG_ACTION_EXTRA" start-sync \
  "${android_fips_args[@]}" >/dev/null

if ! wait_for_android_provider_entry "android-smoke.txt" "$PROVIDER_SYNC_TIMEOUT_SECS"; then
  echo "FAIL: Android provider did not expose the owner file after approval and sync." >&2
  dump_android_debug_files
  "$IDRIVE" --config-dir "$OWNER_CONFIG" status >&2 || true
  echo "--- Owner publish JSON ---" >&2
  echo "$publish_json" >&2
  cat "$OWNER_DAEMON_LOG" >&2 || true
  exit 1
fi

echo "ANDROID_GUI_LINKING_AND_SYNC_SMOKE_OK"
echo "authorization_delivery_ms=$authorization_delivery_ms clock=host_monotonic_ms start=before_approval_command finish=host_observed_authorization ceiling_ms=$((AUTHORIZATION_TIMEOUT_SECS * 1000))"
echo "serial=$serial"
echo "owner_config=$OWNER_CONFIG"
echo "owner_fips_addr=$owner_fips_addr"
