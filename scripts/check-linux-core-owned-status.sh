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
require_contains linux/src/setup.rs 'iris_drive_app_core::validate_link_input(value.to_string()).is_complete'
require_contains linux/src/main.rs 'NativeAppAction, NativeAppState, UiState'
require_contains linux/src/daemon_control.rs 'pub(crate) fn desktop_state() -> Result<NativeAppState, String>'
require_contains linux/src/daemon_control.rs 'pub(crate) fn dispatch_desktop_action(action: NativeAppAction)'
require_contains linux/src/data.rs 'state.ui.awaiting_approval'
require_contains linux/src/data.rs 'state.ui.revoked'
require_contains linux/src/data.rs 'state.ui.setup_label'
require_contains linux/src/data.rs 'state.ui.file_count'
require_contains linux/src/data.rs 'state.ui.visible_file_bytes'
require_contains linux/src/data.rs 'state.ui.online_app_key_count'
require_contains linux/src/data.rs 'state.ui.authorized_app_key_count'
require_contains linux/src/data.rs 'state.ui.primary_status_label'
require_contains linux/src/render.rs 'actor.display_label'
require_contains linux/src/render.rs 'actor.role_label'
require_contains linux/src/render.rs 'actor.connection_label'
require_contains linux/src/render.rs 'actor.can_appoint_admin'
require_contains linux/src/render.rs 'fips.state_label'
require_contains linux/src/render.rs 'fips.roster_label'
require_contains linux/src/render.rs 'fips.direct_device_count'
require_contains linux/src/render.rs 'render_relay_statuses(relays_list, &state.ui)'

require_absent linux/src/setup.rs "fn is_complete_link_owner_input"
require_absent linux/src/data.rs 'summary_json'
require_absent linux/src/render.rs 'find_string('
require_absent linux/src/render.rs 'find_bool('
require_absent linux/src/render.rs 'find_number('
require_absent linux/src/render.rs 'get("account")'
require_absent linux/src/render.rs '"has_owner_signing_authority"'
require_absent linux/src/render.rs '"label", "app_key_npub", "device_pubkey"'
require_absent linux/src/render.rs 'admin_count'
require_absent linux/src/data.rs '"authorization_state"'
require_absent linux/src/refresh.rs '"authorization_state"'
require_absent linux/src/data.rs '"local_block_bytes"'
require_absent linux/src/data.rs '"published_app_key_roots"'
require_absent linux/src/data.rs '"roster_connected_peer_count"'
require_absent linux/src/render.rs "pub(crate) fn fips_state_text"
require_absent linux/src/render.rs "fn fips_connection_label"
require_absent linux/src/render.rs "fn fips_peer_status_label"
require_absent linux/src/render.rs "pub(crate) fn relay_status"
require_absent linux/src/render.rs '"roster_connected_peer_count"'
require_absent linux/src/render.rs '"connected_peer_count"'

echo "LINUX_CORE_OWNED_STATUS_OK"
