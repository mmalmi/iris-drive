#!/usr/bin/env bash

set -Eeuo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
SMOKE="$ROOT/scripts/macos-smoke.sh"
LIFECYCLE="$ROOT/scripts/lib/macos-blossom-smoke.sh"
SERVER="$ROOT/scripts/local-blossom-server.py"
TEST_DIR="$(mktemp -d -t iris-drive-blossom-fixture-check)"
SERVER_PID=""

cleanup() {
  local status=$?
  trap - EXIT
  set +e
  if [[ -n "$SERVER_PID" ]]; then
    kill "$SERVER_PID" >/dev/null 2>&1 || true
    wait "$SERVER_PID" >/dev/null 2>&1 || true
  fi
  rm -rf "$TEST_DIR"
  exit "$status"
}
trap cleanup EXIT

require_contains() {
  local needle="$1"
  local file="${2:-$SMOKE}"
  grep -F -- "$needle" "$file" >/dev/null || {
    echo "FAIL: $file must contain: $needle" >&2
    exit 1
  }
}

require_contains 'source "$ROOT/scripts/lib/macos-blossom-smoke.sh"'
require_contains 'start_macos_smoke_blossom'
require_contains 'configure_macos_smoke_blossom "$SMOKE_CONFIG_DIR"'
require_contains 'configure_macos_smoke_blossom "$owner_config_dir"'
require_contains 'configure_macos_smoke_blossom "$SMOKE_DIR/macos-owner-link-joiner/Config"'
require_contains 'stop_macos_smoke_blossom'
require_contains 'for artifact in "$SMOKE_DIR"/*; do'
if [[ "$(grep -Fc 'matching = [report for report in reports if report.get("kind") == sys.argv[1]]' "$SMOKE")" -ne 2 ]]; then
  echo "FAIL: backup assertions must select their target from multi-target fixture reports" >&2
  exit 1
fi
require_contains 'x-sha-256' "$SERVER"
require_contains 'hashlib.sha256(body).hexdigest()' "$SERVER"
require_contains 'os.O_CREAT | os.O_EXCL | os.O_WRONLY' "$SERVER"
require_contains 'def do_HEAD' "$SERVER"
require_contains 'def do_GET' "$SERVER"
require_contains '--request-log' "$SERVER"
require_contains 'request_log' "$SERVER"
require_contains 'blossom-servers remove https://upload.iris.to' "$LIFECYCLE"
require_contains 'blossom-servers add "$MACOS_SMOKE_BLOSSOM_URL"' "$LIFECYCLE"
require_contains '--request-log "$MACOS_SMOKE_BLOSSOM_REQUEST_LOG"' "$LIFECYCLE"
require_contains 'capture_macos_smoke_failure_diagnostics "$status"'
require_contains 'MACOS_SMOKE_APPROVE_JSON'

python3 - "$SMOKE" <<'PY'
from pathlib import Path
import sys

source = Path(sys.argv[1]).read_text()
cleanup = source.split("\ncleanup() {", 1)[1].split("\ntrap cleanup EXIT", 1)[0]
if cleanup.index("stop_macos_smoke_blossom") > cleanup.index(
    'remove_smoke_path_best_effort "$SMOKE_STATE_DIR"'
):
    raise SystemExit("local Blossom must stop before its state directory is removed")
if 'artifacts=("$SMOKE_DIR"/*)' in cleanup:
    raise SystemExit("empty artifact cleanup must not expand an array under macOS Bash nounset")
PY

ready_file="$TEST_DIR/ready"
storage_dir="$TEST_DIR/blobs"
python3 "$SERVER" \
  --host 127.0.0.1 \
  --port 0 \
  --storage-dir "$storage_dir" \
  --ready-file "$ready_file" \
  >"$TEST_DIR/server.stdout" 2>"$TEST_DIR/server.stderr" &
SERVER_PID=$!

for _ in $(seq 1 100); do
  [[ -s "$ready_file" ]] && break
  kill -0 "$SERVER_PID" >/dev/null 2>&1 || {
    sed -n '1,120p' "$TEST_DIR/server.stderr" >&2
    exit 1
  }
  sleep 0.05
done
[[ -s "$ready_file" ]] || {
  echo "FAIL: local Blossom fixture did not become ready" >&2
  exit 1
}
IFS= read -r server_url <"$ready_file"

payload='deterministic macOS Blossom fixture'
payload_hash="$(printf '%s' "$payload" | shasum -a 256 | awk '{print $1}')"
status="$(
  printf '%s' "$payload" | curl -sS -o "$TEST_DIR/upload-response" -w '%{http_code}' \
    -X PUT -H "x-sha-256: $payload_hash" --data-binary @- "$server_url/upload"
)"
[[ "$status" == 201 ]] || {
  echo "FAIL: local Blossom upload returned HTTP $status" >&2
  exit 1
}
curl -fsSI "$server_url/$payload_hash.bin" >/dev/null
[[ "$(curl -fsS "$server_url/$payload_hash.bin")" == "$payload" ]]

status="$(
  printf '%s' "$payload" | curl -sS -o "$TEST_DIR/duplicate-response" -w '%{http_code}' \
    -X PUT -H "x-sha-256: $payload_hash" --data-binary @- "$server_url/upload"
)"
[[ "$status" == 409 ]] || {
  echo "FAIL: duplicate Blossom upload returned HTTP $status" >&2
  exit 1
}

status="$(
  printf '%s' 'wrong digest' | curl -sS -o "$TEST_DIR/digest-response" -w '%{http_code}' \
    -X PUT -H "x-sha-256: $payload_hash" --data-binary @- "$server_url/upload"
)"
[[ "$status" == 400 ]] || {
  echo "FAIL: invalid Blossom digest returned HTTP $status" >&2
  exit 1
}

kill "$SERVER_PID"
wait "$SERVER_PID"
SERVER_PID=""
[[ ! -e "$ready_file" ]] || {
  echo "FAIL: local Blossom fixture left stale readiness state" >&2
  exit 1
}

echo "MACOS_SMOKE_BLOSSOM_FIXTURE_OK"
