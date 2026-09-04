# shellcheck shell=bash

MACOS_SMOKE_RELAY_PID=""
MACOS_SMOKE_RELAY_URL=""
MACOS_SMOKE_RELAY_EVENT_LOG=""

start_macos_smoke_relay() {
  local ready_file="$SMOKE_STATE_DIR/relay-ready"
  local fixture_log="$SMOKE_STATE_DIR/relay-fixture.log"
  MACOS_SMOKE_RELAY_EVENT_LOG="$SMOKE_STATE_DIR/relay-events.jsonl"

  [[ -z "$MACOS_SMOKE_RELAY_PID" ]] || return 0
  rm -f "$ready_file"
  python3 "$ROOT/scripts/local-nostr-relay.py" \
    --ready-file "$ready_file" \
    --event-log "$MACOS_SMOKE_RELAY_EVENT_LOG" \
    >"$fixture_log" 2>&1 &
  MACOS_SMOKE_RELAY_PID=$!
  for _ in {1..100}; do
    if [[ -s "$ready_file" ]]; then
      IFS= read -r MACOS_SMOKE_RELAY_URL <"$ready_file"
      [[ "$MACOS_SMOKE_RELAY_URL" == ws://127.0.0.1:* ]] && return 0
    fi
    if ! kill -0 "$MACOS_SMOKE_RELAY_PID" >/dev/null 2>&1; then
      echo "FAIL: local Nostr relay fixture exited during startup." >&2
      sed -n '1,80p' "$fixture_log" >&2
      return 1
    fi
    sleep 0.05
  done
  echo "FAIL: local Nostr relay fixture did not become ready." >&2
  return 1
}

stop_macos_smoke_relay() {
  [[ -n "$MACOS_SMOKE_RELAY_PID" ]] || return 0
  kill "$MACOS_SMOKE_RELAY_PID" >/dev/null 2>&1 || true
  wait "$MACOS_SMOKE_RELAY_PID" >/dev/null 2>&1 || true
  MACOS_SMOKE_RELAY_PID=""
  MACOS_SMOKE_RELAY_URL=""
}

configure_macos_smoke_relay() {
  local config_dir="$1" relay servers
  [[ -n "$MACOS_SMOKE_RELAY_URL" ]] || {
    echo "FAIL: local Nostr relay fixture URL is unavailable." >&2
    return 1
  }
  mkdir_p_or_fail "$config_dir"
  servers="$("$IDRIVE_CLI" --config-dir "$config_dir" relays list)"
  while IFS= read -r relay; do
    [[ -n "$relay" ]] || continue
    "$IDRIVE_CLI" --config-dir "$config_dir" relays remove "$relay" >/dev/null
  done < <(printf '%s' "$servers" | python3 -c '
import json, sys
for relay in json.load(sys.stdin):
    print(relay)
')
  servers="$(
    "$IDRIVE_CLI" --config-dir "$config_dir" relays add "$MACOS_SMOKE_RELAY_URL"
  )"
  printf '%s' "$servers" | python3 -c '
import json, sys
servers = json.load(sys.stdin)
raise SystemExit(0 if servers == [sys.argv[1]] else 1)
' "$MACOS_SMOKE_RELAY_URL" || {
    echo "FAIL: smoke profile did not retain only the local Nostr relay fixture." >&2
    return 1
  }
}
