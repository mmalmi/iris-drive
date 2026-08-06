#!/usr/bin/env bash
# macOS VM half of the physical macOS <-> Android manual-entry link matrix.
set -Eeuo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
ARTIFACT_DIR="$ROOT/artifacts/macos-android-manual-link"
PRIVATE_DIR="$ARTIFACT_DIR/.private"
OWNER_BASE="$PRIVATE_DIR/owner"
JOINER_BASE="$PRIVATE_DIR/joiner"
APP_PATH="${IRIS_DRIVE_MACOS_ANDROID_APP_PATH:-$ROOT/macos/.build/Applications/Iris Drive.app}"
WAIT_SECS="${IRIS_DRIVE_MACOS_ANDROID_WAIT_SECS:-15}"

[[ "$(uname -s)" == Darwin ]] || {
  echo "macOS/Android remote link driver requires macOS" >&2
  exit 2
}
if [[ ! "$WAIT_SECS" =~ ^[1-9][0-9]*$ ]] || ((WAIT_SECS > 15)); then
  echo "IRIS_DRIVE_MACOS_ANDROID_WAIT_SECS must be 1..15" >&2
  exit 2
fi

base_for_role() {
  case "$1" in
    owner) printf '%s\n' "$OWNER_BASE" ;;
    joiner) printf '%s\n' "$JOINER_BASE" ;;
    *) echo "role must be owner or joiner" >&2; return 2 ;;
  esac
}

pid_file() { printf '%s/%s.%s.pid\n' "$ARTIFACT_DIR" "$1" "$2"; }

stop_pid_file() {
  local file="$1" pid=""
  [[ -f "$file" ]] || return 0
  pid="$(cat "$file" 2>/dev/null || true)"
  if [[ "$pid" =~ ^[0-9]+$ ]]; then
    kill "$pid" >/dev/null 2>&1 || true
    wait "$pid" >/dev/null 2>&1 || true
  fi
  rm -f "$file"
}

stop_role() {
  stop_pid_file "$(pid_file "$1" app)"
  stop_pid_file "$(pid_file "$1" daemon)"
}

load_candidate() {
  if [[ ! -d "$APP_PATH" ]]; then
    APP_PATH="$(IRIS_DRIVE_MACOS_SIGNING="${IRIS_DRIVE_MACOS_SIGNING:-none}" \
      "$ROOT/scripts/macos-dev-app.sh" build)"
  fi
  IDRIVE="$APP_PATH/Contents/MacOS/idrive"
  APP_EXE="$APP_PATH/Contents/MacOS/Iris Drive"
  [[ -x "$IDRIVE" && -x "$APP_EXE" ]] || {
    echo "macOS candidate is incomplete: $APP_PATH" >&2
    return 1
  }
}

run_ax_action() {
  local label="$1" status
  shift
  set +e
  /usr/bin/swift "$ROOT/scripts/macos-device-link-ax.swift" "$@"
  status=$?
  set -e
  if ((status != 0)); then
    echo "$label failed" >&2
    return "$status"
  fi
}

status_for_role() {
  local base
  base="$(base_for_role "$1")"
  "$IDRIVE" --config-dir "$base/Config" status 2>/dev/null
}

sanitized_status_for_role() {
  local role="$1"
  status_for_role "$role" | python3 -c '
import json, sys
s=json.load(sys.stdin); profile=s.get("profile") or {}; summary=s.get("summary") or {}
fips=((s.get("network") or {}).get("fips") or {})
peers=[peer for peer in s.get("peers", []) if peer.get("connection_state") != "local"]
print(json.dumps({
    "role": sys.argv[1],
    "summary": {key: summary.get(key) for key in (
        "setup_state", "setup_complete", "awaiting_approval", "revoked",
        "authorized_app_key_count", "online_app_key_count", "file_count",
    )},
    "profile": {
        "authorization_state": profile.get("authorization_state"),
        "can_admin_profile": profile.get("can_admin_profile"),
        "can_write_roots": profile.get("can_write_roots"),
        "pending_device_approval_receipt_count": profile.get("pending_device_approval_receipt_count"),
        "roster_size": profile.get("roster_size"),
    },
    "fips": {key: fips.get(key) for key in (
        "running", "state", "connected_peer_count", "authorized_peer_count",
        "roster_online_peer_count", "roster_direct_device_count",
    )},
    "remote_peers": {
        "authorized_online_count": sum(peer.get("authorized") is True and peer.get("fips_online") is True for peer in peers),
        "authorized_direct_count": sum(peer.get("authorized") is True and peer.get("fips_direct_online") is True for peer in peers),
        "authorized_mesh_count": sum(peer.get("authorized") is True and peer.get("fips_mesh_online") is True for peer in peers),
    },
}, sort_keys=True))
' "$role"
}

