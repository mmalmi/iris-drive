# shellcheck shell=bash
# shellcheck disable=SC2034,SC2154

MACOS_OWNER_LINK_TIMEOUT_SECS="${IRIS_DRIVE_MACOS_OWNER_LINK_TIMEOUT_SECS:-15}"
MACOS_FORWARD_LINK_OWNER_PORT=$((41000 + $$ % 1000))
MACOS_FORWARD_LINK_JOINER_PORT=$((MACOS_FORWARD_LINK_OWNER_PORT + 1))
MACOS_FORWARD_LINK_JOINER_DAEMON_PID=""
MACOS_OWNER_LINK_JOINER_DAEMON_PID=""
MACOS_OWNER_LINK_OWNER_CONFIG=""
MACOS_OWNER_LINK_JOINER_CONFIG=""
MACOS_OWNER_LINK_OWNER_NPUB=""
MACOS_OWNER_LINK_JOINER_NPUB=""
MACOS_OWNER_LINK_EXPECTED_ROSTER=""
MACOS_OWNER_LINK_SOURCE=""
MACOS_OWNER_LINK_OUTPUT=""
MACOS_OWNER_LINK_PATH=""

macos_owner_link_monotonic_milliseconds() {
  perl -MTime::HiRes=clock_gettime,CLOCK_MONOTONIC \
    -e 'printf "%.0f\n", clock_gettime(CLOCK_MONOTONIC) * 1000'
}

validate_macos_owner_link_timeout() {
  if [[ ! "$MACOS_OWNER_LINK_TIMEOUT_SECS" =~ ^[1-9][0-9]*$ ]] \
    || (( MACOS_OWNER_LINK_TIMEOUT_SECS > 15 )); then
      echo "FAIL: IRIS_DRIVE_MACOS_OWNER_LINK_TIMEOUT_SECS must be 1..15" >&2
      return 1
  fi
}

macos_owner_link_status() {
  "$IDRIVE_CLI" --config-dir "$1" status 2>/dev/null || true
}

macos_owner_link_dump_statuses() {
  local config label
  for label in owner joiner; do
    if [[ "$label" == owner ]]; then
      config="$MACOS_OWNER_LINK_OWNER_CONFIG"
    else
      config="$MACOS_OWNER_LINK_JOINER_CONFIG"
    fi
    echo "macOS $label device-link status:" >&2
    macos_owner_link_status "$config" | python3 -c '
import json, sys
s = json.load(sys.stdin); p = s.get("profile") or {}; f = ((s.get("network") or {}).get("fips") or {})
print(json.dumps({
    "authorization_state": p.get("authorization_state"),
    "roster_size": p.get("roster_size"),
    "pending_device_approval_receipt_count": p.get("pending_device_approval_receipt_count"),
    "udp_bind_addr": f.get("udp_bind_addr"),
    "direct_peers": f.get("direct_peers", []),
    "peers": [{k: x.get(k) for k in ("app_key_npub", "fips_online", "fips_direct_online")}
              for x in s.get("peers", [])],
}, separators=(",", ":")))
' >&2 || true
  done
}

macos_owner_link_wait_before() {
  local label="$1" deadline="$2" check="$3" start="$4" now
  while true; do
    now="$(macos_owner_link_monotonic_milliseconds)"
    if (( now > deadline )); then
      echo "FAIL: $label exceeded $((deadline - start))ms" >&2
      macos_owner_link_dump_statuses
      return 1
    fi
    if "$check"; then
      now="$(macos_owner_link_monotonic_milliseconds)"
      if (( now <= deadline )); then
        echo "ok: $label ($((now - start))ms)"
        return 0
      fi
      echo "FAIL: $label became true after $((now - start))ms" >&2
      macos_owner_link_dump_statuses
      return 1
    fi
    sleep 0.1
  done
}

macos_owner_link_wait_for() {
  local label="$1" seconds="$2" check="$3" start
  start="$(macos_owner_link_monotonic_milliseconds)"
  macos_owner_link_wait_before "$label" "$((start + seconds * 1000))" "$check" "$start"
}

stop_macos_owner_daemon() {
  [[ -n "$OWNER_DAEMON_PID" ]] || return 0
  kill "$OWNER_DAEMON_PID" >/dev/null 2>&1 || true
  wait "$OWNER_DAEMON_PID" >/dev/null 2>&1 || true
  OWNER_DAEMON_PID=""
}

