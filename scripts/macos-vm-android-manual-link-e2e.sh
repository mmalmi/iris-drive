#!/usr/bin/env bash
# Bidirectional shipped-UI manual linking: macOS VM <-> physical Android.
set -Eeuo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
ENV_FILE="${IRIS_DRIVE_DEV_LAB_ENV:-$HOME/.config/iris-drive/dev-lab.env}"
if [[ -f "$ENV_FILE" ]]; then
  set -a
  # shellcheck disable=SC1090
  source "$ENV_FILE"
  set +a
fi

MODE="${IRIS_DRIVE_MACOS_ANDROID_MANUAL_LINKING:-auto}"
MAC_HOST="${IRIS_DRIVE_MACOS_SSH_HOST:-}"
MAC_REMOTE_NAME="${IRIS_DRIVE_DEV_VM_MACOS_REMOTE:-macos}"
if [[ -z "$MAC_HOST" ]] && MAC_REMOTE_URL="$(git -C "$ROOT" remote get-url "$MAC_REMOTE_NAME" 2>/dev/null)"; then
  MAC_HOST="${MAC_REMOTE_URL%%:*}"
  MAC_HOST="${MAC_HOST#*@}"
fi
MAC_GUEST_REPO="${IRIS_DRIVE_MACOS_GUEST_SRC_ROOT:-src}/iris-drive-release-gate"
REMOTE_SCRIPT="./scripts/macos-android-manual-link-remote.sh"
WAIT_SECS="${IRIS_DRIVE_MACOS_ANDROID_WAIT_SECS:-15}"
POST_LINK_WAIT_SECS="${IRIS_DRIVE_MACOS_ANDROID_POST_LINK_WAIT_SECS:-45}"
REUSE_ANDROID="${IRIS_DRIVE_MOBILE_REUSE_ANDROID_ARTIFACTS:-0}"
PACKAGE="${IRIS_DRIVE_ANDROID_PHYSICAL_LINK_PACKAGE:-to.iris.drive.uitest}"
ACTIVITY="${IRIS_DRIVE_ANDROID_PHYSICAL_LINK_ACTIVITY:-$PACKAGE/to.iris.drive.app.MainActivity}"
APK="${IRIS_DRIVE_ANDROID_PHYSICAL_LINK_APK:-$ROOT/android/app/build/outputs/apk/uiTest/app-uiTest.apk}"
TEST_APK="${IRIS_DRIVE_ANDROID_PHYSICAL_LINK_TEST_APK:-$ROOT/android/app/build/outputs/apk/androidTest/uiTest/app-uiTest-androidTest.apk}"
TEST_RUNNER="${IRIS_DRIVE_ANDROID_PHYSICAL_LINK_TEST_RUNNER:-$PACKAGE.test/androidx.test.runner.AndroidJUnitRunner}"
TEST_PACKAGE="${IRIS_DRIVE_ANDROID_PHYSICAL_LINK_TEST_PACKAGE:-${TEST_RUNNER%%/*}}"
DEBUG_ACTION_EXTRA="${IRIS_DRIVE_ANDROID_DEBUG_ACTION_EXTRA:-to.iris.drive.DEBUG_ACTION}"
RUN_ID="$(date +%s)-$$-$RANDOM"
RESULT_DIR="${IRIS_DRIVE_MACOS_ANDROID_RESULT_DIR:-$ROOT/artifacts/macos-android-manual-link/$RUN_ID}"
SUMMARY="$RESULT_DIR/summary.json"
TMP=""
ADB=""
SERIAL=""
REMOTE_DIRTY=0

bool_true() {
  case "${1:-}" in
    1|true|TRUE|True|yes|YES|Yes|on|ON|On|required) return 0 ;;
    *) return 1 ;;
  esac
}

fail() {
  echo "physical macOS/Android manual-link e2e failed: $*" >&2
  exit 1
}

