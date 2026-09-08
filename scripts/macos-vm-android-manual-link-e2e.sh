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
DIRECTION="${IRIS_DRIVE_MACOS_ANDROID_DIRECTION:-both}"
PACKAGE="${IRIS_DRIVE_ANDROID_PHYSICAL_LINK_PACKAGE:-to.iris.drive.uitest}"
ACTIVITY="${IRIS_DRIVE_ANDROID_PHYSICAL_LINK_ACTIVITY:-$PACKAGE/to.iris.drive.app.MainActivity}"
APK="${IRIS_DRIVE_ANDROID_PHYSICAL_LINK_APK:-$ROOT/android/app/build/outputs/apk/uiTest/app-uiTest.apk}"
TEST_APK="${IRIS_DRIVE_ANDROID_PHYSICAL_LINK_TEST_APK:-$ROOT/android/app/build/outputs/apk/androidTest/uiTest/app-uiTest-androidTest.apk}"
TEST_RUNNER="${IRIS_DRIVE_ANDROID_PHYSICAL_LINK_TEST_RUNNER:-$PACKAGE.test/androidx.test.runner.AndroidJUnitRunner}"
TEST_PACKAGE="${IRIS_DRIVE_ANDROID_PHYSICAL_LINK_TEST_PACKAGE:-${TEST_RUNNER%%/*}}"
DEBUG_ACTION_EXTRA="${IRIS_DRIVE_ANDROID_DEBUG_ACTION_EXTRA:-to.iris.drive.DEBUG_ACTION}"
DEBUG_RELAY_EXTRA="${IRIS_DRIVE_ANDROID_DEBUG_RELAY_EXTRA:-to.iris.drive.DEBUG_RELAY}"
RUN_ID="$(date +%s)-$$-$RANDOM"
RESULT_DIR="${IRIS_DRIVE_MACOS_ANDROID_RESULT_DIR:-$ROOT/artifacts/macos-android-manual-link/$RUN_ID}"
SUMMARY="$RESULT_DIR/summary.json"
TMP=""
ADB=""
SERIAL=""
REMOTE_DIRTY=0
LOCAL_RELAY_PID=""
LOCAL_RELAY_TUNNEL_PID=""
LOCAL_RELAY_URL=""
LOCAL_RELAY_PORT=""
LOCAL_RELAY_EVENT_LOG=""
LOCAL_RELAY_EPOCH=0
LOCAL_BLOSSOM_PID=""
LOCAL_BLOSSOM_TUNNEL_PID=""
LOCAL_BLOSSOM_URL=""
LOCAL_BLOSSOM_PORT=""
LOCAL_BLOSSOM_STORAGE=""
LOCAL_BLOSSOM_REQUEST_LOG=""

# shellcheck source=scripts/lib/macos-android-physical-evidence.sh
source "$ROOT/scripts/lib/macos-android-physical-evidence.sh"

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
  local command arg name
  command="cd '$MAC_GUEST_REPO' && env"
  for name in CARGO_BUILD_JOBS CARGO_INCREMENTAL CARGO_PROFILE_DEV_DEBUG CARGO_PROFILE_TEST_DEBUG; do
    if [[ -n "${!name+x}" ]]; then
      command+=" $(printf '%q' "$name=${!name}")"
    fi
  done
  command+=" IRIS_DRIVE_MACOS_ANDROID_RELAY_URL=$(printf '%q' "$LOCAL_RELAY_URL") IRIS_DRIVE_MACOS_ANDROID_BLOSSOM_URL=$(printf '%q' "$LOCAL_BLOSSOM_URL") '$REMOTE_SCRIPT'"
  shift 0
  for arg in "$@"; do
    command+=" $(printf '%q' "$arg")"
  done
  ssh -o BatchMode=yes "$MAC_HOST" "$command"
}