stop_macos_owner_link_joiner_daemon() {
  [[ -n "$MACOS_OWNER_LINK_JOINER_DAEMON_PID" ]] || return 0
  kill "$MACOS_OWNER_LINK_JOINER_DAEMON_PID" >/dev/null 2>&1 || true
  wait "$MACOS_OWNER_LINK_JOINER_DAEMON_PID" >/dev/null 2>&1 || true
  MACOS_OWNER_LINK_JOINER_DAEMON_PID=""
}

stop_macos_forward_link_joiner_daemon() {
  [[ -n "$MACOS_FORWARD_LINK_JOINER_DAEMON_PID" ]] || return 0
  kill "$MACOS_FORWARD_LINK_JOINER_DAEMON_PID" >/dev/null 2>&1 || true
  wait "$MACOS_FORWARD_LINK_JOINER_DAEMON_PID" >/dev/null 2>&1 || true
  MACOS_FORWARD_LINK_JOINER_DAEMON_PID=""
}

start_macos_owner_link_daemon() {
  local config="$1" bind_port="$2" peer_npub="$3" peer_port="$4" log="$5" pid_var="$6"
  local daemon_idrive="$APP_PATH/Contents/MacOS/idrive"
  [[ -x "$daemon_idrive" ]] || {
    echo "FAIL: shipped macOS app does not contain idrive" >&2
    return 1
  }
  env \
    IRIS_DRIVE_FIPS_UDP_BIND_ADDR="127.0.0.1:$bind_port" \
    IRIS_DRIVE_FIPS_UDP_EXTERNAL_ADDR="127.0.0.1:$bind_port" \
    IRIS_DRIVE_FIPS_UDP_PUBLIC=false \
    IRIS_DRIVE_FIPS_ENABLE_UDP=true \
    IRIS_DRIVE_FIPS_ENABLE_LAN_DISCOVERY=false \
    IRIS_DRIVE_FIPS_ENABLE_NOSTR_DISCOVERY=false \
    IRIS_DRIVE_FIPS_ENABLE_MESH_PUBSUB=false \
    IRIS_DRIVE_FIPS_ENABLE_LOCAL_RENDEZVOUS=false \
    IRIS_DRIVE_FIPS_ENABLE_WEBRTC=false \
    IRIS_DRIVE_FIPS_ENABLE_BOOTSTRAP=false \
    IRIS_DRIVE_FIPS_SHARE_LOCAL_CANDIDATES=false \
    IRIS_DRIVE_FIPS_OPEN_DISCOVERY_MAX_PENDING=2 \
    IRIS_DRIVE_FIPS_STATIC_PEERS="$peer_npub=127.0.0.1:$peer_port" \
    IRIS_FIPS_WEBSOCKET_SEED_URLS= \
    "$daemon_idrive" --config-dir "$config" daemon --watch-interval 0 --no-gateway \
    >"$log.stdout" 2>"$log.stderr" &
  printf -v "$pid_var" '%s' "$!"
  sleep 0.2
  if ! kill -0 "${!pid_var}" >/dev/null 2>&1; then
    echo "FAIL: macOS device-link daemon did not start for $config" >&2
    cat "$log.stderr" >&2 2>/dev/null || true
    return 1
  fi
}