monotonic_milliseconds() {
  perl -MTime::HiRes=clock_gettime,CLOCK_MONOTONIC \
    -e 'printf "%.0f\n", clock_gettime(CLOCK_MONOTONIC) * 1000'
}

base64_url() {
  python3 -c 'import base64,sys; print(base64.urlsafe_b64encode(sys.stdin.buffer.read()).decode().rstrip("="))'
}

emit_skip() {
  local reason="$1"
  mkdir -p "$RESULT_DIR"
  python3 - "$MODE" "$reason" >"$SUMMARY" <<'PY'
import json, sys
print(json.dumps({"ok": False, "skipped": True, "mode": sys.argv[1], "reason": sys.argv[2]}, sort_keys=True))
PY
  echo "MACOS_ANDROID_MANUAL_LINKING_SKIPPED mode=$MODE reason=$reason summary=$SUMMARY"
}

remote() {
  local command="cd '$MAC_GUEST_REPO' && '$REMOTE_SCRIPT'" arg
  shift 0
  for arg in "$@"; do
    command+=" $(printf '%q' "$arg")"
  done
  ssh -o BatchMode=yes "$MAC_HOST" "$command"
}

capture_remote_evidence() {
  local role="$1"
  local evidence
  [[ -n "$TMP" ]] || return 0
  evidence="$TMP/macos-$role-evidence.json"
  [[ -f "$RESULT_DIR/macos-$role-evidence.json" ]] && return
  remote evidence "$role" >"$evidence" 2>/dev/null || return
  cp "$evidence" "$RESULT_DIR/macos-$role-evidence.json"
}

cleanup() {
  local status=$?
  trap - EXIT
  set +e
  if [[ "$REMOTE_DIRTY" == 1 ]]; then
    capture_remote_evidence owner || true
    capture_remote_evidence joiner || true
    remote cleanup >/dev/null 2>&1 || true
  fi
  if [[ -n "$ADB" && -n "$SERIAL" ]]; then
    "$ADB" -s "$SERIAL" shell am force-stop "$PACKAGE" >/dev/null 2>&1 || true
    "$ADB" -s "$SERIAL" shell am force-stop "$TEST_PACKAGE" >/dev/null 2>&1 || true
    "$ADB" -s "$SERIAL" uninstall "$TEST_PACKAGE" >/dev/null 2>&1 || true
    [[ "$PACKAGE" == "$TEST_PACKAGE" ]] \
      || "$ADB" -s "$SERIAL" uninstall "$PACKAGE" >/dev/null 2>&1 || true
  fi
  [[ -z "$TMP" ]] || rm -rf "$TMP"
  exit "$status"
}
trap cleanup EXIT

case "$MODE" in
  0|false|FALSE|False|no|NO|No|off|OFF|Off)
    MODE=disabled
    emit_skip explicit
    exit 0
    ;;
  auto|AUTO|Auto|"") MODE=auto ;;
  1|true|TRUE|True|yes|YES|Yes|on|ON|On|required) MODE=required ;;
  *) fail "unsupported IRIS_DRIVE_MACOS_ANDROID_MANUAL_LINKING=$MODE" ;;
esac
if [[ ! "$WAIT_SECS" =~ ^[1-9][0-9]*$ ]] || ((WAIT_SECS > 15)); then
  fail "IRIS_DRIVE_MACOS_ANDROID_WAIT_SECS must be 1..15"
fi
[[ "$POST_LINK_WAIT_SECS" =~ ^[1-9][0-9]*$ ]] \
  || fail "IRIS_DRIVE_MACOS_ANDROID_POST_LINK_WAIT_SECS must be positive"