start_daemon() {
  local role="$1" base log file pid
  base="$(base_for_role "$role")"
  log="$ARTIFACT_DIR/$role-daemon"
  file="$(pid_file "$role" daemon)"
  stop_pid_file "$file"
  env IRIS_DRIVE_DEBUG_LOG_DIR="$ARTIFACT_DIR/$role-debug" \
    "$IDRIVE" --config-dir "$base/Config" daemon --watch-interval 0 --no-gateway \
    >"$log.stdout.log" 2>"$log.stderr.log" &
  pid=$!
  printf '%s\n' "$pid" >"$file"
  sleep 0.3
  kill -0 "$pid" >/dev/null 2>&1 || {
    echo "macOS $role daemon did not start" >&2
    return 1
  }
}

start_app() {
  local role="$1" base file deadline pid=""
  base="$(base_for_role "$role")"
  file="$(pid_file "$role" app)"
  stop_pid_file "$file"
  open -n \
    --stdout "$ARTIFACT_DIR/$role-app.stdout.log" \
    --stderr "$ARTIFACT_DIR/$role-app.stderr.log" \
    --env "IRIS_DRIVE_APP_BASE_DIR=$base" \
    --env "IRIS_DRIVE_DEBUG_LOG_DIR=$ARTIFACT_DIR/$role-debug" \
    --env "IRIS_DRIVE_DISABLE_LOGIN_AGENT_SYNC=true" \
    --env "IRIS_DRIVE_DISABLE_SINGLE_INSTANCE=true" \
    --env "IRIS_DRIVE_DISABLE_DAEMON_SERVICE=true" \
    --env "IRIS_DRIVE_EXTERNAL_DAEMON=true" \
    "$APP_PATH"
  deadline=$((SECONDS + 10))
  while ((SECONDS < deadline)); do
    pid="$(pgrep -x 'Iris Drive' | tail -n 1 || true)"
    [[ "$pid" =~ ^[0-9]+$ ]] && break
    sleep 0.1
  done
  [[ "$pid" =~ ^[0-9]+$ ]] || {
    echo "macOS $role UI did not launch" >&2
    return 1
  }
  printf '%s\n' "$pid" >"$file"
  /usr/bin/swift - >/dev/null <<'SWIFT'
import Foundation
DistributedNotificationCenter.default().postNotificationName(
    Notification.Name("to.iris.drive.showControlPanel"),
    object: nil,
    userInfo: nil,
    deliverImmediately: true
)
RunLoop.current.run(until: Date().addingTimeInterval(0.2))
SWIFT
}

app_pid() {
  local pid
  pid="$(cat "$(pid_file "$1" app)" 2>/dev/null || true)"
  [[ "$pid" =~ ^[0-9]+$ ]] && kill -0 "$pid" >/dev/null 2>&1 || return 1
  printf '%s\n' "$pid"
}

request_url() {
  status_for_role joiner | python3 -c '
import json, sys
url = ((json.load(sys.stdin).get("profile") or {}).get("app_key_link_request") or {}).get("url", "")
if not url.startswith("https://drive.iris.to/approve-device/"):
    raise SystemExit(1)
print(url)
'
}

status_has_peer_state() {
  local role="$1" peer="$2" require_authorized="$3"
  status_for_role "$role" | python3 -c '
import json, sys
s=json.load(sys.stdin); p=s.get("profile") or {}; peer=sys.argv[1]
authorized=(not bool(int(sys.argv[2])) or (p.get("authorization_state")=="authorized" and p.get("pending_device_approval_receipt_count")==0))
authenticated_fips=any(x.get("app_key_npub")==peer and x.get("fips_online") is True for x in s.get("peers", []))
raise SystemExit(0 if authorized and authenticated_fips else 1)
' "$peer" "$require_authorized"
}

status_fips_ready() {
  status_for_role "$1" | python3 -c '
import json, sys
f=((json.load(sys.stdin).get("network") or {}).get("fips") or {})
raise SystemExit(0 if f.get("running") is True and int(f.get("connected_peer_count") or 0) > 0 else 1)
'
}

case "${1:-}" in
  prepare)
    APP_PATH="$(IRIS_DRIVE_MACOS_SIGNING="${IRIS_DRIVE_MACOS_SIGNING:-none}" \
      "$ROOT/scripts/macos-dev-app.sh" build)"
    load_candidate
    stop_role owner
    stop_role joiner
    rm -rf "$ARTIFACT_DIR"
    mkdir -p "$OWNER_BASE/Config" "$JOINER_BASE" "$ARTIFACT_DIR"
    chmod 700 "$ARTIFACT_DIR" "$PRIVATE_DIR"
    "$IDRIVE" --config-dir "$OWNER_BASE/Config" init --force \
      --label "macOS physical Android owner" >/dev/null
    status_for_role owner | python3 -c '
