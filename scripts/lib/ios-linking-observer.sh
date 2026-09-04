#!/usr/bin/env bash

# Sourced by ios-gui-linking-smoke.sh. The caller owns all paths, processes,
# and simulator lifecycle referenced by these approval-observation helpers.

approval_deadline() {
  local state_file="$1"
  python3 - "$state_file" <<'PY'
import datetime
import json
import sys

with open(sys.argv[1], encoding="utf-8") as handle:
    state = json.load(handle)
events = [
    event for event in state.get("ios_config_mutation_audit", [])
    if event.get("action") == "approve_device" and not event.get("error")
]
if not events:
    raise SystemExit("approval audit marker is missing")
timestamp = events[-1]["timestamp"].replace("Z", "+00:00")
print(datetime.datetime.fromisoformat(timestamp).timestamp() + 15)
PY
}

approval_receipt_consumed_at() {
  local state_file="$1"
  local deadline="$2"
  python3 - "$state_file" "$deadline" <<'PY'
import datetime
import json
import sys

with open(sys.argv[1], encoding="utf-8") as handle:
    state = json.load(handle)
approval_started_at = float(sys.argv[2]) - 15
transitions = []
for event in state.get("ios_foreground_sync_audit", []):
    if event.get("phase") != "pending_receipt_transition":
        continue
    if int(event.get("pendingBefore") or 0) <= 0:
        continue
    if int(event.get("pendingAfter") or 0) != 0:
        continue
    timestamp = event["timestamp"].replace("Z", "+00:00")
    persisted_at = datetime.datetime.fromisoformat(timestamp).timestamp()
    if persisted_at >= approval_started_at:
        transitions.append(persisted_at)
if not transitions:
    raise SystemExit("persisted pending-receipt transition is missing")
print(max(transitions))
PY
}

before_deadline() {
  python3 -c 'import sys,time; raise SystemExit(0 if time.time() <= float(sys.argv[1]) else 1)' "$1"
}

start_linked_device_sync_observer() {
  local linked_config_dir="$1"
  local owner_config_dir="$2"
  local expected_owner_roster="$3"
  local linked_observation_file="$4"
  local ack_observation_file="$5"
  local pid_variable="$6"
  local observer_deadline
  observer_deadline=$(( $(date +%s) + 60 ))
  : >"$linked_observation_file"
  : >"$ack_observation_file"
  (
    while before_deadline "$observer_deadline"; do
      if [[ ! -s "$linked_observation_file" ]]; then
        "$IDRIVE" --config-dir "$linked_config_dir" sync \
          --relay "$LOCAL_RELAY_URL" --timeout 2 >/dev/null 2>&1 || true
        # XCTest can terminate the owner as soon as its locally linked row is
        # visible, before the native exchange publishes the durable approval
        # receipt. Once that exact persisted state exists, keep the real iOS
        # product exchange alive; never mutate or sync the owner from the host.
        if "$IDRIVE" --config-dir "$owner_config_dir" status 2>/dev/null \
          | python3 -c 'import json,sys; p=json.load(sys.stdin).get("profile") or {}; raise SystemExit(0 if int(p.get("roster_size") or 0) == int(sys.argv[1]) and int(p.get("pending_device_approval_receipt_count") or 0) > 0 else 1)' \
            "$expected_owner_roster" >/dev/null 2>&1; then
          xcrun simctl launch "$DEVICE_UDID" "$BUNDLE_ID" >/dev/null 2>&1 || true
        fi
        if "$IDRIVE" --config-dir "$linked_config_dir" status 2>/dev/null \
          | python3 -c 'import json,sys; p=(json.load(sys.stdin).get("profile") or {}); raise SystemExit(0 if p.get("authorization_state") == "authorized" else 1)' >/dev/null 2>&1; then
          python3 -c 'import time; print(time.time())' >"$linked_observation_file"
        fi
      fi
      if [[ -s "$linked_observation_file" ]]; then
        # XCTest may terminate the owner immediately after the approval UI
        # returns. Launching an already-running app is non-destructive, while
        # repeating it during teardown keeps mobile ACK ingestion alive.
        xcrun simctl launch "$DEVICE_UDID" "$BUNDLE_ID" >/dev/null 2>&1 || true
        if "$IDRIVE" --config-dir "$owner_config_dir" status 2>/dev/null \
          | python3 -c 'import json,sys; p=json.load(sys.stdin).get("profile") or {}; raise SystemExit(0 if int(p.get("roster_size") or 0) == int(sys.argv[1]) and int(p.get("pending_device_approval_receipt_count") or 0) == 0 else 1)' \
            "$expected_owner_roster" >/dev/null 2>&1; then
          python3 -c 'import time; print(time.time())' >"$ack_observation_file"
          exit 0
        fi
      fi
      sleep 0.2
    done
    exit 1
  ) &
  printf -v "$pid_variable" '%s' "$!"
}

