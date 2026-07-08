#!/usr/bin/env bash
set -Eeuo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
PACKAGE_NAME="${IRIS_DRIVE_ANDROID_PACKAGE:-to.iris.drive.uitest}"
MAIN_ACTIVITY="${IRIS_DRIVE_ANDROID_ACTIVITY:-to.iris.drive.uitest/to.iris.drive.app.MainActivity}"
DEBUG_ACTION_EXTRA="${IRIS_DRIVE_ANDROID_DEBUG_ACTION_EXTRA:-to.iris.drive.DEBUG_ACTION}"
DEBUG_OWNER_EXTRA="${IRIS_DRIVE_ANDROID_DEBUG_OWNER_EXTRA:-to.iris.drive.DEBUG_OWNER}"
DEBUG_NETWORK_HOST_EXTRA="${IRIS_DRIVE_ANDROID_DEBUG_NETWORK_HOST_EXTRA:-to.iris.drive.DEBUG_NETWORK_HOST}"
DEBUG_NETWORK_PORT_EXTRA="${IRIS_DRIVE_ANDROID_DEBUG_NETWORK_PORT_EXTRA:-to.iris.drive.DEBUG_NETWORK_PORT}"
APK_PATH="${IRIS_DRIVE_ANDROID_APK:-$ROOT/android/app/build/outputs/apk/uiTest/app-uiTest.apk}"
TARGET_DIR="${CARGO_TARGET_DIR:-$(cargo metadata --format-version 1 --no-deps | python3 -c 'import json,sys; print(json.load(sys.stdin)["target_directory"])')}"
IDRIVE="${IRIS_DRIVE_IDRIVE_BIN:-$TARGET_DIR/debug/idrive}"
OWNER_CONFIG="$(mktemp -d -t iris-drive-android-gui-owner)"
OWNER_SOURCE_DIR="$(mktemp -d -t iris-drive-android-gui-owner-files)"
OWNER_DAEMON_LOG="$(mktemp -t iris-drive-android-gui-owner-daemon.XXXXXX)"
OWNER_DAEMON_PID=""
OWNER_FIPS_PORT=""
OWNER_HOST_ADDR="${IRIS_DRIVE_ANDROID_HOST_ADDR:-}"
USE_DIRECT_STATIC_PEER="${IRIS_DRIVE_ANDROID_USE_DIRECT_STATIC_PEER:-true}"
LINK_TIMEOUT_SECS="${IRIS_DRIVE_ANDROID_LINK_TIMEOUT_SECS:-90}"
PUBLISH_TIMEOUT_SECS="${IRIS_DRIVE_ANDROID_PUBLISH_TIMEOUT_SECS:-3}"
NETWORK_PROBE_HOST="${IRIS_DRIVE_ANDROID_NETWORK_PROBE_HOST:-1.1.1.1}"
NETWORK_PROBE_PORT="${IRIS_DRIVE_ANDROID_NETWORK_PROBE_PORT:-443}"
serial="${IRIS_DRIVE_ANDROID_SERIAL:-${ANDROID_SERIAL:-}}"

cleanup() {
  if [[ -n "$OWNER_DAEMON_PID" ]]; then
    kill "$OWNER_DAEMON_PID" >/dev/null 2>&1 || true
    wait "$OWNER_DAEMON_PID" 2>/dev/null || true
  fi
  if [[ "${IRIS_DRIVE_ANDROID_KEEP_TEST_APP:-false}" != "true" && -n "${ADB:-}" && -n "$serial" ]]; then
    "$ADB" -s "$serial" uninstall "$PACKAGE_NAME" >/dev/null 2>&1 || true
    "$ADB" -s "$serial" uninstall "$PACKAGE_NAME.test" >/dev/null 2>&1 || true
  fi
  rm -rf "$OWNER_CONFIG"
  rm -rf "$OWNER_SOURCE_DIR"
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
      | python3 -c 'import json,sys; s=json.load(sys.stdin); expected=sys.argv[1]; reqs=((s.get("profile") or {}).get("inbound_app_key_link_requests") or []); raise SystemExit(0 if any(r.get("app_key_npub") == expected and r.get("url") for r in reqs) else 1)' "$expected_device" >/dev/null 2>&1; then
      return 0
    fi
    sleep 0.2
  done
  return 1
}

owner_inbound_request_url() {
  local expected_device="$1"
  "$IDRIVE" --config-dir "$OWNER_CONFIG" status \
    | python3 -c 'import json,sys; s=json.load(sys.stdin); expected=sys.argv[1]; reqs=((s.get("profile") or {}).get("inbound_app_key_link_requests") or []); print(next(r["url"] for r in reqs if r.get("app_key_npub") == expected and r.get("url"))) ' "$expected_device"
}

