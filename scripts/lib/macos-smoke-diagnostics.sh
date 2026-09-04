# shellcheck shell=bash

capture_macos_smoke_failure_diagnostics() {
  local status="$1" diagnostics="$SMOKE_DIR/private-diagnostics" source_file
  ((status != 0)) || return 0
  truthy "${IRIS_DRIVE_MACOS_SMOKE_PRESERVE_ARTIFACTS:-0}" || return 0

  mkdir -p "$diagnostics"
  chmod 700 "$diagnostics"
  for source_file in \
    "$MACOS_SMOKE_APPROVE_JSON" \
    "$MACOS_SMOKE_APPROVAL_TIMING" \
    "$MACOS_SMOKE_BLOSSOM_REQUEST_LOG" \
    "$MACOS_SMOKE_RELAY_EVENT_LOG" \
    "$SMOKE_STATE_DIR/blossom-fixture.log" \
    "$SMOKE_STATE_DIR/relay-fixture.log" \
    "$MACOS_SMOKE_APPINTENTS_LOG" \
    "$APP_STDOUT" \
    "$APP_STDERR" \
    "$APP_DEBUG_LOG" \
    "$SMOKE_STATE_DIR/owner-daemon.stdout" \
    "$SMOKE_STATE_DIR/owner-daemon.stderr" \
    "$SMOKE_STATE_DIR/joiner-daemon.stdout" \
    "$SMOKE_STATE_DIR/joiner-daemon.stderr"; do
    [[ -f "$source_file" ]] || continue
    cp "$source_file" "$diagnostics/$(basename "$source_file")"
  done

  for source_file in "$SMOKE_DIR"/*.daemon.stdout "$SMOKE_DIR"/*.daemon.stderr \
    "$SMOKE_DIR"/owner-app.stdout.log "$SMOKE_DIR"/owner-app.stderr.log; do
    [[ -f "$source_file" ]] || continue
    cp "$source_file" "$diagnostics/$(basename "$source_file")"
  done

  python3 - "$MACOS_SMOKE_APPROVE_JSON" "$MACOS_SMOKE_BLOSSOM_REQUEST_LOG" \
    "$MACOS_SMOKE_RELAY_EVENT_LOG" \
    >"$diagnostics/approval-phase-summary.json" <<'PY'
import json
from pathlib import Path
import sys

approve_path = Path(sys.argv[1])
requests_path = Path(sys.argv[2])
relay_path = Path(sys.argv[3])
approve = json.loads(approve_path.read_text()) if approve_path.is_file() else {}
requests = []
if requests_path.is_file():
    for line in requests_path.read_text().splitlines():
        try:
            requests.append(json.loads(line))
        except json.JSONDecodeError:
            pass
relay_events = []
if relay_path.is_file():
    for line in relay_path.read_text().splitlines():
        try:
            relay_events.append(json.loads(line))
        except json.JSONDecodeError:
            pass
summary = {
    "approve": approve,
    "blossom_request_count": len(requests),
    "blossom_requests": requests,
    "expected_relay_publish_order": ["roster", "drive_root", "approval_receipt"],
    "relay_event_count": len(relay_events),
    "relay_events": relay_events,
    "relay_published_event_count": int(approve.get("published_approval_events") or 0),
    "relay_publish_error": approve.get("approval_publish_error"),
}
print(json.dumps(summary, indent=2, sort_keys=True))
PY
  chmod 600 "$diagnostics"/* 2>/dev/null || true
}
