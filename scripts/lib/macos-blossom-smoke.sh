# shellcheck shell=bash

MACOS_SMOKE_BLOSSOM_PID=""
MACOS_SMOKE_BLOSSOM_URL=""
MACOS_SMOKE_BLOSSOM_REQUEST_LOG=""

start_macos_smoke_blossom() {
  local ready_file="$SMOKE_STATE_DIR/blossom-ready"
  local storage_dir="$SMOKE_STATE_DIR/blossom-blobs"
  local fixture_log="$SMOKE_STATE_DIR/blossom-fixture.log"
  MACOS_SMOKE_BLOSSOM_REQUEST_LOG="$SMOKE_STATE_DIR/blossom-requests.jsonl"

  [[ -z "$MACOS_SMOKE_BLOSSOM_PID" ]] || return 0
  rm -f "$ready_file"
  mkdir_p_or_fail "$storage_dir"
  python3 "$ROOT/scripts/local-blossom-server.py" \
    --host 127.0.0.1 \
    --port 0 \
    --storage-dir "$storage_dir" \
    --ready-file "$ready_file" \
    --request-log "$MACOS_SMOKE_BLOSSOM_REQUEST_LOG" \
    >"$fixture_log" 2>&1 &
  MACOS_SMOKE_BLOSSOM_PID=$!
  for _ in {1..100}; do
    if [[ -s "$ready_file" ]]; then
      IFS= read -r MACOS_SMOKE_BLOSSOM_URL <"$ready_file"
      if /usr/bin/curl -fsS "$MACOS_SMOKE_BLOSSOM_URL/health" >/dev/null; then
        return 0
      fi
    fi
    if ! kill -0 "$MACOS_SMOKE_BLOSSOM_PID" >/dev/null 2>&1; then
      echo "FAIL: local Blossom fixture exited during startup." >&2
      sed -n '1,80p' "$fixture_log" >&2
      return 1
    fi
    sleep 0.05
  done
  echo "FAIL: local Blossom fixture did not become ready." >&2
  return 1
}

stop_macos_smoke_blossom() {
  [[ -n "$MACOS_SMOKE_BLOSSOM_PID" ]] || return 0
  kill "$MACOS_SMOKE_BLOSSOM_PID" >/dev/null 2>&1 || true
  wait "$MACOS_SMOKE_BLOSSOM_PID" >/dev/null 2>&1 || true
  MACOS_SMOKE_BLOSSOM_PID=""
  MACOS_SMOKE_BLOSSOM_URL=""
}

configure_macos_smoke_blossom() {
  local config_dir="$1" servers
  [[ -n "$MACOS_SMOKE_BLOSSOM_URL" ]] || {
    echo "FAIL: local Blossom fixture URL is unavailable." >&2
    return 1
  }
  mkdir_p_or_fail "$config_dir"
  "$IDRIVE_CLI" --config-dir "$config_dir" \
    blossom-servers remove https://upload.iris.to >/dev/null
  servers="$(
    "$IDRIVE_CLI" --config-dir "$config_dir" \
      blossom-servers add "$MACOS_SMOKE_BLOSSOM_URL"
  )"
  printf '%s' "$servers" | python3 -c '
import json, sys
servers = json.load(sys.stdin)
raise SystemExit(0 if servers == [sys.argv[1]] else 1)
  ' "$MACOS_SMOKE_BLOSSOM_URL" || {
    echo "FAIL: smoke profile did not retain only the local Blossom fixture." >&2
    return 1
  }
}
