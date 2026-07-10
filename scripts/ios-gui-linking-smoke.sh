#!/usr/bin/env bash

set -Eeuo pipefail

case "$(uname -s)" in
  Darwin) ;;
  *)
    echo "iOS GUI smoke is Darwin-only; skipping on $(uname -s)"
    exit 0
    ;;
esac

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
source "$ROOT/scripts/ios-simulator-signing.sh"
PROJECT="$ROOT/ios/IrisDriveIOS.xcodeproj"
SCHEME="IrisDriveIOS"
CONFIGURATION="${IRIS_DRIVE_IOS_XCODE_CONFIGURATION:-Debug}"
DERIVED_DATA="$ROOT/ios/.build/DerivedData"
BUILD_LOG="${IRIS_DRIVE_IOS_UI_BUILD_LOG:-/tmp/iris-drive-ios-ui-tests.log}"
BUNDLE_ID="${IRIS_DRIVE_IOS_BUNDLE_ID:-fi.siriusbusiness.drive}"
SHARE_SOURCE_BUNDLE_ID="${IRIS_DRIVE_IOS_SHARE_SOURCE_BUNDLE_ID:-fi.siriusbusiness.drive.ShareSource}"
APP_GROUP_ID="${IRIS_DRIVE_IOS_APP_GROUP_IDENTIFIER:-group.fi.siriusbusiness.drive}"
SHARE_SHEET_SMOKE_FILE="Iris Drive Share Sheet Smoke.txt"
SHARE_SHEET_SMOKE_CONTENT="shared from iOS share sheet"
DEVICE_NAME="${IRIS_DRIVE_IOS_SIMULATOR_DEVICE:-}"
TARGET_DIR="${CARGO_TARGET_DIR:-$(cargo metadata --format-version 1 --no-deps | python3 -c 'import json,sys; print(json.load(sys.stdin)["target_directory"])')}"
IDRIVE="${IRIS_DRIVE_IDRIVE_BIN:-$TARGET_DIR/debug/idrive}"
RUST_IOS_TARGET="${IRIS_DRIVE_IOS_RUST_TARGET:-aarch64-apple-ios-sim}"
RUST_LIB_DIR="$TARGET_DIR/$RUST_IOS_TARGET/debug"
RUST_STATIC_LIB="$RUST_LIB_DIR/libiris_drive_app_core.a"
OWNER_CONFIG="$(mktemp -d -t iris-drive-ios-ui-owner)"
LINKED_CONFIG="$(mktemp -d -t iris-drive-ios-ui-linked)"
XCTESTRUN=""
OWNER_DAEMON_PID=""
OWNER_DAEMON_LOG="$(mktemp -t iris-drive-ios-ui-owner-daemon.XXXXXX.log)"
OWNER_FIPS_PORT=""
SIM_APP_BASE_DIR=""
LOCAL_RELAY_READY="$(mktemp -t iris-drive-ios-ui-relay.XXXXXX)"
LOCAL_RELAY_LOG="$(mktemp -t iris-drive-ios-ui-relay.XXXXXX.log)"
LOCAL_RELAY_PID=""
LOCAL_RELAY_URL=""

cleanup() {
  if [[ -n "$OWNER_DAEMON_PID" ]]; then
    kill "$OWNER_DAEMON_PID" >/dev/null 2>&1 || true
    wait "$OWNER_DAEMON_PID" >/dev/null 2>&1 || true
  fi
  if [[ -n "$LOCAL_RELAY_PID" ]]; then
    kill "$LOCAL_RELAY_PID" >/dev/null 2>&1 || true
    wait "$LOCAL_RELAY_PID" >/dev/null 2>&1 || true
  fi
  rm -rf "$OWNER_CONFIG" "$LINKED_CONFIG"
  rm -f "$OWNER_DAEMON_LOG" "$LOCAL_RELAY_READY" "$LOCAL_RELAY_LOG"
}
trap cleanup EXIT

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
        if preferred and preferred not in (device.get("name"), device.get("udid")):
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

safe_remove_sim_container() {
  local container="$1"

  if [[ -z "$container" ]]; then
    return 0
  fi
  if [[ "$container" != *"/CoreSimulator/Devices/$DEVICE_UDID/"* ]]; then
    echo "FAIL: refusing to remove unexpected simulator container path: $container" >&2
    exit 1
  fi
  rm -rf "$container"
}

