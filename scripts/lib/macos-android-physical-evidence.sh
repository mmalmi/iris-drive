#!/usr/bin/env bash
# Secret-safe evidence helpers for the physical macOS <-> Android gate.

write_relay_summary() {
  [[ -n "$TMP" ]] || return 0
  python3 - "$TMP" "${ANDROID_OWNER_NPUB:-}" "${MAC_JOINER_NPUB:-}" \
    >"$RESULT_DIR/relay-summary.json" <<'PY'
import collections, json, pathlib, sys
paths=sorted(pathlib.Path(sys.argv[1]).glob("local-relay-events-*.jsonl"))
charset="qpzry9x8gf2tvdw0s3jn54khce6mua7l"
def npub_hex(value):
    if not value.startswith("npub1"): return None
    data=[charset.index(c) for c in value[value.rfind("1")+1:-6]]
    acc=bits=0; out=[]
    for item in data:
        acc=(acc << 5) | item; bits += 5
        while bits >= 8:
            bits -= 8; out.append((acc >> bits) & 255)
    return bytes(out).hex() if len(out) == 32 else None
expected={value for value in (npub_hex(sys.argv[2]), npub_hex(sys.argv[3])) if value}
expected_roles={
    role: value for role, value in (
        ("android_owner", npub_hex(sys.argv[2])),
        ("macos_joiner", npub_hex(sys.argv[3])),
    ) if value
}
def advert_capability(event):
    try: endpoints=json.loads(event.get("content") or "{}").get("endpoints") or []
    except (TypeError, json.JSONDecodeError): return "invalid"
    if not endpoints: return "invalid"
    return "inbound_routable" if any(
        str(item.get("transport") or "").lower() != "udp"
        or str(item.get("addr") or "").strip().lower() != "nat"
        for item in endpoints if isinstance(item, dict)
    ) else "outbound_only"
epochs=[]
events=[]
for epoch, path in enumerate(paths, start=1):
    epoch_events=[json.loads(line) for line in path.read_text(encoding="utf-8").splitlines() if line]
    advert_authors={event.get("pubkey") for event in epoch_events if event.get("kind")==37195}
    epochs.append({
        "epoch": epoch,
        "event_count": len(epoch_events),
        "event_kinds": dict(sorted(collections.Counter(str(event.get("kind")) for event in epoch_events).items())),
        "fips_advert_count": sum(event.get("kind")==37195 for event in epoch_events),
        "expected_reverse_adverts_present": bool(expected) and expected.issubset(advert_authors),
    })
    events.extend((epoch, event) for event in epoch_events)
if not events:
    raise SystemExit(0)
latest_expected_adverts={}
for _, event in events:
    if event.get("kind") == 37195 and event.get("pubkey") in expected:
        latest_expected_adverts[event.get("pubkey")]=event
print(json.dumps({
    "event_count": len(events),
    "event_kinds": dict(sorted(collections.Counter(str(event.get("kind")) for _, event in events).items())),
    "epochs": epochs,
    "last_sequence": max((int(event.get("sequence") or 0) for _, event in events), default=0),
    "drive_root_events": [{
        "epoch": epoch,
        "sequence": int(event.get("sequence") or 0),
        "event_id": str(event.get("id") or "")[:16],
        "created_at": event.get("created_at"),
        "received_at": event.get("received_at"),
    } for epoch, event in events if event.get("kind") == 30078],
    "expected_reverse_advert_capabilities": {
        role: advert_capability(latest_expected_adverts.get(author, {}))
        for role, author in expected_roles.items()
    },
    "transport": "loopback fixture via adb reverse and SSH remote forward",
}, indent=2, sort_keys=True))
PY
}

write_android_evidence() {
  local role="$1" peer="$2" destination="$3"
  android_debug_action refresh >/dev/null 2>&1 || true
  copy_android_file config.toml "$TMP/android-config.toml"
  copy_android_file native-fips-status.json "$TMP/android-fips.json"
  python3 - "$TMP/android-config.toml" "$TMP/android-fips.json" \
    "$role" "$peer" >"$destination" <<'PY'
import json, re, sys
config=open(sys.argv[1], encoding="utf-8").read()
fips=json.load(open(sys.argv[2], encoding="utf-8")); peer=sys.argv[4]
state=re.search(r'^authorization_state\s*=\s*"([^"]+)"\s*$', config, re.MULTILINE)
online=peer in (fips.get("online_devices") or []) or peer in (fips.get("online_peers") or [])
direct=peer in (fips.get("direct_devices") or []) or peer in (fips.get("direct_peers") or [])
mesh=peer in (fips.get("mesh_devices") or []) or peer in (fips.get("mesh_peers") or [])
peer_status=next((status for status in fips.get("peer_statuses") or [] if status.get("npub")==peer), None)
print(json.dumps({
    "role": sys.argv[3],
    "authorization_state": state.group(1) if state else None,
    "pending_device_approval_receipts_zero": "pending_device_approval_receipts" not in config,
    "fips": {
        "running": fips.get("running") is True,
        "connected_peer_count": int(fips.get("connected_peer_count") or 0),
        "authorized_peer_count": int(fips.get("authorized_peer_count") or 0),
        "expected_peer_authorized": peer in (fips.get("authorized_peers") or []),
        "expected_peer_online": online,
        "expected_peer_direct": direct,
        "expected_peer_mesh": mesh,
        "expected_peer_status": None if peer_status is None else {
            key: peer_status.get(key) for key in (
                "connected", "transport_type", "srtt_ms", "packets_sent", "packets_recv",
            )
        },
    },
}, indent=2, sort_keys=True))
PY
}

write_android_root_sync_evidence() {
  local path="$1" expected_size="$2" destination="$3" block_count
  android_debug_action dump-provider-list >/dev/null 2>&1 || true
  for _ in $(seq 1 30); do
    copy_android_file debug-provider-list.json "$TMP/android-provider-list.json" && break
    sleep 0.1
  done
  copy_android_file config.toml "$TMP/android-config.toml" || return 0
  [[ -s "$TMP/android-provider-list.json" ]] || return 0
  block_count="$("$ADB" -s "$SERIAL" shell run-as "$PACKAGE" \
    sh -c 'find files -type f 2>/dev/null | wc -l' 2>/dev/null | tr -d '\r ' || true)"
  python3 - "$TMP/android-provider-list.json" "$TMP/android-config.toml" \
    "$path" "$expected_size" "${block_count:-0}" >"$destination" <<'PY'
import hashlib, json, re, sys
provider=json.load(open(sys.argv[1], encoding="utf-8"))
config=open(sys.argv[2], encoding="utf-8").read()
entry=next((item for item in provider.get("entries", []) if item.get("path")==sys.argv[3]), None)
def identity(value):
    return hashlib.sha256(str(value).encode()).hexdigest()[:16] if value else None
print(json.dumps({
    "expected_size": int(sys.argv[4]),
    "entry": None if entry is None else {
        "kind": entry.get("kind"),
        "size": entry.get("size"),
        "version_identity": identity(entry.get("version")),
    },
    "provider": {
        "file_count": provider.get("file_count"),
        "visible_file_bytes": provider.get("visible_file_bytes"),
        "root_identity": identity(provider.get("root_cid")),
        "anchor_identity": identity(provider.get("anchor")),
        "error_class": str(provider.get("error") or "").split(":", 1)[0],
    },
    "configured_root_identities": [
        identity(root) for root in re.findall(r'root_cid\s*=\s*"([^"]+)"', config)
    ],
    "private_file_count": int(sys.argv[5]) if sys.argv[5].isdigit() else None,
}, indent=2, sort_keys=True))
PY
}