launch_macos_forward_link_shell() {
  local owner_npub="$1"
  terminate_app_process
  terminate_smoke_daemon_processes
  open -n \
    --stdout "$APP_STDOUT" \
    --stderr "$APP_STDERR" \
    --env "HOME=$SMOKE_HOME" \
    --env "CFFIXED_USER_HOME=$SMOKE_HOME" \
    --env "IRIS_DRIVE_APP_BASE_DIR=$SMOKE_APP_DATA" \
    --env "IRIS_DRIVE_DEBUG_LOG_DIR=$APP_DEBUG_LOG_DIR" \
    --env "IRIS_DRIVE_DISABLE_LOGIN_AGENT_SYNC=true" \
    --env "IRIS_DRIVE_DISABLE_DAEMON_SERVICE=true" \
    --env "IRIS_DRIVE_ENABLE_E2E_NOTIFICATIONS=1" \
    --env "IRIS_DRIVE_EXTERNAL_DAEMON=true" \
    --env "IRIS_DRIVE_FIPS_UDP_BIND_ADDR=127.0.0.1:$MACOS_FORWARD_LINK_JOINER_PORT" \
    --env "IRIS_DRIVE_FIPS_UDP_EXTERNAL_ADDR=127.0.0.1:$MACOS_FORWARD_LINK_JOINER_PORT" \
    --env "IRIS_DRIVE_FIPS_UDP_PUBLIC=false" \
    --env "IRIS_DRIVE_FIPS_ENABLE_UDP=true" \
    --env "IRIS_DRIVE_FIPS_ENABLE_BOOTSTRAP=false" \
    --env "IRIS_DRIVE_FIPS_ENABLE_WEBRTC=false" \
    --env "IRIS_DRIVE_FIPS_ENABLE_LAN_DISCOVERY=false" \
    --env "IRIS_DRIVE_FIPS_ENABLE_NOSTR_DISCOVERY=false" \
    --env "IRIS_DRIVE_FIPS_ENABLE_MESH_PUBSUB=false" \
    --env "IRIS_DRIVE_FIPS_ENABLE_LOCAL_RENDEZVOUS=false" \
    --env "IRIS_DRIVE_FIPS_SHARE_LOCAL_CANDIDATES=false" \
    --env "IRIS_DRIVE_FIPS_OPEN_DISCOVERY_MAX_PENDING=0" \
    --env "IRIS_DRIVE_FIPS_STATIC_PEERS=$owner_npub=127.0.0.1:$MACOS_FORWARD_LINK_OWNER_PORT" \
    --env "IRIS_FIPS_WEBSOCKET_SEED_URLS=" \
    "$APP_PATH"
  wait_for_app_process 10 || {
    echo "FAIL: macOS joining shell did not relaunch with its direct owner hint" >&2
    return 1
  }
}

macos_owner_link_cancel_preserved_request() {
  local owner_status joiner_status
  owner_status="$(macos_owner_link_status "$MACOS_OWNER_LINK_OWNER_CONFIG")"
  joiner_status="$(macos_owner_link_status "$MACOS_OWNER_LINK_JOINER_CONFIG")"
  printf '%s' "$owner_status" | python3 -c '
import json, sys
p = (json.load(sys.stdin).get("profile") or {})
expected = int(sys.argv[1])
raise SystemExit(0 if p.get("roster_size") == expected and p.get("pending_device_approval_receipt_count") == 0 else 1)
' "$((MACOS_OWNER_LINK_EXPECTED_ROSTER - 1))" || return 1
  printf '%s' "$joiner_status" | python3 -c '
import json, sys
p = (json.load(sys.stdin).get("profile") or {})
raise SystemExit(0 if p.get("authorization_state") == "awaiting_approval" else 1)
'
}

macos_owner_link_joiner_direct() {
  local owner_status joiner_status
  owner_status="$(macos_owner_link_status "$MACOS_OWNER_LINK_OWNER_CONFIG")"
  joiner_status="$(macos_owner_link_status "$MACOS_OWNER_LINK_JOINER_CONFIG")"
  printf '%s' "$owner_status" | python3 -c '
import json, sys
s = json.load(sys.stdin); p = s.get("profile") or {}
f = ((s.get("network") or {}).get("fips") or {})
joiner = sys.argv[1]
ok = (p.get("authorization_state") == "authorized"
      and f.get("udp_bind_addr")
      and joiner in f.get("direct_peers", []))
raise SystemExit(0 if ok else 1)
' "$MACOS_OWNER_LINK_JOINER_NPUB" || return 1
  printf '%s' "$joiner_status" | python3 -c '
import json, sys
s = json.load(sys.stdin); p = s.get("profile") or {}
f = ((s.get("network") or {}).get("fips") or {})
owner = sys.argv[1]
ok = (p.get("authorization_state") == "awaiting_approval"
      and f.get("udp_bind_addr")
      and owner in f.get("direct_peers", []))
raise SystemExit(0 if ok else 1)
' "$MACOS_OWNER_LINK_OWNER_NPUB"
}

