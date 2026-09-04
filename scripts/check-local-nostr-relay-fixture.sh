#!/usr/bin/env bash

set -Eeuo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
SERVER="$ROOT/scripts/local-nostr-relay.py"
SMOKE="$ROOT/scripts/macos-smoke.sh"
LIFECYCLE="$ROOT/scripts/lib/macos-relay-smoke.sh"
TEST_DIR="$(mktemp -d -t iris-drive-local-relay-check)"
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
  local needle="$1" file="$2"
  grep -F -- "$needle" "$file" >/dev/null || {
    echo "FAIL: $file must contain: $needle" >&2
    exit 1
  }
}

require_contains 'source "$ROOT/scripts/lib/macos-relay-smoke.sh"' "$SMOKE"
require_contains 'start_macos_smoke_relay' "$SMOKE"
require_contains 'configure_macos_smoke_relay "$SMOKE_CONFIG_DIR"' "$SMOKE"
require_contains 'configure_macos_smoke_relay "$owner_config_dir"' "$SMOKE"
require_contains 'configure_macos_smoke_relay "$MACOS_OWNER_LINK_JOINER_CONFIG"' \
  "$ROOT/scripts/lib/macos-device-link-smoke.sh"
require_contains 'stop_macos_smoke_relay' "$SMOKE"
require_contains 'relays remove "$relay"' "$LIFECYCLE"
require_contains 'relays add "$MACOS_SMOKE_RELAY_URL"' "$LIFECYCLE"
require_contains 'servers == [sys.argv[1]]' "$LIFECYCLE"
require_contains '--event-log "$MACOS_SMOKE_RELAY_EVENT_LOG"' "$LIFECYCLE"
require_contains 'IRIS_DRIVE_FIPS_ENABLE_NOSTR_DISCOVERY=false' "$ROOT/scripts/lib/macos-device-link-smoke.sh"
require_contains 'MACOS_OWNER_LINK_TIMEOUT_SECS:-15' "$ROOT/scripts/lib/macos-device-link-smoke.sh"
require_contains '--event-log' "$SERVER"
require_contains 'event_log' "$SERVER"

ready_file="$TEST_DIR/ready"
event_log="$TEST_DIR/events.jsonl"
python3 "$SERVER" --ready-file "$ready_file" --event-log "$event_log" \
  >"$TEST_DIR/server.stdout" 2>"$TEST_DIR/server.stderr" &
SERVER_PID=$!
for _ in $(seq 1 100); do
  [[ -s "$ready_file" ]] && break
  kill -0 "$SERVER_PID" >/dev/null 2>&1 || {
    sed -n '1,100p' "$TEST_DIR/server.stderr" >&2
    exit 1
  }
  sleep 0.05
done
[[ -s "$ready_file" ]] || {
  echo "FAIL: local Nostr relay did not become ready" >&2
  exit 1
}

python3 - "$ready_file" <<'PY'
import base64
import json
import os
import socket
import struct
import sys
from urllib.parse import urlsplit


def connect(url):
    parsed = urlsplit(url)
    sock = socket.create_connection((parsed.hostname, parsed.port), timeout=3)
    key = base64.b64encode(os.urandom(16)).decode()
    request = (
        "GET / HTTP/1.1\r\n"
        f"Host: {parsed.hostname}:{parsed.port}\r\n"
        "Upgrade: websocket\r\n"
        "Connection: Upgrade\r\n"
        f"Sec-WebSocket-Key: {key}\r\n"
        "Sec-WebSocket-Version: 13\r\n\r\n"
    )
    sock.sendall(request.encode())
    response = b""
    while b"\r\n\r\n" not in response:
        response += sock.recv(4096)
    if not response.startswith(b"HTTP/1.1 101"):
        raise RuntimeError(response.decode(errors="replace"))
    return sock


def send_json(sock, value):
    payload = json.dumps(value, separators=(",", ":")).encode()
    mask = os.urandom(4)
    header = bytearray([0x81])
    if len(payload) < 126:
        header.append(0x80 | len(payload))
    else:
        header.append(0x80 | 126)
        header.extend(struct.pack("!H", len(payload)))
    header.extend(mask)
    masked = bytes(byte ^ mask[index % 4] for index, byte in enumerate(payload))
    sock.sendall(header + masked)


def recv_json(sock):
    first, second = sock.recv(2)
    if first & 0x0F != 1:
        raise RuntimeError(f"unexpected opcode {first & 0x0F}")
    length = second & 0x7F
    if length == 126:
        length = struct.unpack("!H", sock.recv(2))[0]
    elif length == 127:
        length = struct.unpack("!Q", sock.recv(8))[0]
    payload = b""
    while len(payload) < length:
        payload += sock.recv(length - len(payload))
    return json.loads(payload)


url = open(sys.argv[1], encoding="utf-8").read().strip()
event_one = {
    "id": "a" * 64,
    "pubkey": "1" * 64,
    "created_at": 10,
    "kind": 39003,
    "tags": [["p", "2" * 64]],
    "content": "ciphertext-one",
    "sig": "3" * 128,
}
event_two = dict(event_one, id="b" * 64, created_at=11, kind=35001)
publisher = connect(url)
send_json(publisher, ["EVENT", event_one])
assert recv_json(publisher) == ["OK", event_one["id"], True, ""]
send_json(publisher, ["EVENT", event_two])
assert recv_json(publisher) == ["OK", event_two["id"], True, ""]

subscriber = connect(url)
send_json(subscriber, ["REQ", "sub", {"authors": [event_one["pubkey"]], "#p": ["2" * 64]}])
assert recv_json(subscriber) == ["EVENT", "sub", event_one]
assert recv_json(subscriber) == ["EVENT", "sub", event_two]
assert recv_json(subscriber) == ["EOSE", "sub"]
send_json(subscriber, ["CLOSE", "sub"])

live_subscriber = connect(url)
send_json(live_subscriber, ["REQ", "live", {"kinds": [30078]}])
assert recv_json(live_subscriber) == ["EOSE", "live"]
event_live = dict(event_one, id="c" * 64, created_at=12, kind=30078)
send_json(publisher, ["EVENT", event_live])
assert recv_json(publisher) == ["OK", event_live["id"], True, ""]
assert recv_json(live_subscriber) == ["EVENT", "live", event_live]
send_json(live_subscriber, ["CLOSE", "live"])
publisher.close()
subscriber.close()
live_subscriber.close()
PY

python3 - "$event_log" <<'PY'
import json, sys
events = [json.loads(line) for line in open(sys.argv[1], encoding="utf-8")]
assert [event["sequence"] for event in events] == [1, 2, 3]
assert [event["id"] for event in events] == ["a" * 64, "b" * 64, "c" * 64]
assert [event["kind"] for event in events] == [39003, 35001, 30078]
assert events[0]["tag_names"] == ["p"]
assert [event["created_at"] for event in events] == [10, 11, 12]
assert all(isinstance(event["received_at"], float) for event in events)
assert events[0]["received_at"] <= events[1]["received_at"]
PY

kill "$SERVER_PID"
wait "$SERVER_PID"
SERVER_PID=""
[[ ! -e "$ready_file" ]] || {
  echo "FAIL: local Nostr relay left stale readiness state" >&2
  exit 1
}

echo "LOCAL_NOSTR_RELAY_FIXTURE_OK"
