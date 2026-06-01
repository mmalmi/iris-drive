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

require_contains crates/iris-drive-core/src/device_summary.rs "pub fn device_connection_label"
require_contains crates/iris-drive-core/src/device_summary.rs "pub fn device_display_label"
require_contains crates/iris-drive-core/src/device_summary.rs "pub fn device_management_actions"
require_contains crates/iris-drive-core/src/device_summary.rs "pub fn primary_status_for_setup_state"
require_contains crates/iris-drive-app-core/src/ffi.rs "use iris_drive_core::device_summary"
require_contains crates/iris-drive-cli/src/status/peers.rs "use iris_drive_core::device_summary"
require_contains crates/iris-drive-cli/src/status/peers.rs '"can_revoke": actions.can_revoke'
require_contains crates/iris-drive-cli/src/status/peers.rs '"can_appoint_admin": actions.can_appoint_admin'
require_contains crates/iris-drive-cli/src/status/peers.rs '"can_demote_admin": actions.can_demote_admin'
require_contains macos/Sources/IrisDriveStatus.swift 'json["display_label"]'
require_contains macos/Sources/IrisDriveStatus.swift 'json["role_label"]'
require_contains macos/Sources/IrisDriveStatus.swift 'json["connection_state"]'
require_contains macos/Sources/IrisDriveStatus.swift 'json["connection_label"]'
require_contains windows/IrisDriveModels.cs '"can_revoke"'
require_contains windows/IrisDriveModels.cs '"can_appoint_admin"'
require_contains windows/IrisDriveModels.cs '"can_demote_admin"'
require_contains windows/IrisDriveModels.cs 'var title = String(peer, "display_label") ?? "";'

require_absent crates/iris-drive-app-core/src/ffi.rs "fn device_connection_label("
require_absent crates/iris-drive-app-core/src/ffi.rs "fn device_connection_state("
require_absent crates/iris-drive-cli/src/status/peers.rs "fn peer_connection_label("
require_absent crates/iris-drive-cli/src/status/peers.rs "fn peer_connection_state("
require_absent crates/iris-drive-cli/src/status/peers.rs "fn peer_display_label("
require_absent macos/Sources/IrisDriveStatus.swift "private static func connectionLabel"
require_absent macos/Sources/IrisDriveStatus.swift "private static func connectionState"
require_absent macos/Sources/IrisDriveStatus.swift "private static func roleLabel"
require_absent macos/Sources/IrisDriveStatus.swift "private static func displayLabel"
require_absent macos/Sources/IrisDriveStatus.swift '"fips_online_via"'
require_absent macos/Sources/IrisDriveStatus.swift '"fips_transport_type"'
require_absent macos/Sources/IrisDriveStatus.swift '"fips_srtt_ms"'
require_absent windows/IrisDriveModels.cs 'var adminCount'
require_absent windows/IrisDriveModels.cs '(isOnline ? "Online" : "Offline")'
require_absent windows/IrisDriveModels.cs 'String(peer, "connection_label") ?? "Online"'
require_absent windows/IrisDriveModels.cs 'String(peer, "role_label") ?? role'
require_absent windows/IrisDriveModels.cs '?? "Device"'
require_absent windows/IrisDriveModels.cs 'network.HasValue ? Int(network.Value, "authorized_device_count") : 0'
require_absent windows/IrisDriveModels.cs 'hashtree.HasValue ? Int(hashtree.Value, "file_count") : 0'
require_absent windows/IrisDriveModels.cs 'hashtree.HasValue ? Long(hashtree.Value, "visible_file_bytes") : 0'
require_absent android/app/src/main/java/to/iris/drive/app/core/AppState.kt "connectionLabelFor"
require_absent android/app/src/main/java/to/iris/drive/app/core/AppState.kt "optNonBlankString"

echo "DEVICE_SUMMARY_OWNERSHIP_OK"