reset_sim_app_state() {
  local data_container group_container

  xcrun simctl uninstall "$DEVICE_UDID" "$BUNDLE_ID" >/dev/null 2>&1 || true
  xcrun simctl install "$DEVICE_UDID" "$APP_PATH" >/dev/null
  data_container="$(xcrun simctl get_app_container "$DEVICE_UDID" "$BUNDLE_ID" data 2>/dev/null || true)"
  group_container="$(xcrun simctl get_app_container "$DEVICE_UDID" "$BUNDLE_ID" "$APP_GROUP_ID" 2>/dev/null || true)"
  if [[ -z "$data_container" ]]; then
    echo "FAIL: simulator app data container was not created." >&2
    exit 1
  fi
  if [[ -z "$group_container" ]]; then
    echo "FAIL: simulator app group container was not created." >&2
    exit 1
  fi
  xcrun simctl terminate "$DEVICE_UDID" "$BUNDLE_ID" >/dev/null 2>&1 || true
  safe_remove_sim_container "$data_container/Library/Application Support/IrisDrive"
  SIM_APP_BASE_DIR="$group_container/IrisDrive"
  safe_remove_sim_container "$SIM_APP_BASE_DIR"
  mkdir -p "$SIM_APP_BASE_DIR"
  clear_sim_env \
    IRIS_DRIVE_DEBUG_ACTION \
    IRIS_DRIVE_DEBUG_OWNER \
    IRIS_DRIVE_FIPS_STATIC_PEERS \
    IRIS_DRIVE_FIPS_ENABLE_BOOTSTRAP \
    IRIS_DRIVE_FIPS_ENABLE_WEBRTC \
    IRIS_DRIVE_FIPS_UDP_BIND_ADDR \
    IRIS_DRIVE_FIPS_UDP_EXTERNAL_ADDR
  xcrun simctl spawn "$DEVICE_UDID" launchctl setenv \
    IRIS_DRIVE_UI_TEST_BASE_DIR "$SIM_APP_BASE_DIR" >/dev/null
}

clear_sim_env() {
  local key
  for key in "$@"; do
    xcrun simctl spawn "$DEVICE_UDID" launchctl unsetenv "$key" >/dev/null 2>&1 || true
  done
}

set_sim_env() {
  local assignment key value
  for assignment in "$@"; do
    key="${assignment%%=*}"
    value="${assignment#*=}"
    xcrun simctl spawn "$DEVICE_UDID" launchctl setenv "$key" "$value" >/dev/null
  done
}

launch_sim_app() {
  xcrun simctl terminate "$DEVICE_UDID" "$BUNDLE_ID" >/dev/null 2>&1 || true
  clear_sim_env IRIS_DRIVE_DEBUG_ACTION IRIS_DRIVE_DEBUG_OWNER
  set_sim_env "$@"
  xcrun simctl launch "$DEVICE_UDID" "$BUNDLE_ID" >/dev/null
}

wait_for_debug_state() {
  local state_file="$1"
  local jq_expr="$2"
  local seconds="$3"
  for _ in $(seq 1 "$((seconds * 5))"); do
    if [[ -f "$state_file" ]] && python3 -c "$jq_expr" <"$state_file" >/dev/null 2>&1; then
      return 0
    fi
    sleep 0.2
  done
  return 1
}