write_blossom_summary() {
  [[ -d "$LOCAL_BLOSSOM_STORAGE" ]] || return 0
  python3 - "$LOCAL_BLOSSOM_STORAGE" "$LOCAL_BLOSSOM_REQUEST_LOG" \
    >"$RESULT_DIR/blossom-summary.json" <<'PY'
import collections, json, pathlib, sys
storage=pathlib.Path(sys.argv[1])
requests=[]
request_log=pathlib.Path(sys.argv[2])
if request_log.is_file():
    requests=[json.loads(line) for line in request_log.read_text().splitlines() if line]
print(json.dumps({
    "blob_count": len(list(storage.glob("*.bin"))),
    "request_count": len(requests),
    "methods": dict(sorted(collections.Counter(item["method"] for item in requests).items())),
    "statuses": dict(sorted(collections.Counter(str(item["status"]) for item in requests).items())),
    "requests": [{
        "sequence": int(item.get("sequence") or 0),
        "method": item.get("method"),
        "status": item.get("status"),
    } for item in requests],
    "transport": "SHA-validating loopback fixture via adb reverse and SSH remote forward",
}, indent=2, sort_keys=True))
PY
}

stop_deterministic_relay() {
  if [[ -n "${LOCAL_RELAY_TUNNEL_PID:-}" ]]; then
    kill "$LOCAL_RELAY_TUNNEL_PID" >/dev/null 2>&1 || true
    wait "$LOCAL_RELAY_TUNNEL_PID" >/dev/null 2>&1 || true
    LOCAL_RELAY_TUNNEL_PID=""
  fi
  if [[ -n "${ADB:-}" && -n "${SERIAL:-}" && -n "${LOCAL_RELAY_PORT:-}" ]]; then
    "$ADB" -s "$SERIAL" reverse --remove "tcp:$LOCAL_RELAY_PORT" >/dev/null 2>&1 || true
  fi
  if [[ -n "${LOCAL_RELAY_PID:-}" ]]; then
    kill "$LOCAL_RELAY_PID" >/dev/null 2>&1 || true
    wait "$LOCAL_RELAY_PID" >/dev/null 2>&1 || true
    LOCAL_RELAY_PID=""
  fi
  LOCAL_RELAY_URL=""
  LOCAL_RELAY_PORT=""
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

capture_remote_root_sync_evidence() {
  local role="$1"
  local evidence
  [[ -n "$TMP" ]] || return 0
  evidence="$TMP/macos-$role-root-sync-evidence.json"
  [[ -f "$RESULT_DIR/macos-$role-root-sync-evidence.json" ]] && return
  remote root-sync-evidence "$role" >"$evidence" 2>/dev/null || return
  cp "$evidence" "$RESULT_DIR/macos-$role-root-sync-evidence.json"
}

cleanup() {
  local status=$?
  trap - EXIT
  set +e
  if [[ "$REMOTE_DIRTY" == 1 ]]; then
    capture_remote_evidence owner || true
    capture_remote_evidence joiner || true
    if ((status != 0)); then
      capture_remote_root_sync_evidence owner || true
      capture_remote_root_sync_evidence joiner || true
    fi
    remote cleanup >/dev/null 2>&1 || true
  fi
  if declare -F write_relay_summary >/dev/null; then
    write_relay_summary || true
  fi
  if declare -F write_blossom_summary >/dev/null; then
    write_blossom_summary || true
  fi
  if declare -F stop_deterministic_relay >/dev/null; then
    stop_deterministic_relay || true
  fi
  if [[ -n "${LOCAL_BLOSSOM_TUNNEL_PID:-}" ]]; then
    kill "$LOCAL_BLOSSOM_TUNNEL_PID" >/dev/null 2>&1 || true
    wait "$LOCAL_BLOSSOM_TUNNEL_PID" >/dev/null 2>&1 || true
  fi
  if [[ -n "${LOCAL_BLOSSOM_PID:-}" ]]; then
    kill "$LOCAL_BLOSSOM_PID" >/dev/null 2>&1 || true
    wait "$LOCAL_BLOSSOM_PID" >/dev/null 2>&1 || true
  fi
  if [[ -n "$ADB" && -n "$SERIAL" ]]; then
    write_android_action_evidence >/dev/null 2>&1 || true
    [[ -z "${LOCAL_RELAY_PORT:-}" ]] \
      || "$ADB" -s "$SERIAL" reverse --remove "tcp:$LOCAL_RELAY_PORT" >/dev/null 2>&1 || true
    [[ -z "${LOCAL_BLOSSOM_PORT:-}" ]] \
      || "$ADB" -s "$SERIAL" reverse --remove "tcp:$LOCAL_BLOSSOM_PORT" >/dev/null 2>&1 || true
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
[[ "$DIRECTION" == both || "$DIRECTION" == reverse ]] \
  || fail "IRIS_DRIVE_MACOS_ANDROID_DIRECTION must be both or reverse"
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

start_deterministic_relay() {
  local ready log
  LOCAL_RELAY_EPOCH=$((LOCAL_RELAY_EPOCH + 1))
  ready="$TMP/local-relay-$LOCAL_RELAY_EPOCH.ready"
  log="$TMP/local-relay-$LOCAL_RELAY_EPOCH.log"
  LOCAL_RELAY_EVENT_LOG="$TMP/local-relay-events-$LOCAL_RELAY_EPOCH.jsonl"
  python3 "$ROOT/scripts/local-nostr-relay.py" --ready-file "$ready" \
    --event-log "$LOCAL_RELAY_EVENT_LOG" >"$log" 2>&1 &
  LOCAL_RELAY_PID=$!
  for _ in $(seq 1 100); do
    [[ -s "$ready" ]] && break
    kill -0 "$LOCAL_RELAY_PID" >/dev/null 2>&1 || {
      cat "$log" >&2
      fail "local Nostr relay exited before readiness"
    }
    sleep 0.1
  done
  [[ -s "$ready" ]] || fail "local Nostr relay did not become ready"
  LOCAL_RELAY_URL="$(cat "$ready")"
  LOCAL_RELAY_PORT="$(python3 -c 'import sys,urllib.parse; print(urllib.parse.urlsplit(sys.argv[1]).port or "")' "$LOCAL_RELAY_URL")"
  [[ "$LOCAL_RELAY_URL" == "ws://127.0.0.1:$LOCAL_RELAY_PORT" \
    && "$LOCAL_RELAY_PORT" =~ ^[1-9][0-9]*$ ]] \
    || fail "local Nostr relay returned an invalid loopback URL"

  "$ADB" -s "$SERIAL" reverse "tcp:$LOCAL_RELAY_PORT" "tcp:$LOCAL_RELAY_PORT" >/dev/null
  ssh -o BatchMode=yes -o ExitOnForwardFailure=yes -N \
    -R "127.0.0.1:$LOCAL_RELAY_PORT:127.0.0.1:$LOCAL_RELAY_PORT" "$MAC_HOST" &
  LOCAL_RELAY_TUNNEL_PID=$!
  for _ in $(seq 1 50); do
    if ssh -o BatchMode=yes "$MAC_HOST" \
      "python3 -c 'import socket; s=socket.create_connection((\"127.0.0.1\", $LOCAL_RELAY_PORT), 1); s.close()'" \
      >/dev/null 2>&1; then
      return 0
    fi
    kill -0 "$LOCAL_RELAY_TUNNEL_PID" >/dev/null 2>&1 \
      || fail "macOS relay forward exited before readiness"
    sleep 0.1
  done
  fail "macOS VM could not reach the deterministic relay forward"
}

restart_deterministic_relay() {
  stop_deterministic_relay
  start_deterministic_relay
}

start_deterministic_relay

start_deterministic_blossom() {
  local ready="$TMP/local-blossom.ready" log="$TMP/local-blossom.log"
  LOCAL_BLOSSOM_STORAGE="$TMP/local-blossom-blobs"
  LOCAL_BLOSSOM_REQUEST_LOG="$TMP/local-blossom-requests.jsonl"
  mkdir -p "$LOCAL_BLOSSOM_STORAGE"
  python3 "$ROOT/scripts/local-blossom-server.py" --host 127.0.0.1 --port 0 \
    --storage-dir "$LOCAL_BLOSSOM_STORAGE" --ready-file "$ready" \
    --request-log "$LOCAL_BLOSSOM_REQUEST_LOG" >"$log" 2>&1 &
  LOCAL_BLOSSOM_PID=$!
  for _ in $(seq 1 100); do
    [[ -s "$ready" ]] && break
    kill -0 "$LOCAL_BLOSSOM_PID" >/dev/null 2>&1 || {
      cat "$log" >&2
      fail "local Blossom fixture exited before readiness"
    }
    sleep 0.1
  done
  [[ -s "$ready" ]] || fail "local Blossom fixture did not become ready"
  LOCAL_BLOSSOM_URL="$(cat "$ready")"
  LOCAL_BLOSSOM_PORT="$(python3 -c 'import sys,urllib.parse; print(urllib.parse.urlsplit(sys.argv[1]).port or "")' "$LOCAL_BLOSSOM_URL")"
  "$ADB" -s "$SERIAL" reverse "tcp:$LOCAL_BLOSSOM_PORT" "tcp:$LOCAL_BLOSSOM_PORT" >/dev/null
  ssh -o BatchMode=yes -o ExitOnForwardFailure=yes -N \
    -R "127.0.0.1:$LOCAL_BLOSSOM_PORT:127.0.0.1:$LOCAL_BLOSSOM_PORT" "$MAC_HOST" &
  LOCAL_BLOSSOM_TUNNEL_PID=$!
  for _ in $(seq 1 50); do
    ssh -o BatchMode=yes "$MAC_HOST" \
      "curl -fsS '$LOCAL_BLOSSOM_URL/health' >/dev/null" >/dev/null 2>&1 && return 0
    kill -0 "$LOCAL_BLOSSOM_TUNNEL_PID" >/dev/null 2>&1 \
      || fail "macOS Blossom forward exited before readiness"
    sleep 0.1
  done
  fail "macOS VM could not reach the deterministic Blossom forward"
}

start_deterministic_blossom

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

android_relay_action() {
  "$ADB" -s "$SERIAL" shell am start -W -n "$ACTIVITY" \
    --es "$DEBUG_ACTION_EXTRA" "$1" \
    --es "$DEBUG_RELAY_EXTRA" "$2" >/dev/null
}

read_android_config_relays() {
  copy_android_file config.toml "$TMP/android-config.toml" || return 1
  python3 - "$TMP/android-config.toml" <<'PY'
import ast, json, re, sys
text=open(sys.argv[1], encoding="utf-8").read()
match=re.search(r"(?ms)^relays\s*=\s*(\[.*?\])", text)
if match is None: raise SystemExit(1)
relays=ast.literal_eval(match.group(1))
if not isinstance(relays, list) or not all(isinstance(relay, str) for relay in relays):
    raise SystemExit(1)
print(json.dumps(relays))
PY
}

android_relays_are_singleton() {
  read_android_config_relays >"$TMP/android-relays.json" || return 1
  python3 - "$TMP/android-relays.json" "$LOCAL_RELAY_URL" <<'PY'
import json, sys
relays=json.load(open(sys.argv[1], encoding="utf-8"))
raise SystemExit(0 if relays == [sys.argv[2]] else 1)
PY
}

android_blossom_is_singleton() {
  copy_android_file config.toml "$TMP/android-config.toml" || return 1
  python3 - "$TMP/android-config.toml" "$LOCAL_BLOSSOM_URL" <<'PY'
import ast, re, sys
text=open(sys.argv[1], encoding="utf-8").read()
match=re.search(r"(?ms)^blossom_servers\s*=\s*(\[.*?\])", text)
if match is None: raise SystemExit(1)
raise SystemExit(0 if ast.literal_eval(match.group(1)) == [sys.argv[2]] else 1)
PY
}

write_android_relay_evidence() {
  read_android_config_relays >"$TMP/android-relays.json" || return 0
  python3 - "$TMP/android-relays.json" >"$RESULT_DIR/android-relay-evidence.json" <<'PY'
import json, sys
relays=json.load(open(sys.argv[1], encoding="utf-8"))
print(json.dumps({"configured_relays": relays}, indent=2, sort_keys=True))
PY
}

write_android_action_evidence() {
  local log="$TMP/android-state-transitions.log"
  "$ADB" -s "$SERIAL" logcat -d -v epoch IrisDriveState:I '*:S' >"$log"
  python3 - "$DIRECTION" "$log" >"$RESULT_DIR/android-action-evidence.json" <<'PY'
import json, re, sys
entries=[]
pattern=re.compile(r"phase=(\S+) generation=(\d+) origin=(\S+).* error=(\S+)")
for line in open(sys.argv[2], encoding="utf-8"):
    match=pattern.search(line)
    if match:
        entries.append({
            "phase": match.group(1),
            "generation": int(match.group(2)),
            "origin": match.group(3),
            "error_category": match.group(4),
        })
print(json.dumps({"direction": sys.argv[1], "state_transitions": entries}, indent=2, sort_keys=True))
PY
}

configure_android_singleton_relay() {
  local deadline=$((SECONDS + 10))
  android_relay_action replace-relays "$LOCAL_RELAY_URL"
  while ((SECONDS < deadline)); do
    android_relays_are_singleton && return 0
    sleep 0.1
  done
  write_android_relay_evidence
  fail "Android did not atomically persist the singleton deterministic relay"
}

configure_android_singleton_blossom() {
  local deadline=$((SECONDS + 10))
  android_relay_action replace-blossom "$LOCAL_BLOSSOM_URL"
  while ((SECONDS < deadline)); do
    android_blossom_is_singleton && return 0
    sleep 0.1
  done
  fail "Android did not atomically persist the singleton deterministic Blossom endpoint"
}

wait_android_process_stopped() {
  local deadline=$((SECONDS + 5))
  while ((SECONDS < deadline)); do
    [[ -z "$("$ADB" -s "$SERIAL" shell pidof "$PACKAGE" 2>/dev/null | tr -d '\r')" ]] && return 0
    sleep 0.1
  done
  return 1
}

prepare_android_actor() {
  "$ADB" -s "$SERIAL" shell am force-stop "$PACKAGE" >/dev/null 2>&1 || true
  "$ADB" -s "$SERIAL" shell pm clear "$PACKAGE" >/dev/null
  # This unconfigured bootstrap cannot start a profile/FIPS actor. Persist the
  # complete transport set, then stop it so the real actor's first native-core
  # process advertises and subscribes only against the current relay epoch.
  start_android_app
  configure_android_singleton_relay
  configure_android_singleton_blossom
  "$ADB" -s "$SERIAL" shell am force-stop "$PACKAGE" >/dev/null
  wait_android_process_stopped \
    || fail "Android config bootstrap remained alive before actor launch"
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
  prepare_android_actor
  start_android_app
  tap_android_until_ui "Create profile" text "Username (optional)" 15 \
    || fail "Android shipped profile form did not open"
  tap_android_until_ui "Create profile" text "My Drive" 20 \
    || fail "Android owner did not reach My Drive"
  android_debug_action start-sync
}

create_android_join_request() {
  prepare_android_actor
  start_android_app
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
  android_type_text_in_chunks "$1"
  wait_android_ui text "Approve" 10
}

android_type_text_in_chunks() {
  local remaining="$1" chunk
  while [[ -n "$remaining" ]]; do
    chunk="${remaining:0:32}"
    remaining="${remaining:32}"
    "$ADB" -s "$SERIAL" shell input text "$chunk" >/dev/null
    # uiautomator waits for the prior input events and Compose recomposition;
    # this prevents the next chunk from racing the mutable text-field state.
    android_dump_ui || return 1
  done
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
direct = peer in (fips.get("direct_devices") or []) or peer in (fips.get("direct_peers") or [])
mesh = peer in (fips.get("mesh_devices") or []) or peer in (fips.get("mesh_peers") or [])
owner_ok=sys.argv[4] != "1" or not receipts
raise SystemExit(0 if authorized and owner_ok and authorized_peer and online and (direct or mesh) else 1)
PY
}

assert_relay_epoch_adverts() {
  python3 - "$LOCAL_RELAY_EVENT_LOG" "$1" "$2" <<'PY'
import json, pathlib, sys
charset="qpzry9x8gf2tvdw0s3jn54khce6mua7l"
def npub_hex(value):
    data=[charset.index(c) for c in value[value.rfind("1")+1:-6]]
    acc=bits=0; out=[]
    for item in data:
        acc=(acc << 5) | item; bits += 5
        while bits >= 8:
            bits -= 8; out.append((acc >> bits) & 255)
    if len(out) != 32: raise SystemExit(1)
    return bytes(out).hex()
events=[json.loads(line) for line in pathlib.Path(sys.argv[1]).read_text().splitlines() if line]
authors={event.get("pubkey") for event in events if event.get("kind")==37195}
expected={npub_hex(value) for value in sys.argv[2:]}
raise SystemExit(0 if len(authors) >= 2 and expected.issubset(authors) else 1)
PY
}

wait_actor_adverts() {
  local android_peer="$1" mac_peer="$2" deadline=$((SECONDS + 30))
  while ((SECONDS < deadline)); do
    if assert_relay_epoch_adverts "$android_peer" "$mac_peer"; then
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
    if ((now > deadline)); then
      write_android_evidence "link-timeout" "$android_peer" \
        "$RESULT_DIR/android-link-timeout-evidence.json" || true
      remote peer-evidence "$mac_role" "$mac_peer" \
        >"$RESULT_DIR/macos-link-timeout-evidence.json" 2>/dev/null || true
      fail "$label exceeded the strict $((WAIT_SECS * 1000))ms ceiling"
    fi
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
  local method="$1" name="$2" content="$3" instrumentation_pid=""
  local log="$TMP/android-$method.log"
  if [[ "$method" == readPostLinkFileThroughDocumentsProvider ]]; then
    # `am instrument` force-stops the target package. Start the provider poll
    # first, then relaunch the shipped app so its real relay/FIPS runtime stays
    # alive while the same unchanged 45-second read deadline is measured.
    "$ADB" -s "$SERIAL" shell am instrument -w -r \
      -e class "to.iris.drive.app.provider.IrisDrivePhysicalLinkingProviderTest#$method" \
      -e file_name "$name" \
      -e content_b64 "$(printf '%s' "$content" | base64_url)" \
      -e wait_millis "$((POST_LINK_WAIT_SECS * 1000))" \
      "$TEST_RUNNER" >"$log" 2>&1 &
    instrumentation_pid=$!
    for _ in $(seq 1 50); do
      grep -Fq "INSTRUMENTATION_STATUS: test=$method" "$log" && break
      kill -0 "$instrumentation_pid" >/dev/null 2>&1 || break
      sleep 0.1
    done
    grep -Fq "INSTRUMENTATION_STATUS: test=$method" "$log" || {
      wait "$instrumentation_pid" >/dev/null 2>&1 || true
      cp "$log" "$RESULT_DIR/android-$method.log"
      return 1
    }
    start_android_app
    android_debug_action start-sync
    wait "$instrumentation_pid" || {
      cp "$log" "$RESULT_DIR/android-$method.log"
      return 1
    }
  elif ! "$ADB" -s "$SERIAL" shell am instrument -w -r \
    -e class "to.iris.drive.app.provider.IrisDrivePhysicalLinkingProviderTest#$method" \
    -e file_name "$name" \
    -e content_b64 "$(printf '%s' "$content" | base64_url)" \
    -e wait_millis "$((POST_LINK_WAIT_SECS * 1000))" \
    "$TEST_RUNNER" >"$log" 2>&1; then
    {
      cp "$log" "$RESULT_DIR/android-$method.log"
      return 1
    }
  fi
  cp "$log" "$RESULT_DIR/android-$method.log"
  grep -Eq 'OK \(1 test\)|IRIS_ANDROID_PROVIDER_(WRITE|READ)_SHA256=' "$log"
}

transport_phase_counts() {
  python3 - "$LOCAL_RELAY_EVENT_LOG" "$LOCAL_BLOSSOM_STORAGE" \
    "$LOCAL_BLOSSOM_REQUEST_LOG" <<'PY'
import json, pathlib, sys
relay=pathlib.Path(sys.argv[1])
events=[json.loads(line) for line in relay.read_text().splitlines() if line] if relay.is_file() else []
storage=pathlib.Path(sys.argv[2])
requests=pathlib.Path(sys.argv[3])
request_rows=[json.loads(line) for line in requests.read_text().splitlines() if line] if requests.is_file() else []
print(" ".join(str(value) for value in (
    sum(event.get("kind") == 30078 for event in events),
    len(list(storage.glob("*.bin"))) if storage.is_dir() else 0,
    sum(item.get("method") == "PUT" for item in request_rows),
    sum(item.get("method") == "GET" for item in request_rows),
)))
PY
}

write_provider_publish_phase() {
  local before="$1" after="$2" output="$3"
  python3 - "$before" "$after" >"$output" <<'PY'
import json, sys
keys=("drive_root_events", "persisted_blobs", "put_requests", "get_requests")
before=dict(zip(keys, map(int, sys.argv[1].split())))
after=dict(zip(keys, map(int, sys.argv[2].split())))
print(json.dumps({
    "before": before,
    "after": after,
    "delta": {key: after[key] - before[key] for key in keys},
}, indent=2, sort_keys=True))
if after["drive_root_events"] <= before["drive_root_events"]:
    raise SystemExit("Android provider mutation did not publish a Drive-root event")
if after["put_requests"] <= before["put_requests"]:
    raise SystemExit("Android provider mutation did not attempt a singleton Blossom upload")
PY
}

stop_first_direction_actors() {
  remote stop owner
  remote stop joiner
  "$ADB" -s "$SERIAL" shell am force-stop "$TEST_PACKAGE" >/dev/null 2>&1 || true
  "$ADB" -s "$SERIAL" shell am force-stop "$PACKAGE" >/dev/null 2>&1 || true
  wait_android_process_stopped \
    || fail "first-direction Android actor remained alive across relay epochs"
}

# macOS owner -> physical Android joiner.
first_elapsed=-1
if [[ "$DIRECTION" == both ]]; then
  remote start-owner
  create_android_join_request
  ANDROID_JOINER_NPUB="$(android_profile_value current_app_key_npub)" \
    || fail "Android joining identity was unavailable"
  ANDROID_REQUEST="$(android_profile_value app_key_link_request)" \
    || fail "Android manual approval request URL was unavailable"
  [[ "$ANDROID_REQUEST" == https://drive.iris.to/approve-device/* ]] \
    || fail "Android request URL is not canonical"
  wait_actor_adverts "$ANDROID_JOINER_NPUB" "$MAC_OWNER_NPUB" \
    || {
      write_android_evidence joiner "$MAC_OWNER_NPUB" \
        "$RESULT_DIR/android-joiner-readiness-evidence.json" || true
      fail "macOS owner and Android joiner did not advertise on the deterministic relay"
    }
  remote manual-prepare "$ANDROID_REQUEST"
  first_started="$(monotonic_milliseconds)"
  remote manual-submit
  first_elapsed="$(wait_link_before \
    "macOS owner -> physical Android joiner" "$first_started" owner \
    "$MAC_OWNER_NPUB" "$ANDROID_JOINER_NPUB" 0)"
  wait_android_ui text "My Drive" 5 || fail "Android joined UI did not reach My Drive"
  first_file="macos-owner-android-joiner-$RUN_ID.txt"
  first_content="physical Android joiner provider write $RUN_ID"
  first_transport_before="$(transport_phase_counts)"
  run_android_provider_test writePostLinkFileThroughDocumentsProvider "$first_file" "$first_content" \
    || fail "Android provider could not write after macOS approval"
  start_android_app
  android_debug_action start-sync
  first_transport_after="$(transport_phase_counts)"
  write_provider_publish_phase "$first_transport_before" "$first_transport_after" \
    "$RESULT_DIR/android-provider-publish-phase.json" \
    || fail "Android provider write did not complete its source publish phases"
  remote wait-provider-read owner "$first_file" "$(printf '%s' "$first_content" | base64_url)"
  capture_remote_evidence owner
  write_android_evidence joiner "$MAC_OWNER_NPUB" "$RESULT_DIR/android-joiner-evidence.json"
fi

# Physical Android owner -> macOS joiner.
if [[ "$DIRECTION" == both ]]; then
  # The directions use independent profiles. A fresh relay epoch prevents
  # stopped first-direction identities from satisfying or perturbing the
  # reverse direction's direct-FIPS readiness through stale discovery events.
  stop_first_direction_actors
  restart_deterministic_relay
fi
create_android_owner
ANDROID_OWNER_NPUB="$(android_profile_value current_app_key_npub)" \
  || fail "Android owner identity was unavailable"
MAC_REQUEST_OUTPUT="$(remote start-joiner)"
MAC_REQUEST="$(printf '%s\n' "$MAC_REQUEST_OUTPUT" | tail -n 1)"
[[ "$MAC_REQUEST" == https://drive.iris.to/approve-device/* ]] \
  || fail "macOS shipped Sign in UI did not return a canonical request"
MAC_JOINER_NPUB="$(remote status joiner | python3 -c 'import json,sys; print((json.load(sys.stdin).get("profile") or {}).get("current_app_key_npub", ""))')"
[[ "$MAC_JOINER_NPUB" == npub1* ]] || fail "macOS joining identity is invalid"
wait_actor_adverts "$ANDROID_OWNER_NPUB" "$MAC_JOINER_NPUB" \
  || {
    write_android_evidence owner "$MAC_JOINER_NPUB" \
      "$RESULT_DIR/android-owner-readiness-evidence.json" || true
    fail "Android owner and macOS joiner did not advertise in relay epoch 2"
  }
assert_relay_epoch_adverts "$ANDROID_OWNER_NPUB" "$MAC_JOINER_NPUB" \
  || fail "reverse actors did not both publish FIPS adverts in relay epoch 2"
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
if ! run_android_provider_test readPostLinkFileThroughDocumentsProvider \
  "$second_file" "$second_content"; then
  write_android_root_sync_evidence "$second_file" "${#second_content}" \
    "$RESULT_DIR/android-owner-root-sync-evidence.json" || true
  write_android_evidence owner "$MAC_JOINER_NPUB" "$RESULT_DIR/android-owner-evidence.json" || true
  fail "Android provider could not read the macOS post-link write"
fi
capture_remote_evidence joiner
write_android_evidence owner "$MAC_JOINER_NPUB" "$RESULT_DIR/android-owner-evidence.json"

remote cleanup
REMOTE_DIRTY=0
python3 - "$first_elapsed" "$second_elapsed" "$WAIT_SECS" "$DIRECTION" >"$SUMMARY" <<'PY'
import json, sys
directions=["physical-android-owner-to-macos-joiner"]
if sys.argv[4] == "both": directions.insert(0, "macos-owner-to-physical-android-joiner")
print(json.dumps({
    "ok": True,
    "directions": directions,
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