macos_owner_link_complete() {
  local owner_status joiner_status
  owner_status="$(macos_owner_link_status "$MACOS_OWNER_LINK_OWNER_CONFIG")"
  joiner_status="$(macos_owner_link_status "$MACOS_OWNER_LINK_JOINER_CONFIG")"
  printf '%s' "$owner_status" | python3 -c '
import json, sys
s = json.load(sys.stdin); p = s.get("profile") or {}
peer, expected = sys.argv[1], int(sys.argv[2])
ok = (p.get("authorization_state") == "authorized"
      and p.get("roster_size") == expected
      and p.get("pending_device_approval_receipt_count") == 0
      and any(x.get("app_key_npub") == peer
              and x.get("fips_online") is True
              and x.get("fips_direct_online") is True for x in s.get("peers", [])))
raise SystemExit(0 if ok else 1)
' "$MACOS_OWNER_LINK_JOINER_NPUB" "$MACOS_OWNER_LINK_EXPECTED_ROSTER" || return 1
  printf '%s' "$joiner_status" | python3 -c '
import json, sys
s = json.load(sys.stdin); p = s.get("profile") or {}
peer, expected = sys.argv[1], int(sys.argv[2])
ok = (p.get("authorization_state") == "authorized"
      and p.get("roster_size") == expected
      and p.get("pending_device_approval_receipt_count") == 0
      and any(x.get("app_key_npub") == peer
              and x.get("fips_online") is True
              and x.get("fips_direct_online") is True for x in s.get("peers", [])))
raise SystemExit(0 if ok else 1)
' "$MACOS_OWNER_LINK_OWNER_NPUB" "$MACOS_OWNER_LINK_EXPECTED_ROSTER"
}

macos_owner_link_file_visible() {
  "$IDRIVE_CLI" --config-dir "$MACOS_OWNER_LINK_OWNER_CONFIG" \
    provider read "$MACOS_OWNER_LINK_PATH" "$MACOS_OWNER_LINK_OUTPUT" >/dev/null 2>&1 \
    && cmp -s "$MACOS_OWNER_LINK_SOURCE" "$MACOS_OWNER_LINK_OUTPUT"
}

launch_macos_owner_link_shell() {
  local owner_base="${MACOS_OWNER_LINK_OWNER_CONFIG%/Config}"
  open -n \
    --stdout "$SMOKE_DIR/owner-app.stdout.log" \
    --stderr "$SMOKE_DIR/owner-app.stderr.log" \
    --env "HOME=$SMOKE_HOME" \
    --env "CFFIXED_USER_HOME=$SMOKE_HOME" \
    --env "IRIS_DRIVE_APP_BASE_DIR=$owner_base" \
    --env "IRIS_DRIVE_DEBUG_LOG_DIR=$APP_DEBUG_LOG_DIR" \
    --env "IRIS_DRIVE_DISABLE_LOGIN_AGENT_SYNC=true" \
    --env "IRIS_DRIVE_DISABLE_SINGLE_INSTANCE=true" \
    --env "IRIS_DRIVE_EXTERNAL_DAEMON=true" \
    "$APP_PATH"
  wait_for_app_process 10 || {
    echo "FAIL: macOS owner shell did not launch" >&2
    return 1
  }
  request_show_control_panel
}