[[ "$TEST_RUNNER" == */* ]] \
  || fail "IRIS_DRIVE_ANDROID_PHYSICAL_LINK_TEST_RUNNER must be package/runner"
[[ "$PACKAGE" =~ ^[A-Za-z][A-Za-z0-9_.]*$ \
  && "$TEST_PACKAGE" =~ ^[A-Za-z][A-Za-z0-9_.]*$ ]] \
  || fail "physical Android package names are invalid"
if [[ -z "$MAC_HOST" ]]; then
  [[ "$MODE" == auto ]] && { emit_skip macos_vm_unavailable; exit 0; }
  fail "set IRIS_DRIVE_MACOS_SSH_HOST"
fi

resolve_adb() {
  local sdk="${ANDROID_HOME:-${ANDROID_SDK_ROOT:-}}"
  [[ -n "$sdk" || ! -f "$ROOT/android/local.properties" ]] \
    || sdk="$(sed -n 's/^sdk\.dir=//p' "$ROOT/android/local.properties" | head -n 1)"
  [[ -n "$sdk" || ! -d "$HOME/Library/Android/sdk" ]] || sdk="$HOME/Library/Android/sdk"
  if [[ -n "$sdk" && -x "$sdk/platform-tools/adb" ]]; then
    printf '%s\n' "$sdk/platform-tools/adb"
  else
    command -v adb
  fi
}

select_physical_android() {
  local requested="${IRIS_DRIVE_ANDROID_SERIAL:-${ANDROID_SERIAL:-}}" serial candidates=""
  while read -r serial state; do
    [[ "$state" == device && "$serial" != emulator-* ]] || continue
    [[ "$("$ADB" -s "$serial" shell getprop ro.kernel.qemu 2>/dev/null | tr -d '\r')" != 1 ]] \
      || continue
    candidates+="${candidates:+$'\n'}$serial"
  done < <("$ADB" devices | tail -n +2)
  if [[ -n "$requested" ]]; then
    printf '%s\n' "$candidates" | grep -Fx "$requested"
    return
  fi
  [[ "$(printf '%s\n' "$candidates" | sed '/^$/d' | wc -l | tr -d ' ')" == 1 ]] \
    || return 1
  printf '%s\n' "$candidates"
}

redact_physical_identifiers() {
  python3 -u -c 'import re, sys
for text in sys.stdin:
    for value in sys.argv[1:]: text=text.replace(value, "<physical-device>")
    for key in ("allocation", "host", "name"):
        text=re.sub(r"(\\\"" + key + r"\\\"\\s*:\\s*)\\\"(?:\\\\.|[^\\\"])*\\\"", r"\\1\\\"<redacted>\\\"", text)
    sys.stdout.write(text)' "$@"
}

reserve_physical_android() {
  bool_true "${IRIS_DRIVE_NATIVE_ANDROID_RESERVED:-0}" && return
  [[ "${IRIS_DRIVE_LAB_ALLOCATED_ANDROID:-}" == "$SERIAL" ]] && return

  set +e
  python3 "$ROOT/scripts/native_lab.py" run \
    --resource iris-drive-macos-android-manual-link \
    --health "android:$SERIAL" \
    --health "ssh:$MAC_HOST" \
    --allocation-env android=IRIS_DRIVE_LAB_ALLOCATED_ANDROID \
    -- env IRIS_DRIVE_NATIVE_ANDROID_RESERVED=1 "$0" "$@" 2>&1 \
    | redact_physical_identifiers "$SERIAL" "$MAC_HOST"
  local status="${PIPESTATUS[0]}"
  set -e
  exit "$status"
}

if ! ADB="$(resolve_adb 2>/dev/null)"; then
  [[ "$MODE" == auto ]] && { emit_skip adb_unavailable; exit 0; }
  fail "adb is required"
fi
if ! SERIAL="$(select_physical_android)"; then
  [[ "$MODE" == auto ]] && { emit_skip physical_android_unavailable; exit 0; }
  fail "select exactly one online non-emulated Android device"
fi
reserve_physical_android "$@"

TMP="$(mktemp -d -t iris-drive-macos-android-link.XXXXXX)"
mkdir -p "$RESULT_DIR"
chmod 700 "$RESULT_DIR" "$TMP"

android_artifacts_valid() {
  python3 - "$APK" "$TEST_APK" <<'PY'
from pathlib import Path
import sys, zipfile
for artifact, required in (
    (Path(sys.argv[1]), {"AndroidManifest.xml", "classes.dex", "lib/arm64-v8a/libiris_drive_app_core.so"}),
    (Path(sys.argv[2]), {"AndroidManifest.xml", "classes.dex"}),
):
    if not artifact.is_file() or not artifact.stat().st_size:
        raise SystemExit(1)
    with zipfile.ZipFile(artifact) as archive:
        if archive.testzip() is not None or not required.issubset(archive.namelist()):
            raise SystemExit(1)
PY
}

if bool_true "$REUSE_ANDROID" && android_artifacts_valid; then
  echo "[macos-android-link] reusing validated Android artifacts" >&2
else
  "$ROOT/tools/run-android" :app:assembleUiTest :app:assembleUiTestAndroidTest
fi
android_artifacts_valid || fail "Android UI/provider test artifacts are missing or invalid"
"$ADB" -s "$SERIAL" install -r -t "$APK" >/dev/null
"$ADB" -s "$SERIAL" install -r -t "$TEST_APK" >/dev/null

case "${IRIS_DRIVE_MACOS_SKIP_GIT_SYNC:-0}" in
  1|true|TRUE|True|yes|YES|Yes|on|ON|On) ;;
  *) "$ROOT/scripts/macos-vm-git-sync.sh" "$MAC_HOST" ;;
esac
REMOTE_DIRTY=1
owner_fixture="$(remote prepare)"
MAC_OWNER_NPUB="$(printf '%s\n' "$owner_fixture" | tail -n 1 \
  | python3 -c 'import json,sys; print(json.load(sys.stdin)["owner_app_key_npub"])')"
[[ "$MAC_OWNER_NPUB" == npub1* ]] || fail "macOS owner identity is invalid"

android_dump_ui() {
  "$ADB" -s "$SERIAL" shell uiautomator dump /sdcard/iris-drive-manual-link.xml >/dev/null 2>&1
  "$ADB" -s "$SERIAL" exec-out cat /sdcard/iris-drive-manual-link.xml >"$TMP/android-ui.xml"
}

android_ui_point() {
  android_dump_ui || return 1
  python3 "$ROOT/scripts/lib/android-ui-point.py" "$TMP/android-ui.xml" "$1" "$2"
}

wait_android_ui() {
  local kind="$1" value="$2" seconds="$3"
  local deadline=$((SECONDS + seconds))
  while ((SECONDS < deadline)); do
    android_ui_point "$kind" "$value" >/dev/null 2>&1 && return 0
    sleep 0.2
  done
  return 1
}

tap_android_ui() {
  local point
  point="$(android_ui_point "$1" "$2")" || return 1
  # shellcheck disable=SC2086
  "$ADB" -s "$SERIAL" shell input tap $point >/dev/null
}

wait_and_tap_text() {
  wait_android_ui text "$1" "$2" && tap_android_ui text "$1"
}

tap_android_until_ui() {
  local tap_value="$1" result_kind="$2" result_value="$3" deadline=$((SECONDS + $4))
  while ((SECONDS < deadline)); do
    android_ui_point "$result_kind" "$result_value" >/dev/null 2>&1 && return 0
    tap_android_ui text "$tap_value" >/dev/null 2>&1 || true
    sleep 0.25
  done
  return 1
}

start_android_app() {
  "$ADB" -s "$SERIAL" shell am start -W -n "$ACTIVITY" >/dev/null
}

android_debug_action() {
  "$ADB" -s "$SERIAL" shell am start -W -n "$ACTIVITY" \
    --es "$DEBUG_ACTION_EXTRA" "$1" >/dev/null
}

prepare_android_fresh() {
  "$ADB" -s "$SERIAL" shell am force-stop "$PACKAGE" >/dev/null 2>&1 || true
  "$ADB" -s "$SERIAL" shell pm clear "$PACKAGE" >/dev/null
  start_android_app
}

copy_android_file() {
  "$ADB" -s "$SERIAL" exec-out run-as "$PACKAGE" cat "files/$1" >"$2" 2>/dev/null
}

refresh_android_state() {
  android_debug_action refresh
  sleep 0.15
  copy_android_file debug-state.json "$TMP/android-state.json"
}

android_profile_value() {
  refresh_android_state
  python3 - "$TMP/android-state.json" "$1" <<'PY'
import json, sys
value=((json.load(open(sys.argv[1], encoding="utf-8")).get("ui") or {}).get("profile") or {}).get(sys.argv[2], "")
if not value: raise SystemExit(1)
print(value)
PY
}

create_android_owner() {
  prepare_android_fresh
  tap_android_until_ui "Create profile" text "Username (optional)" 15 \
    || fail "Android shipped profile form did not open"
  tap_android_until_ui "Create profile" text "My Drive" 20 \
    || fail "Android owner did not reach My Drive"
  android_debug_action start-sync
}

create_android_join_request() {
  prepare_android_fresh
  tap_android_until_ui "Sign in" desc "Device approval request QR" 15 \
    || fail "Android shipped join request UI was unavailable"
  android_debug_action start-sync
}

open_android_manual_approval() {
  wait_and_tap_text "Devices" 10 || fail "Android Devices tab was unavailable"
  if ! wait_android_ui text "Add Device" 3; then
    "$ADB" -s "$SERIAL" shell input swipe 540 1800 540 600 350 >/dev/null
  fi
  wait_and_tap_text "Add Device" 10 || fail "Android Add Device panel was unavailable"
  wait_android_ui desc "Manual device approval request" 10 \
    || fail "Android manual approval field was unavailable"
}

enter_android_approval_request() {
  tap_android_ui desc "Manual device approval request" || return 1
  "$ADB" -s "$SERIAL" shell input text "$1" >/dev/null
  wait_android_ui text "Approve" 10
}

android_link_state() {
  local peer="$1" owner="$2"
  android_debug_action refresh >/dev/null 2>&1 || true
  copy_android_file config.toml "$TMP/android-config.toml" || return 1
  copy_android_file native-fips-status.json "$TMP/android-fips.json" || return 1
  python3 - "$TMP/android-config.toml" "$TMP/android-fips.json" "$peer" "$owner" <<'PY'
import json, re, sys
config=open(sys.argv[1], encoding="utf-8").read()
with open(sys.argv[2], encoding="utf-8") as fh: fips=json.load(fh)
authorized=bool(re.search(r'^authorization_state\s*=\s*"authorized"\s*$', config, re.MULTILINE))
receipts="pending_device_approval_receipts" in config
peer=sys.argv[3]
authorized_peer = peer in (fips.get("authorized_peers") or [])
online = peer in (fips.get("online_devices") or []) or peer in (fips.get("online_peers") or [])
owner_ok=sys.argv[4] != "1" or not receipts
raise SystemExit(0 if authorized and owner_ok and authorized_peer and online else 1)
PY
}

write_android_evidence() {
  local role="$1" peer="$2" destination="$3"
  android_debug_action refresh >/dev/null 2>&1 || true
  copy_android_file config.toml "$TMP/android-config.toml"
  copy_android_file native-fips-status.json "$TMP/android-fips.json"
  python3 - "$TMP/android-config.toml" "$TMP/android-fips.json" "$role" "$peer" >"$destination" <<'PY'
import json, re, sys
config=open(sys.argv[1], encoding="utf-8").read()
fips=json.load(open(sys.argv[2], encoding="utf-8")); peer=sys.argv[4]
state=re.search(r'^authorization_state\s*=\s*"([^"]+)"\s*$', config, re.MULTILINE)
online=peer in (fips.get("online_devices") or []) or peer in (fips.get("online_peers") or [])
direct=peer in (fips.get("direct_devices") or []) or peer in (fips.get("direct_peers") or [])
mesh=peer in (fips.get("mesh_devices") or []) or peer in (fips.get("mesh_peers") or [])
print(json.dumps({
    "role": sys.argv[3],
    "authorization_state": state.group(1) if state else None,
    "pending_device_approval_receipts_zero": "pending_device_approval_receipts" not in config,
    "fips": {
        "running": fips.get("running") is True,
        "connected_peer_count": int(fips.get("connected_peer_count") or 0),
        "authorized_peer_count": int(fips.get("authorized_peer_count") or 0),
        "expected_peer_authorized": peer in (fips.get("authorized_peers") or []),
        "expected_peer_online": online,
        "expected_peer_direct": direct,
        "expected_peer_mesh": mesh,
    },
}, indent=2, sort_keys=True))
PY
}

android_fips_ready() {
  android_debug_action refresh >/dev/null 2>&1 || true
  copy_android_file native-fips-status.json "$TMP/android-fips.json" || return 1
  python3 - "$TMP/android-fips.json" <<'PY'
import json, sys
f=json.load(open(sys.argv[1], encoding="utf-8"))
raise SystemExit(0 if f.get("running") is True and int(f.get("connected_peer_count") or 0) > 0 else 1)
PY
}

wait_delivery_ready() {
  local mac_role="$1" deadline=$((SECONDS + 30))
  while ((SECONDS < deadline)); do
    if android_fips_ready && remote fips-ready "$mac_role" >/dev/null 2>&1; then
      return 0
    fi
    sleep 0.25
  done
  return 1
}

wait_link_before() {
  local label="$1" started="$2" mac_role="$3" android_peer="$4" mac_peer="$5" android_owner="$6"
  local deadline=$((started + WAIT_SECS * 1000)) now
  while true; do
    now="$(monotonic_milliseconds)"
    ((now <= deadline)) || fail "$label exceeded the strict $((WAIT_SECS * 1000))ms ceiling"
    if android_link_state "$android_peer" "$android_owner" \
      && remote link-ready "$mac_role" "$mac_peer" >/dev/null 2>&1; then
      now="$(monotonic_milliseconds)"
      ((now <= deadline)) || fail "$label became true after $((now - started))ms"
      printf '%s\n' "$((now - started))"
      return
    fi
    sleep 0.1
  done
}

run_android_provider_test() {
  local method="$1" name="$2" content="$3"
  local log="$TMP/android-$method.log"
  "$ADB" -s "$SERIAL" shell am instrument -w -r \
    -e class "to.iris.drive.app.provider.IrisDrivePhysicalLinkingProviderTest#$method" \
    -e file_name "$name" \
    -e content_b64 "$(printf '%s' "$content" | base64_url)" \
    -e wait_millis "$((POST_LINK_WAIT_SECS * 1000))" \
    "$TEST_RUNNER" >"$log" 2>&1 || return 1
  grep -Eq 'OK \(1 test\)|IRIS_ANDROID_PROVIDER_(WRITE|READ)_SHA256=' "$log"
}

# macOS owner -> physical Android joiner.
remote start-owner
create_android_join_request
ANDROID_JOINER_NPUB="$(android_profile_value current_app_key_npub)" \
  || fail "Android joining identity was unavailable"
ANDROID_REQUEST="$(android_profile_value app_key_link_request)" \
  || fail "Android manual approval request URL was unavailable"
[[ "$ANDROID_REQUEST" == https://drive.iris.to/approve-device/* ]] \
  || fail "Android request URL is not canonical"
wait_delivery_ready owner \
  || fail "macOS owner and Android joiner delivery stacks were not ready"
remote manual-prepare "$ANDROID_REQUEST"
first_started="$(monotonic_milliseconds)"
remote manual-submit
first_elapsed="$(wait_link_before \
  "macOS owner -> physical Android joiner" "$first_started" owner \
  "$MAC_OWNER_NPUB" "$ANDROID_JOINER_NPUB" 0)"
wait_android_ui text "My Drive" 5 || fail "Android joined UI did not reach My Drive"
first_file="macos-owner-android-joiner-$RUN_ID.txt"
first_content="physical Android joiner provider write $RUN_ID"
run_android_provider_test writePostLinkFileThroughDocumentsProvider "$first_file" "$first_content" \
  || fail "Android provider could not write after macOS approval"
start_android_app
android_debug_action start-sync
remote wait-provider-read owner "$first_file" "$(printf '%s' "$first_content" | base64_url)"
capture_remote_evidence owner
write_android_evidence joiner "$MAC_OWNER_NPUB" "$RESULT_DIR/android-joiner-evidence.json"
remote stop owner

# Physical Android owner -> macOS joiner.
create_android_owner
ANDROID_OWNER_NPUB="$(android_profile_value current_app_key_npub)" \
  || fail "Android owner identity was unavailable"
MAC_REQUEST_OUTPUT="$(remote start-joiner)"
MAC_REQUEST="$(printf '%s\n' "$MAC_REQUEST_OUTPUT" | tail -n 1)"
[[ "$MAC_REQUEST" == https://drive.iris.to/approve-device/* ]] \
  || fail "macOS shipped Sign in UI did not return a canonical request"
MAC_JOINER_NPUB="$(remote status joiner | python3 -c 'import json,sys; print((json.load(sys.stdin).get("profile") or {}).get("current_app_key_npub", ""))')"
[[ "$MAC_JOINER_NPUB" == npub1* ]] || fail "macOS joining identity is invalid"
wait_delivery_ready joiner \
  || fail "Android owner and macOS joiner delivery stacks were not ready"
open_android_manual_approval
enter_android_approval_request "$MAC_REQUEST" \
  || fail "Android shipped manual field did not open exact approval confirmation"
second_started="$(monotonic_milliseconds)"
tap_android_ui text "Approve" || fail "Android exact Approve action was unavailable"
second_elapsed="$(wait_link_before \
  "physical Android owner -> macOS joiner" "$second_started" joiner \
  "$MAC_JOINER_NPUB" "$ANDROID_OWNER_NPUB" 1)"
remote assert-joined-ui
second_file="android-owner-macos-joiner-$RUN_ID.txt"
second_content="macOS joiner provider write $RUN_ID"
remote provider-write joiner "$second_file" "$(printf '%s' "$second_content" | base64_url)"
start_android_app
android_debug_action start-sync
run_android_provider_test readPostLinkFileThroughDocumentsProvider "$second_file" "$second_content" \
  || fail "Android provider could not read the macOS post-link write"
capture_remote_evidence joiner
write_android_evidence owner "$MAC_JOINER_NPUB" "$RESULT_DIR/android-owner-evidence.json"

remote cleanup
REMOTE_DIRTY=0
python3 - "$first_elapsed" "$second_elapsed" "$WAIT_SECS" >"$SUMMARY" <<'PY'
import json, sys
print(json.dumps({
    "ok": True,
    "directions": [
        "macos-owner-to-physical-android-joiner",
        "physical-android-owner-to-macos-joiner",
    ],
    "manual_entry": "shipped UI only",
    "delivery_measurement_clock": "host_monotonic_ms",
    "start_checkpoint": "host_released_approval_submission",
    "finish_checkpoint": "host_observed_authorization_authenticated_fips_and_durable_ack",
    "delivery_elapsed_ms": {
        "macos_owner_to_android_joiner": int(sys.argv[1]),
        "android_owner_to_macos_joiner": int(sys.argv[2]),
    },
    "delivery_ceiling_ms": int(sys.argv[3]) * 1000,
    "authenticated_fips_both_directions": True,
    "pending_device_approval_receipts_zero": True,
    "post_link_provider_exchange_both_directions": True,
}, indent=2, sort_keys=True))
PY

echo "MACOS_VM_PHYSICAL_ANDROID_BIDIRECTIONAL_MANUAL_LINK_E2E_OK"
echo "approval delivery elapsed ms (macOS->Android, Android->macOS): $first_elapsed, $second_elapsed"
echo "Result: $SUMMARY"
