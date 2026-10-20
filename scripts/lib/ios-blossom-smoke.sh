# shellcheck shell=bash
# This loopback fixture exercises storage protocol/bytes, not TLS or upload-auth validation.

IOS_SMOKE_BLOSSOM_STATE=""
IOS_SMOKE_BLOSSOM_PID=""
IOS_SMOKE_BLOSSOM_URL=""

start_ios_smoke_blossom() {
  IOS_SMOKE_BLOSSOM_STATE="$(mktemp -d -t iris-drive-ios-blossom)"
  python3 "$ROOT/scripts/local-blossom-server.py" \
    --host 127.0.0.1 --port 0 \
    --storage-dir "$IOS_SMOKE_BLOSSOM_STATE/blobs" \
    --ready-file "$IOS_SMOKE_BLOSSOM_STATE/ready" \
    --request-log "$IOS_SMOKE_BLOSSOM_STATE/requests.jsonl" \
    >"$IOS_SMOKE_BLOSSOM_STATE/server.log" 2>&1 &
  IOS_SMOKE_BLOSSOM_PID=$!
  for _ in {1..100}; do
    if [[ -s "$IOS_SMOKE_BLOSSOM_STATE/ready" ]]; then
      IFS= read -r IOS_SMOKE_BLOSSOM_URL <"$IOS_SMOKE_BLOSSOM_STATE/ready"
      if curl -fsS "$IOS_SMOKE_BLOSSOM_URL/health" >/dev/null; then return 0; fi
    fi
    if ! kill -0 "$IOS_SMOKE_BLOSSOM_PID" >/dev/null 2>&1; then break; fi
    sleep 0.05
  done
  echo "FAIL: local iOS Blossom fixture did not become ready." >&2
  cat "$IOS_SMOKE_BLOSSOM_STATE/server.log" >&2
  return 1
}

stop_ios_smoke_blossom() {
  if [[ -n "$IOS_SMOKE_BLOSSOM_PID" ]]; then
    kill "$IOS_SMOKE_BLOSSOM_PID" >/dev/null 2>&1 || true
    wait "$IOS_SMOKE_BLOSSOM_PID" >/dev/null 2>&1 || true
  fi
  [[ -z "$IOS_SMOKE_BLOSSOM_STATE" ]] || rm -rf "$IOS_SMOKE_BLOSSOM_STATE"
}

configure_ios_smoke_blossom() {
  local config_dir="$1" configured
  [[ -n "$IOS_SMOKE_BLOSSOM_URL" ]] || return 1
  "$IDRIVE" --config-dir "$config_dir" blossom-servers remove https://upload.iris.to >/dev/null
  configured="$("$IDRIVE" --config-dir "$config_dir" blossom-servers add "$IOS_SMOKE_BLOSSOM_URL")"
  python3 -c 'import json,sys; raise SystemExit(0 if json.load(sys.stdin) == [sys.argv[1]] else 1)' \
    "$IOS_SMOKE_BLOSSOM_URL" <<<"$configured" || {
    echo "FAIL: iOS smoke profile must use only the local Blossom fixture." >&2
    return 1
  }
}

assert_ios_smoke_blossom_handoff() {
  python3 - "$1" "$IOS_SMOKE_BLOSSOM_STATE/blobs" "$IOS_SMOKE_BLOSSOM_URL" <<'PY'
import hashlib, json, re, sys, urllib.request
from pathlib import Path
approval = json.loads(sys.argv[1])
upload = approval.get("blossom_upload") or {}
total = int(upload.get("total_hashes") or 0)
assert approval.get("approval_publish_error") is None, "approval publication failed"
assert approval.get("published_drive_root") is True and approval.get("root_cid"), "Drive root not published"
assert approval.get("published_approval_events") == 6, "incomplete initial approval event publication"
assert total > 0 and upload.get("uploaded", -1) >= 0 and upload.get("already_present", -1) >= 0
assert upload["uploaded"] + upload["already_present"] == total, "incomplete root upload"
blobs = list(Path(sys.argv[2]).glob("*.bin"))
# Cid's production string format is hash[:key]; bind the announced root to
# its downloaded ciphertext rather than inferring identity from a blob count.
root_hash = str(approval["root_cid"]).split(":", 1)[0]
assert re.fullmatch(r"[0-9a-f]{64}", root_hash), "invalid published root hash"
assert any(blob.stem == root_hash for blob in blobs), "published root absent from local Blossom"
assert len(blobs) >= total, "fewer stored blocks than the upload report"
for blob in blobs:
    with urllib.request.urlopen(sys.argv[3] + "/" + blob.name, timeout=5) as response:
        data = response.read()
    assert data == blob.read_bytes() and hashlib.sha256(data).hexdigest() == blob.stem, "unreadable approval block"
print("IOS_LOCAL_BLOSSOM_HANDOFF_OK events=6 published_root_verified=true reported_root_blocks=" + str(total) + " verified_fixture_blocks=" + str(len(blobs)))
PY
}

configure_ios_smoke_relay() {
  local config_dir="$1" relay
  [[ -n "$LOCAL_RELAY_URL" ]] || return 1
  while IFS= read -r relay; do
    [[ -z "$relay" ]] || "$IDRIVE" --config-dir "$config_dir" relays remove "$relay" >/dev/null
  done < <("$IDRIVE" --config-dir "$config_dir" relays list | python3 -c 'import json,sys; print("\n".join(json.load(sys.stdin)))')
  "$IDRIVE" --config-dir "$config_dir" relays add "$LOCAL_RELAY_URL" \
    | python3 -c 'import json,sys; raise SystemExit(0 if json.load(sys.stdin) == [sys.argv[1]] else 1)' "$LOCAL_RELAY_URL"
}