run_macos_owner_device_link_journey() {
  local owner_config="$1" owner_npub="$2"
  local invite_json invite_url request_json request_url request_admin initial_status
  local app_pid owner_port joiner_port approval_started approval_deadline source_content

  validate_macos_owner_link_timeout || return 1
  MACOS_OWNER_LINK_OWNER_CONFIG="$owner_config"
  MACOS_OWNER_LINK_OWNER_NPUB="$owner_npub"
  initial_status="$(macos_owner_link_status "$owner_config")"
  MACOS_OWNER_LINK_EXPECTED_ROSTER="$(printf '%s' "$initial_status" | json_get profile.roster_size)" || {
    echo "FAIL: macOS owner status omitted profile.roster_size" >&2
    return 1
  }
  MACOS_OWNER_LINK_EXPECTED_ROSTER=$((MACOS_OWNER_LINK_EXPECTED_ROSTER + 1))

  invite_json="$("$IDRIVE_CLI" --config-dir "$owner_config" app-keys invite)"
  invite_url="$(printf '%s' "$invite_json" | json_get url)"
  [[ "$invite_url" == https://drive.iris.to/invite/* ]] || {
    echo "FAIL: macOS owner did not create a canonical invite" >&2
    return 1
  }
  MACOS_OWNER_LINK_JOINER_CONFIG="$SMOKE_DIR/macos-owner-link-joiner/Config"
  mkdir_p_or_fail "$MACOS_OWNER_LINK_JOINER_CONFIG"
  configure_macos_smoke_relay "$MACOS_OWNER_LINK_JOINER_CONFIG" || return 1
  request_json="$("$IDRIVE_CLI" --config-dir "$MACOS_OWNER_LINK_JOINER_CONFIG" \
    app-keys request "$invite_url" --label "macOS owner GUI joiner")"
  MACOS_OWNER_LINK_JOINER_NPUB="$(printf '%s' "$request_json" | json_get current_app_key_npub)"
  request_url="$(printf '%s' "$request_json" | json_get app_key_link_request.url)"
  request_admin="$(printf '%s' "$request_json" | json_get app_key_link_request.admin_app_key_npub)"
  [[ "$request_url" == https://drive.iris.to/approve-device/* \
    && "$request_admin" == "$owner_npub" ]] || {
      echo "FAIL: macOS owner-role join request metadata is invalid" >&2
      return 1
    }

  terminate_app_process
  terminate_smoke_daemon_processes
  stop_macos_forward_link_joiner_daemon
  stop_macos_owner_daemon
  SMOKE_CONFIG_DIR="$owner_config"
  owner_port=$((43000 + $$ % 1000))
  joiner_port=$((owner_port + 1))
  start_macos_owner_link_daemon \
    "$MACOS_OWNER_LINK_JOINER_CONFIG" "$joiner_port" "$owner_npub" "$owner_port" \
    "$SMOKE_DIR/macos-owner-link-joiner.daemon" MACOS_OWNER_LINK_JOINER_DAEMON_PID
  start_macos_owner_link_daemon \
    "$owner_config" "$owner_port" "$MACOS_OWNER_LINK_JOINER_NPUB" "$joiner_port" \
    "$SMOKE_DIR/macos-owner-link-owner.daemon" OWNER_DAEMON_PID
  macos_owner_link_wait_for \
    "macOS owner-role joiner established its direct owner route" 10 \
    macos_owner_link_joiner_direct || return 1
  launch_macos_owner_link_shell
  app_pid="$(app_process_pids | head -n 1)"

  open -a "$APP_PATH" "$request_url"
  /usr/bin/swift "$ROOT/scripts/macos-device-link-ax.swift" "$app_pid" Cancel 10
  sleep 1
  macos_owner_link_cancel_preserved_request || {
    echo "FAIL: Cancel changed the macOS owner roster or queued an approval" >&2
    return 1
  }

  approval_started="$(macos_owner_link_monotonic_milliseconds)"
  approval_deadline=$((approval_started + MACOS_OWNER_LINK_TIMEOUT_SECS * 1000))
  open -a "$APP_PATH" "$request_url"
  /usr/bin/swift "$ROOT/scripts/macos-device-link-ax.swift" "$app_pid" Approve 10
  macos_owner_link_wait_before \
    "macOS GUI approval reached the external joiner and drained its ACK" \
    "$approval_deadline" macos_owner_link_complete "$approval_started"

  MACOS_OWNER_LINK_PATH="e2e/macos-owner-link-$$.txt"
  MACOS_OWNER_LINK_SOURCE="$SMOKE_DIR/macos-owner-link-source.txt"
  MACOS_OWNER_LINK_OUTPUT="$SMOKE_DIR/macos-owner-link-output.txt"
  source_content="written by a device linked through the macOS owner shell"
  printf '%s\n' "$source_content" >"$MACOS_OWNER_LINK_SOURCE"
  "$IDRIVE_CLI" --config-dir "$MACOS_OWNER_LINK_JOINER_CONFIG" \
    provider write "$MACOS_OWNER_LINK_PATH" "$MACOS_OWNER_LINK_SOURCE" >/dev/null
  macos_owner_link_wait_for \
    "post-link provider write visible on the macOS owner" \
    "$MACOS_OWNER_LINK_TIMEOUT_SECS" macos_owner_link_file_visible
  echo "MACOS_OWNER_DEVICE_LINK_JOURNEY_OK"
}