assert_linked_device_exchange_observed_before() {
  local observer_pid="$1"
  local linked_observation_file="$2"
  local ack_observation_file="$3"
  local state_file="$4"
  local deadline="$5"
  local phase="$6"
  local linked_observed_at=""
  local ack_observed_at=""
  local product_ack_persisted_at=""

  while before_deadline "$deadline"; do
    product_ack_persisted_at="$(approval_receipt_consumed_at "$state_file" "$deadline" 2>/dev/null || true)"
    if [[ -s "$linked_observation_file" && -n "$product_ack_persisted_at" ]]; then
      break
    fi
    sleep 0.1
  done
  if kill -0 "$observer_pid" >/dev/null 2>&1; then
    kill "$observer_pid" >/dev/null 2>&1 || true
  fi
  wait "$observer_pid" >/dev/null 2>&1 || true

  if [[ -s "$linked_observation_file" ]]; then
    linked_observed_at="$(cat "$linked_observation_file")"
  fi
  if [[ -z "$linked_observed_at" ]] \
    || ! python3 -c 'import sys; raise SystemExit(0 if float(sys.argv[1]) <= float(sys.argv[2]) else 1)' \
      "$linked_observed_at" "$deadline"; then
    echo "FAIL: $phase linked CLI device did not apply the iOS approval within 15 seconds." >&2
    return 1
  fi
  if [[ -s "$ack_observation_file" ]]; then
    ack_observed_at="$(cat "$ack_observation_file")"
  fi
  if [[ -z "$product_ack_persisted_at" ]]; then
    product_ack_persisted_at="$(approval_receipt_consumed_at "$state_file" "$deadline" 2>/dev/null || true)"
  fi
  python3 - "$LOCAL_RELAY_EVENT_LOG" "$phase" <<'PY'
import json, sys
events = [json.loads(line) for line in open(sys.argv[1], encoding="utf-8") if line.strip()]
print("IOS_LOCAL_RELAY_EVENT_TIMELINE " + json.dumps({
    "phase": sys.argv[2],
    "events": events[-40:],
}, separators=(",", ":"), sort_keys=True))
PY
  if [[ -z "$product_ack_persisted_at" ]] \
    || ! python3 -c 'import sys; raise SystemExit(0 if float(sys.argv[1]) <= float(sys.argv[2]) else 1)' \
      "$product_ack_persisted_at" "$deadline"; then
    echo "FAIL: approval owner did not consume the $phase ACK within 15 seconds of approval." >&2
    return 1
  fi
  echo "IOS_APPROVAL_EXCHANGE_OBSERVED phase=$phase linked_observed_at=$linked_observed_at product_ack_persisted_at=$product_ack_persisted_at host_ack_observed_at=${ack_observed_at:-missing} deadline=$deadline"
}

wait_for_approval_ack() {
  local config_dir="$1"
  local phase="$2"
  local deadline="$3"
  local observed_at=""
  while before_deadline "$deadline"; do
    if "$IDRIVE" --config-dir "$config_dir" status 2>/dev/null \
      | python3 -c 'import json,sys; p=json.load(sys.stdin).get("profile") or {}; raise SystemExit(0 if p.get("pending_device_approval_receipt_count") == 0 else 1)' >/dev/null 2>&1; then
      observed_at="$(python3 -c 'import time; print(time.time())')"
      if before_deadline "$deadline"; then
        print_cli_owner_ack_diagnostics "$phase"
        echo "IOS_CLI_OWNER_ACK_OBSERVED phase=$phase host_observed_at=$observed_at deadline=$deadline"
        return 0
      fi
      break
    fi
    sleep 0.2
  done
  print_cli_owner_ack_diagnostics "$phase"
  echo "IOS_CLI_OWNER_ACK_OBSERVED phase=$phase host_observed_at=${observed_at:-missing} deadline=$deadline"
  echo "FAIL: approval owner did not consume the $phase ACK within 15 seconds of approval." >&2
  "$IDRIVE" --config-dir "$config_dir" status >&2 || true
  exit 1
}

print_cli_owner_ack_diagnostics() {
  local phase="$1"
  python3 - "$SIM_APP_BASE_DIR/native-app-key-link-audit.json" "$phase" <<'PY'
import json, os, sys
path = sys.argv[1]
events = []
if os.path.exists(path):
    try:
        events = json.load(open(path, encoding="utf-8"))
    except (OSError, json.JSONDecodeError):
        pass
print("IOS_NATIVE_APP_KEY_LINK_AUDIT " + json.dumps({
    "phase": sys.argv[2],
    "events": events[-40:],
}, separators=(",", ":"), sort_keys=True))
PY
  python3 - "$LOCAL_RELAY_EVENT_LOG" "$phase" <<'PY'
import json, sys
events = [json.loads(line) for line in open(sys.argv[1], encoding="utf-8") if line.strip()]
print("IOS_LOCAL_RELAY_EVENT_TIMELINE " + json.dumps({
    "phase": sys.argv[2],
    "events": events[-40:],
}, separators=(",", ":"), sort_keys=True))
PY
  python3 - "$OWNER_DAEMON_LOG" "$phase" <<'PY'
import json, sys
events = []
for line in open(sys.argv[1], encoding="utf-8", errors="replace"):
    try:
        event = json.loads(line)
    except json.JSONDecodeError:
        continue
    if "approval_applied_ack" not in str(event.get("event") or ""):
        continue
    events.append({key: event.get(key) for key in (
        "event", "event_id", "event_created_at", "received_at_ms",
        "config_lock_wait_ms", "persisted_at_ms", "pending_before",
        "pending_after", "outcome", "error",
    ) if key in event})
print("IOS_CLI_OWNER_ACK_DAEMON_TIMELINE " + json.dumps({
    "phase": sys.argv[2],
    "events": events[-20:],
}, separators=(",", ":"), sort_keys=True))
PY
}