wait_for_android_authorized() {
  local expected_device="$1"
  local seconds="$2"
  for _ in $(seq 1 "$((seconds * 5))"); do
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
  local mode="${IRIS_DRIVE_ANDROID_GUI_TEST_MODE:-class}"
  local tests=(
    devicesViewUsesOnlineStatusDots
    documentsProviderListsNativeProviderRoot
    authenticatedAppShowsBottomTabsAndSeparateDevicesView
    settingsViewUsesNativeRelayStatusRows
    createProfileFlowDoesNotRequireUsernameOrProfilePhoto
    createProfileFlowWithUsernameCanSkipProfilePhoto
    linkThisDeviceFlowClicksThroughSignInUi
    linkDeviceFlowDoesNotRenderInviteInput
    addDeviceSectionRequiresCompleteNativeLinkInput
    addDeviceSectionDispatchesManualDeviceApproval
    acceptedLinkedDeviceShowsSyncedFileCountInGui
    inboundDeviceRequestApprovalKeepsInlineAddDevicePanelOpen
    deleteDeviceRequiresConfirmation
    acceptedLinkedDeviceThatIsNotOnlineShowsOfflineInGui
    syncPanelShowsOnlyTheAvailableAction
    acceptedLinkedDevicePersistsLoginAndFileCountAfterRestartInGui
  )

  if [[ "$mode" == "class" ]]; then
    (
      cd "$ROOT"
      ANDROID_SERIAL="$serial" ./tools/run-android :app:connectedUiTestAndroidTest \
        "-Pandroid.testInstrumentationRunnerArguments.class=$class"
    )
    return
  fi

  local test
  for test in "${tests[@]}"; do
    (
      cd "$ROOT"
      ANDROID_SERIAL="$serial" ./tools/run-android :app:connectedUiTestAndroidTest \
        "-Pandroid.testInstrumentationRunnerArguments.class=$class#$test"
    )
  done
}

run_android_share_api_test() {
  local class="to.iris.drive.app.ShareActivityInstrumentedTest"
  (
    cd "$ROOT"
    ANDROID_SERIAL="$serial" ./tools/run-android :app:connectedUiTestAndroidTest \
      "-Pandroid.testInstrumentationRunnerArguments.class=$class"
  )
}

ADB="$(resolve_adb)"
serial="$(select_serial "$ADB")"
if [[ -z "$serial" ]]; then
  echo "FAIL: no online Android device or emulator found" >&2
  exit 1
fi

"$ADB" -s "$serial" wait-for-device
run_android_gui_tests
run_android_share_api_test

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
owner_invite="$(python3 -c 'import json,sys; print(json.load(sys.stdin)["app_key_link_invite"]["url"])' <<<"$owner_json")"
owner_app_key_npub="$(python3 -c 'import json,sys; print(json.load(sys.stdin)["current_app_key_npub"])' <<<"$owner_json")"
printf 'hello from android gui sync smoke\n' >"$OWNER_SOURCE_DIR/android-smoke.txt"
"$IDRIVE" --config-dir "$OWNER_CONFIG" import "$OWNER_SOURCE_DIR" >/dev/null
owner_fips_addr="default-graph"
owner_daemon_env=()
android_fips_args=()
if bool_true "$USE_DIRECT_STATIC_PEER"; then
  OWNER_FIPS_PORT="$(unused_loopback_port)"
  owner_host_addr="$(android_host_addr)"
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
    --es IRIS_DRIVE_FIPS_UDP_BIND_ADDR "0.0.0.0:0"
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
env "${owner_daemon_env[@]}" \
  "$IDRIVE" --config-dir "$OWNER_CONFIG" daemon --watch-interval 0 --no-gateway \
  >"$OWNER_DAEMON_LOG" 2>&1 &
OWNER_DAEMON_PID="$!"
if ! wait_for_owner_fips 20; then
  echo "FAIL: owner daemon did not start FIPS for Android GUI link delivery." >&2
  cat "$OWNER_DAEMON_LOG" >&2 || true
  exit 1
fi

"$ADB" -s "$serial" shell pm clear "$PACKAGE_NAME" >/dev/null
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
if ! wait_for_owner_inbound_request "$linked_device" "$LINK_TIMEOUT_SECS"; then
  echo "FAIL: owner did not receive the Android GUI app-key-link request over FIPS." >&2
  dump_android_debug_files
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

publish_json="$("$IDRIVE" --config-dir "$OWNER_CONFIG" publish --timeout "$PUBLISH_TIMEOUT_SECS")"
if ! python3 -c 'import json,sys; s=json.load(sys.stdin); raise SystemExit(0 if s.get("published_drive_root") and not s.get("drive_root_publish_error") else 1)' <<<"$publish_json"; then
  echo "WARN: CLI owner did not confirm relay drive-root publish before Android sync; continuing with direct FIPS sync." >&2
  echo "$publish_json" >&2
fi

if ! wait_for_android_authorized "$linked_device" "$LINK_TIMEOUT_SECS"; then
  echo "FAIL: Android did not leave Waiting for approval after the owner approved its request." >&2
  dump_android_debug_files
  "$IDRIVE" --config-dir "$OWNER_CONFIG" status >&2 || true
  cat "$OWNER_DAEMON_LOG" >&2 || true
  exit 1
fi

adb_am_start -n "$MAIN_ACTIVITY" \
  --es "$DEBUG_ACTION_EXTRA" start-sync \
  "${android_fips_args[@]}" >/dev/null

if ! wait_for_android_provider_entry "android-smoke.txt" "$LINK_TIMEOUT_SECS"; then
  echo "FAIL: Android provider did not expose the owner file after approval and sync." >&2
  dump_android_debug_files
  "$IDRIVE" --config-dir "$OWNER_CONFIG" status >&2 || true
  echo "--- Owner publish JSON ---" >&2
  echo "$publish_json" >&2
  cat "$OWNER_DAEMON_LOG" >&2 || true
  exit 1
fi

echo "ANDROID_GUI_LINKING_AND_SYNC_SMOKE_OK"
echo "serial=$serial"
echo "owner_config=$OWNER_CONFIG"
echo "owner_fips_addr=$owner_fips_addr"