import json, sys
p=json.load(sys.stdin).get("profile") or {}
print(json.dumps({"owner_app_key_npub": p.get("current_app_key_npub", "")}, sort_keys=True))
'
    ;;
  start-owner)
    load_candidate
    start_daemon owner
    start_app owner
    echo "MACOS_ANDROID_OWNER_READY_OK"
    ;;
  start-joiner)
    load_candidate
    rm -rf "$JOINER_BASE"
    mkdir -p "$JOINER_BASE"
    start_app joiner
    run_ax_action "macOS shipped Sign in UI action" \
      "$(app_pid joiner)" SignIn "$WAIT_SECS"
    deadline=$((SECONDS + WAIT_SECS))
    while ((SECONDS < deadline)); do
      if url="$(request_url 2>/dev/null)"; then
        start_daemon joiner
        printf '%s\n' "$url"
        exit 0
      fi
      sleep 0.1
    done
    echo "macOS shipped Sign in UI did not create a request URL" >&2
    exit 1
    ;;
  manual-prepare)
    [[ $# == 2 ]] || { echo "usage: $0 manual-prepare <request-url>" >&2; exit 2; }
    load_candidate
    run_ax_action "macOS shipped manual approval preparation" \
      "$(app_pid owner)" ManualPrepare "$WAIT_SECS" "$2"
    ;;
  manual-submit)
    [[ $# == 1 ]] || { echo "usage: $0 manual-submit" >&2; exit 2; }
    load_candidate
    run_ax_action "macOS shipped approval submission" \
      "$(app_pid owner)" Approve "$WAIT_SECS"
    ;;
  fips-ready)
    [[ $# == 2 ]] || { echo "usage: $0 fips-ready <owner|joiner>" >&2; exit 2; }
    load_candidate
    status_fips_ready "$2"
    ;;
  link-ready)
    [[ $# == 3 ]] || { echo "usage: $0 link-ready <owner|joiner> <peer-npub>" >&2; exit 2; }
    load_candidate
    status_has_peer_state "$2" "$3" 1
    ;;
  assert-joined-ui)
    load_candidate
    run_ax_action "macOS shipped joined-UI assertion" \
      "$(app_pid joiner)" AssertJoined "$WAIT_SECS"
    ;;
  status)
    [[ $# == 2 ]] || { echo "usage: $0 status <owner|joiner>" >&2; exit 2; }
    load_candidate
    status_for_role "$2"
    ;;
  evidence)
    [[ $# == 2 ]] || { echo "usage: $0 evidence <owner|joiner>" >&2; exit 2; }
    load_candidate
    sanitized_status_for_role "$2"
    ;;
  provider-write)
    [[ $# == 4 ]] || { echo "usage: $0 provider-write <role> <path> <content-b64>" >&2; exit 2; }
    load_candidate
    source_file="$ARTIFACT_DIR/$2-provider-source"
    python3 - "$4" "$source_file" <<'PY'
import base64, pathlib, sys
pathlib.Path(sys.argv[2]).write_bytes(base64.urlsafe_b64decode(sys.argv[1] + "=" * (-len(sys.argv[1]) % 4)))
PY
    "$IDRIVE" --config-dir "$(base_for_role "$2")/Config" provider write "$3" "$source_file" >/dev/null
    ;;
  wait-provider-read)
    [[ $# == 4 ]] || { echo "usage: $0 wait-provider-read <role> <path> <content-b64>" >&2; exit 2; }
    load_candidate
    expected="$ARTIFACT_DIR/$2-provider-expected"
    output="$ARTIFACT_DIR/$2-provider-output"
    python3 - "$4" "$expected" <<'PY'
import base64, pathlib, sys
pathlib.Path(sys.argv[2]).write_bytes(base64.urlsafe_b64decode(sys.argv[1] + "=" * (-len(sys.argv[1]) % 4)))
PY
    deadline=$((SECONDS + 45))
    while ((SECONDS < deadline)); do
      if "$IDRIVE" --config-dir "$(base_for_role "$2")/Config" provider read "$3" "$output" >/dev/null 2>&1 \
        && cmp -s "$expected" "$output"; then
        echo "MACOS_ANDROID_PROVIDER_READ_OK"
        exit 0
      fi
      sleep 0.25
    done
    echo "macOS provider did not receive $3" >&2
    exit 1
    ;;
  stop)
    [[ $# == 2 ]] || { echo "usage: $0 stop <owner|joiner>" >&2; exit 2; }
    stop_role "$2"
    ;;
  cleanup)
    stop_role owner
    stop_role joiner
    rm -rf "$ARTIFACT_DIR"
    echo "MACOS_ANDROID_REMOTE_CLEANUP_OK"
    ;;
  *)
    echo "usage: $0 <prepare|start-owner|start-joiner|manual-prepare|manual-submit|fips-ready|link-ready|assert-joined-ui|status|evidence|provider-write|wait-provider-read|stop|cleanup>" >&2
    exit 2
    ;;
esac