wait_for_config_status() {
  local config_dir="$1"
  local jq_expr="$2"
  local seconds="$3"
  for _ in $(seq 1 "$((seconds * 5))"); do
    if "$IDRIVE" --config-dir "$config_dir" status 2>/dev/null \
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

base64_value() {
  python3 -c 'import base64,sys; print(base64.b64encode(sys.stdin.buffer.read()).decode("ascii"))'
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

resolve_xctestrun() {
  find "$DERIVED_DATA/Build/Products" \
    -maxdepth 1 \
    -name "${SCHEME}_*.xctestrun" \
    -type f \
    -print \
    -quit 2>/dev/null
}

resolve_app_path() {
  find "$DERIVED_DATA/Build/Products" \
    -path "*/$CONFIGURATION-iphonesimulator/Iris Drive.app" \
    -type d \
    -print \
    -quit 2>/dev/null
}

resolve_share_source_app_path() {
  find "$DERIVED_DATA/Build/Products" \
    -path "*/$CONFIGURATION-iphonesimulator/Iris Drive Share Source.app" \
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

run_ui_test() {
  local use_app_group=0
  if [[ "${1:-}" == "--app-group" ]]; then
    use_app_group=1
    shift
  fi
  local only_testing="$1"
  shift
  local run_stem
  local run_file
  local env_updates=()
  local -a python_args
  while [[ $# -gt 0 ]]; do
    env_updates+=("$1")
    shift
  done
  run_stem="$(mktemp "$DERIVED_DATA/Build/Products/IrisDriveIOS-ui.XXXXXX")"
  run_file="$run_stem.xctestrun"
  mv "$run_stem" "$run_file"
  cp "$XCTESTRUN" "$run_file"

  if [[ "$use_app_group" != "1" ]]; then
    if [[ "${#env_updates[@]}" -gt 0 ]]; then
      env_updates=("IRIS_DRIVE_UI_TEST_BASE_DIR=$SIM_APP_BASE_DIR" "${env_updates[@]}")
    else
      env_updates=("IRIS_DRIVE_UI_TEST_BASE_DIR=$SIM_APP_BASE_DIR")
    fi
  fi

  python_args=("$run_file")
  if [[ "${#env_updates[@]}" -gt 0 ]]; then
    python_args+=("${env_updates[@]}")
  fi

  python3 - "${python_args[@]}" <<'PY'
import plistlib
import sys

path = sys.argv[1]
updates = {}
for assignment in sys.argv[2:]:
    key, separator, value = assignment.partition("=")
    if not separator:
        raise SystemExit(f"invalid environment assignment: {assignment}")
    updates[key] = value

with open(path, "rb") as handle:
    data = plistlib.load(handle)

for target in data.values():
    if not isinstance(target, dict) or not target.get("IsUITestBundle"):
        continue
    for env_key in (
        "EnvironmentVariables",
        "TestingEnvironmentVariables",
        "UITargetAppEnvironmentVariables",
    ):
        target.setdefault(env_key, {}).update(updates)

with open(path, "wb") as handle:
    plistlib.dump(data, handle)
PY

  local status=0
  xcodebuild \
    -xctestrun "$run_file" \
    -destination "$DESTINATION" \
    -only-testing:"$only_testing" \
    test-without-building >>"$BUILD_LOG" || status=$?
  rm -f "$run_file"
  return "$status"
}

reset_sim_app_group_state() {
  local data_container group_container

  xcrun simctl uninstall "$DEVICE_UDID" "$BUNDLE_ID" >/dev/null 2>&1 || true
  xcrun simctl uninstall "$DEVICE_UDID" "$SHARE_SOURCE_BUNDLE_ID" >/dev/null 2>&1 || true
  xcrun simctl install "$DEVICE_UDID" "$APP_PATH" >/dev/null
  xcrun simctl install "$DEVICE_UDID" "$SHARE_SOURCE_APP_PATH" >/dev/null
  data_container="$(xcrun simctl get_app_container "$DEVICE_UDID" "$BUNDLE_ID" data 2>/dev/null || true)"
  group_container="$(xcrun simctl get_app_container "$DEVICE_UDID" "$BUNDLE_ID" "$APP_GROUP_ID" 2>/dev/null || true)"
  if [[ -z "$data_container" ]]; then
    echo "FAIL: simulator app data container was not created for share-sheet smoke." >&2
    exit 1
  fi
  if [[ -z "$group_container" ]]; then
    echo "FAIL: simulator app group container was not created for share-sheet smoke." >&2
    exit 1
  fi
  xcrun simctl terminate "$DEVICE_UDID" "$BUNDLE_ID" >/dev/null 2>&1 || true
  xcrun simctl terminate "$DEVICE_UDID" "$SHARE_SOURCE_BUNDLE_ID" >/dev/null 2>&1 || true
  safe_remove_sim_container "$data_container/Library/Application Support/IrisDrive"
  safe_remove_sim_container "$group_container/IrisDrive"
  SIM_APP_BASE_DIR="$group_container/IrisDrive"
  mkdir -p "$SIM_APP_BASE_DIR"
  clear_sim_env IRIS_DRIVE_DEBUG_ACTION IRIS_DRIVE_DEBUG_OWNER IRIS_DRIVE_UI_TEST_BASE_DIR
}

verify_share_sheet_import() {
  local provider_json
  local output

  provider_json="$("$IDRIVE" --config-dir "$SIM_APP_BASE_DIR" provider list)"
  if ! PROVIDER_JSON="$provider_json" python3 - "$SHARE_SHEET_SMOKE_FILE" "$SHARE_SHEET_SMOKE_CONTENT" <<'PY'; then
import json
import os
import sys

expected_name = sys.argv[1]
expected_size = len(sys.argv[2].encode("utf-8"))
provider = json.loads(os.environ["PROVIDER_JSON"])
entries = provider.get("entries") or []
ok = (
    provider.get("file_count") == 1
    and any(
        entry.get("path") == expected_name
        and entry.get("kind") == "file"
        and entry.get("size") == expected_size
        for entry in entries
    )
)
raise SystemExit(0 if ok else 1)
PY
    echo "FAIL: iOS share-sheet import did not produce the expected provider entry." >&2
    echo "$provider_json" >&2
    exit 1
  fi

  output="$(mktemp -t iris-drive-ios-share-sheet.XXXXXX)"
  "$IDRIVE" --config-dir "$SIM_APP_BASE_DIR" provider read "$SHARE_SHEET_SMOKE_FILE" "$output" >/dev/null
  if ! python3 - "$output" "$SHARE_SHEET_SMOKE_CONTENT" <<'PY'; then
import pathlib
import sys

actual = pathlib.Path(sys.argv[1]).read_text(encoding="utf-8")
expected = sys.argv[2]
raise SystemExit(0 if actual == expected else 1)
PY
    echo "FAIL: iOS share-sheet import bytes did not match expected content." >&2
    exit 1
  fi
  rm -f "$output"
}

cargo build -p idrive

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

DEVICE_UDID="$(select_simulator)"
DESTINATION="platform=iOS Simulator,id=$DEVICE_UDID"

xcodebuild \
  -project "$PROJECT" \
  -scheme "$SCHEME" \
  -configuration "$CONFIGURATION" \
  -derivedDataPath "$DERIVED_DATA" \
  -destination "$DESTINATION" \
  CODE_SIGNING_ALLOWED=YES \
  CODE_SIGNING_REQUIRED=YES \
  CODE_SIGN_IDENTITY="${IRIS_DRIVE_IOS_CODE_SIGN_IDENTITY:--}" \
  PROVISIONING_PROFILE_SPECIFIER= \
  LIBRARY_SEARCH_PATHS="$RUST_LIB_DIR" \
  OTHER_LDFLAGS="$RUST_STATIC_LIB" \
  build-for-testing >"$BUILD_LOG"

APP_PATH="$(resolve_app_path)"
if [[ -z "$APP_PATH" || ! -d "$APP_PATH" ]]; then
  echo "FAIL: built iOS app not found. Build log: $BUILD_LOG" >&2
  exit 1
fi
SHARE_SOURCE_APP_PATH="$(resolve_share_source_app_path)"
if [[ -z "$SHARE_SOURCE_APP_PATH" || ! -d "$SHARE_SOURCE_APP_PATH" ]]; then
  echo "FAIL: built iOS share source app not found. Build log: $BUILD_LOG" >&2
  exit 1
fi
assert_static_app_core_linkage "$APP_PATH"
iris_drive_ios_assert_simulator_entitlements "$DERIVED_DATA" "$CONFIGURATION"

XCTESTRUN="$(resolve_xctestrun)"
if [[ -z "$XCTESTRUN" || ! -f "$XCTESTRUN" ]]; then
  echo "FAIL: iOS UI test run file not found. Build log: $BUILD_LOG" >&2
  exit 1
fi

xcrun simctl boot "$DEVICE_UDID" >/dev/null 2>&1 || true
xcrun simctl bootstatus "$DEVICE_UDID" -b >/dev/null

run_ui_test "IrisDriveIOSShareExtensionTests"

reset_sim_app_state
run_ui_test "IrisDriveIOSUITests/IrisDriveIOSUITests/testWelcomeRoutesWithoutSetupTitle"

reset_sim_app_state
run_ui_test "IrisDriveIOSUITests/IrisDriveIOSUITests/testLinkThisDeviceFromWelcome"

reset_sim_app_group_state
run_ui_test \
  --app-group \
  "IrisDriveIOSUITests/IrisDriveIOSUITests/testShareSheetImportsFileFromExternalSender" \
  "IRIS_DRIVE_UI_TEST_SHARE_SHEET_FILE=$SHARE_SHEET_SMOKE_FILE" \
  "IRIS_DRIVE_UI_TEST_SHARE_SHEET_CONTENT=$SHARE_SHEET_SMOKE_CONTENT"
verify_share_sheet_import

owner_json="$("$IDRIVE" --config-dir "$OWNER_CONFIG" init --force --label "CLI owner")"
configure_owner_local_relay
owner_invite="$(python3 -c 'import json,sys; print(json.load(sys.stdin)["app_key_link_invite"]["url"])' <<<"$owner_json")"
owner_app_key_npub="$(python3 -c 'import json,sys; print(json.load(sys.stdin)["current_app_key_npub"])' <<<"$owner_json")"
OWNER_FIPS_PORT="$(unused_loopback_port)"
owner_fips_peer="$owner_app_key_npub=127.0.0.1:$OWNER_FIPS_PORT"
IRIS_DRIVE_FIPS_UDP_BIND_ADDR="127.0.0.1:$OWNER_FIPS_PORT" \
  IRIS_DRIVE_FIPS_UDP_EXTERNAL_ADDR="127.0.0.1:$OWNER_FIPS_PORT" \
  IRIS_DRIVE_FIPS_UDP_PUBLIC=false \
  IRIS_DRIVE_FIPS_ENABLE_BOOTSTRAP=false \
  IRIS_DRIVE_FIPS_ENABLE_WEBRTC=false \
  "$IDRIVE" --config-dir "$OWNER_CONFIG" daemon --watch-interval 0 --no-gateway \
  >"$OWNER_DAEMON_LOG" 2>&1 &
OWNER_DAEMON_PID="$!"
if ! wait_for_owner_fips 20; then
  echo "FAIL: owner daemon did not start FIPS for iOS GUI link delivery." >&2
  cat "$OWNER_DAEMON_LOG" >&2 || true
  exit 1
fi

reset_sim_app_state
launch_sim_app \
  "IRIS_DRIVE_FIPS_STATIC_PEERS=$owner_fips_peer" \
  "IRIS_DRIVE_FIPS_ENABLE_BOOTSTRAP=false" \
  "IRIS_DRIVE_FIPS_ENABLE_WEBRTC=false" \
  "IRIS_DRIVE_FIPS_UDP_BIND_ADDR=127.0.0.1:0" \
  "IRIS_DRIVE_FIPS_UDP_EXTERNAL_ADDR=" \
  "IRIS_DRIVE_DEBUG_ACTION=link-device" \
  "IRIS_DRIVE_DEBUG_OWNER=$owner_invite"

STATE_FILE="$SIM_APP_BASE_DIR/debug-state.json"
if ! wait_for_config_status \
  "$SIM_APP_BASE_DIR" \
  'import json,sys; s=json.load(sys.stdin); a=s.get("profile") or {}; raise SystemExit(0 if a.get("authorization_state") == "awaiting_approval" and a.get("app_key_link_request") else 1)' \
  15; then
  echo "FAIL: iOS owner invite did not create an awaiting linked-device profile." >&2
  [[ -f "$STATE_FILE" ]] && cat "$STATE_FILE" >&2
  exit 1
fi
run_ui_test \
  "IrisDriveIOSUITests/IrisDriveIOSUITests/testAwaitingApprovalViewVisible" \
  "IRIS_DRIVE_FIPS_STATIC_PEERS=$owner_fips_peer" \
  "IRIS_DRIVE_FIPS_ENABLE_BOOTSTRAP=false" \
  "IRIS_DRIVE_FIPS_ENABLE_WEBRTC=false" \
  "IRIS_DRIVE_FIPS_UDP_BIND_ADDR=127.0.0.1:0" \
  "IRIS_DRIVE_FIPS_UDP_EXTERNAL_ADDR="
launch_sim_app \
  "IRIS_DRIVE_FIPS_STATIC_PEERS=$owner_fips_peer" \
  "IRIS_DRIVE_FIPS_ENABLE_BOOTSTRAP=false" \
  "IRIS_DRIVE_FIPS_ENABLE_WEBRTC=false" \
  "IRIS_DRIVE_FIPS_UDP_BIND_ADDR=127.0.0.1:0" \
  "IRIS_DRIVE_FIPS_UDP_EXTERNAL_ADDR="
linked_device="$("$IDRIVE" --config-dir "$SIM_APP_BASE_DIR" status \
  | python3 -c 'import json,sys; print(json.load(sys.stdin)["profile"]["current_app_key_npub"])')"
if ! wait_for_owner_inbound_request "$linked_device" 30; then
  echo "FAIL: owner did not receive the iOS GUI app-key-link request over FIPS." >&2
  "$IDRIVE" --config-dir "$OWNER_CONFIG" status >&2 || true
  cat "$OWNER_DAEMON_LOG" >&2 || true
  exit 1
fi
request_url="$(owner_inbound_request_url "$linked_device")"
approve_status=0
approved_json="$("$IDRIVE" --config-dir "$OWNER_CONFIG" approve "$request_url" --label "iOS UI linked")" || approve_status="$?"
if [[ "$approve_status" != "0" ]]; then
  echo "FAIL: CLI owner could not approve the inbound iOS UI request." >&2
  echo "request_url_length=${#request_url}" >&2
  echo "request_url_prefix=${request_url:0:96}" >&2
  "$IDRIVE" --config-dir "$OWNER_CONFIG" status >&2 || true
  cat "$OWNER_DAEMON_LOG" >&2 || true
  exit "$approve_status"
fi
roster_size="$(python3 -c 'import json,sys; print(json.load(sys.stdin)["roster_size"])' <<<"$approved_json")"
if [[ "$roster_size" != "2" ]]; then
  echo "FAIL: CLI owner did not approve the inbound iOS UI link request." >&2
  echo "$approved_json" >&2
  exit 1
fi

launch_sim_app \
  "IRIS_DRIVE_FIPS_STATIC_PEERS=$owner_fips_peer" \
  "IRIS_DRIVE_FIPS_ENABLE_BOOTSTRAP=false" \
  "IRIS_DRIVE_FIPS_ENABLE_WEBRTC=false" \
  "IRIS_DRIVE_FIPS_UDP_BIND_ADDR=127.0.0.1:0" \
  "IRIS_DRIVE_FIPS_UDP_EXTERNAL_ADDR="

STATE_FILE="$SIM_APP_BASE_DIR/debug-state.json"
if ! wait_for_config_status \
  "$SIM_APP_BASE_DIR" \
  'import json,sys; s=json.load(sys.stdin); a=s.get("profile") or {}; raise SystemExit(0 if a.get("authorization_state") == "authorized" else 1)' \
  90; then
  echo "FAIL: iOS GUI device did not ingest owner approval before the approved-device UI check." >&2
  [[ -f "$STATE_FILE" ]] && cat "$STATE_FILE" >&2
  "$IDRIVE" --config-dir "$OWNER_CONFIG" status >&2 || true
  cat "$OWNER_DAEMON_LOG" >&2 || true
  exit 1
fi

run_ui_test \
  "IrisDriveIOSUITests/IrisDriveIOSUITests/testApprovedLinkedDeviceLeavesWaiting" \
  "IRIS_DRIVE_FIPS_STATIC_PEERS=$owner_fips_peer" \
  "IRIS_DRIVE_FIPS_ENABLE_BOOTSTRAP=false" \
  "IRIS_DRIVE_FIPS_ENABLE_WEBRTC=false" \
  "IRIS_DRIVE_FIPS_UDP_BIND_ADDR=127.0.0.1:0" \
  "IRIS_DRIVE_FIPS_UDP_EXTERNAL_ADDR="

STATE_FILE="$SIM_APP_BASE_DIR/debug-state.json"
if ! wait_for_debug_state \
  "$STATE_FILE" \
  'import json,sys; s=json.load(sys.stdin); ui=s.get("ui",{}); a=ui.get("profile") or {}; current=a.get("current_app_key_npub") or a.get("device_pubkey"); devices=ui.get("app_actors") or ui.get("devices") or []; ok=ui.get("setup_complete") and not ui.get("awaiting_approval") and a.get("authorization_state") == "authorized" and any(d.get("pubkey") == current and (d.get("is_current_app_key") or d.get("is_current_device")) and d.get("state") == "Linked" for d in devices); raise SystemExit(0 if ok else 1)' \
  45; then
  echo "FAIL: iOS GUI device did not show linked/authorized after the owner approved its FIPS request." >&2
  [[ -f "$STATE_FILE" ]] && cat "$STATE_FILE" >&2
  "$IDRIVE" --config-dir "$OWNER_CONFIG" status >&2 || true
  cat "$OWNER_DAEMON_LOG" >&2 || true
  exit 1
fi

reset_sim_app_state
run_ui_test "IrisDriveIOSUITests/IrisDriveIOSUITests/testCreateProfileFromWelcome"
reset_sim_app_state
run_ui_test "IrisDriveIOSUITests/IrisDriveIOSUITests/testCreateProfileWithUsernameCanSkipProfilePhoto"
run_ui_test "IrisDriveIOSUITests/IrisDriveIOSUITests/testOpenIrisAppsLoadsBrowserWithoutConnectionError"
run_ui_test "IrisDriveIOSUITests/IrisDriveIOSUITests/testOpenIrisAppsLoadsBrowserWhenSyncPaused"
run_ui_test "IrisDriveIOSUITests/IrisDriveIOSUITests/testOpenDriveFolderInFilesApp"

STATE_FILE="$SIM_APP_BASE_DIR/debug-state.json"
if ! wait_for_debug_state \
  "$STATE_FILE" \
  'import json,sys; s=json.load(sys.stdin); a=s.get("ui",{}).get("profile") or {}; raise SystemExit(0 if a.get("authorization_state") == "authorized" and a.get("app_key_link_invite") else 1)' \
  15; then
  echo "FAIL: iOS Create profile UI did not initialize an owner profile." >&2
  [[ -f "$STATE_FILE" ]] && cat "$STATE_FILE" >&2
  exit 1
fi
app_invite="$(python3 -c 'import json,sys; print(json.load(sys.stdin)["ui"]["profile"]["app_key_link_invite"])' <"$STATE_FILE")"
linked_json="$("$IDRIVE" --config-dir "$LINKED_CONFIG" link "$app_invite" --label "iOS UI linked")"
linked_device="$(python3 -c 'import json,sys; print(json.load(sys.stdin)["current_app_key_npub"])' <<<"$linked_json")"
linked_request="$(python3 -c 'import json,sys; print(json.load(sys.stdin)["app_key_link_request"]["url"])' <<<"$linked_json")"
linked_request_file="$SIM_APP_BASE_DIR/linked-device-request.txt"
printf '%s' "$linked_request" >"$linked_request_file"
linked_request_b64="$(printf '%s' "$linked_request" | base64_value)"

run_ui_test \
  "IrisDriveIOSUITests/IrisDriveIOSUITests/testAddLinkedDeviceFromDevices" \
  "IRIS_DRIVE_UI_TEST_LINKED_DEVICE=$linked_device" \
  "IRIS_DRIVE_UI_TEST_LINKED_DEVICE_REQUEST_B64=$linked_request_b64" \
  "IRIS_DRIVE_UI_TEST_LINKED_DEVICE_REQUEST_FILE=$linked_request_file" \
  "IRIS_DRIVE_UI_TEST_LINKED_DEVICE_LABEL=iOS UI linked"

STATE_FILE="$SIM_APP_BASE_DIR/debug-state.json"
if ! wait_for_debug_state \
  "$STATE_FILE" \
  'import json,sys; s=json.load(sys.stdin); ui=s.get("ui",{}); devices=ui.get("app_actors") or ui.get("devices") or []; raise SystemExit(0 if any(d.get("role") == "member" and d.get("state") == "Linked" and d.get("display_label") == "iOS UI linked" for d in devices) and len(devices) >= 2 else 1)' \
  15; then
  echo "FAIL: iOS Add Device UI did not add the linked device." >&2
  [[ -f "$STATE_FILE" ]] && cat "$STATE_FILE" >&2
  exit 1
fi

echo "IOS_GUI_LINKING_SMOKE_OK"
echo "device=$DEVICE_UDID"
echo "build_log=$BUILD_LOG"
