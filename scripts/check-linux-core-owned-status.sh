#!/usr/bin/env bash

set -Eeuo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"

require_contains() {
  local file="$1"
  local needle="$2"
  if ! grep -Fq "$needle" "$ROOT/$file"; then
    echo "missing '$needle' in $file" >&2
    exit 1
  fi
}

require_absent() {
  local file="$1"
  local needle="$2"
  if grep -Fq "$needle" "$ROOT/$file"; then
    echo "unexpected '$needle' in $file" >&2
    exit 1
  fi
}

require_contains linux/Cargo.toml 'iris-drive-app-core = { path = "../crates/iris-drive-app-core" }'
require_contains linux/src/setup.rs 'iris_drive_app_core::classify_link_input(value.to_string()).is_complete'
require_contains linux/src/data.rs 'summary_json(json)'
require_contains linux/src/data.rs 'find_number(summary_json(json), &["file_count"])'
require_contains linux/src/data.rs 'find_number(summary_json(json), &["visible_file_bytes"])'
require_contains linux/src/data.rs 'find_number(summary_json(json), &["online_device_count"])'
require_contains linux/src/data.rs 'find_string(summary_json(json), &["primary_status_label"])'
require_contains linux/src/render.rs 'find_string(peer, &["display_label"])'
require_contains linux/src/render.rs 'find_string(peer, &["role_label"])'
require_contains linux/src/render.rs 'find_string(peer, &["connection_label"])'
require_contains linux/src/render.rs 'find_bool(peer, &["can_appoint_admin"])'
require_contains linux/src/render.rs 'find_string(fips, &["state_label"])'
require_contains linux/src/render.rs 'find_string(fips, &["roster_label"])'
require_contains linux/src/render.rs 'find_string(peer, &["connection_label"])'
require_contains linux/src/render.rs 'render_relay_statuses(relays_list, network)'

require_absent linux/src/setup.rs "fn is_complete_link_owner_input"
require_absent linux/src/render.rs 'get("account")'
require_absent linux/src/render.rs '"has_owner_signing_authority"'
require_absent linux/src/render.rs '"label", "device_npub", "device_pubkey"'
require_absent linux/src/render.rs 'admin_count'
require_absent linux/src/data.rs '"local_block_bytes"'
require_absent linux/src/data.rs '"published_device_roots"'
require_absent linux/src/data.rs '"roster_connected_peer_count"'
require_absent linux/src/render.rs "pub(crate) fn fips_state_text"
require_absent linux/src/render.rs "fn fips_connection_label"
require_absent linux/src/render.rs "fn fips_peer_status_label"
require_absent linux/src/render.rs "pub(crate) fn relay_status"
require_absent linux/src/render.rs '"roster_connected_peer_count"'

echo "LINUX_CORE_OWNED_STATUS_OK"
