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

require_missing_file() {
  local file="$1"
  if [[ -e "$ROOT/$file" ]]; then
    echo "unexpected file $file" >&2
    exit 1
  fi
}

require_contains crates/iris-drive-core/src/lib.rs "pub mod fips_status;"
require_contains crates/iris-drive-core/src/fips_status.rs "pub fn normalize_fips_status_value"
require_contains crates/iris-drive-core/src/fips_status.rs "pub fn fips_online_devices_from_status"
require_contains crates/iris-drive-core/src/fips_status.rs "pub fn online_device_ids"

require_contains crates/iris-drive-cli/src/status/network.rs "normalize_fips_status_value("
require_absent crates/iris-drive-cli/src/status/network.rs "fn fips_state_label"
require_absent crates/iris-drive-cli/src/status/network.rs "fn fips_peer_connection_label"
require_absent crates/iris-drive-cli/src/status/network.rs "fn normalized_fips_peer_statuses"

require_contains crates/iris-drive-app-core/src/ffi.rs "normalize_fips_status_value("
require_contains crates/iris-drive-app-core/src/ffi.rs "online_device_ids("
require_contains crates/iris-drive-app-core/src/state.rs "pub roster_label: String"
require_contains crates/iris-drive-app-core/src/state.rs "pub roster_online_device_count: u64"
require_contains crates/iris-drive-app-core/src/state.rs "pub peer_statuses: Vec<UiFipsPeerStatus>"
require_contains crates/iris-drive-app-core/src/ffi.rs "peer_statuses: ui_fips_peer_statuses("
require_absent crates/iris-drive-app-core/src/ffi.rs "struct NativeFipsStatus"
require_absent crates/iris-drive-app-core/src/ffi.rs "fn native_fips_state_label"
require_absent crates/iris-drive-app-core/src/lib.rs "mod native_fips;"
require_missing_file crates/iris-drive-app-core/src/native_fips.rs

require_contains windows/IrisDriveModels.cs 'Object(ui, "fips")'
require_contains windows/IrisDriveModels.cs '"visible_file_bytes"'
require_contains windows/IrisDriveModels.cs '"connection_label"'
require_contains windows/IrisDriveModels.cs '"state_label"'
require_contains windows/IrisDriveModels.cs '"roster_label"'
require_contains windows/IrisDriveModels.cs '"roster_online_device_count"'
require_contains windows/IrisDriveModels.cs '"direct_device_count"'
require_contains windows/IrisDriveModels.cs '"mesh_device_count"'
require_absent windows/IrisDriveModels.cs "FipsConnectionLabel"
require_absent windows/IrisDriveModels.cs "FipsPeerStatusLabel"
require_absent windows/IrisDriveModels.cs '"roster_connected_peer_count"'
require_absent windows/IrisDriveModels.cs '"connected_peer_count"'
require_absent windows/IrisDriveModels.cs '"fips_online_via"'
require_absent windows/IrisDriveModels.cs '"fips_transport_type"'
require_absent windows/IrisDriveModels.cs '"fips_srtt_ms"'

require_contains ios/Sources/IrisDriveNativeCore.swift "var rosterLabel: String"
require_contains ios/Sources/IrisDriveNativeCore.swift 'case peerStatuses = "peer_statuses"'
require_contains android/app/src/main/java/to/iris/drive/app/core/AppState.kt "val fips: FipsState"
require_contains android/app/src/main/java/to/iris/drive/app/core/AppState.kt "rosterOnlineDeviceCount = optInt(\"roster_online_device_count\")"
require_contains android/app/src/main/java/to/iris/drive/app/core/AppState.kt "connectionLabel = item.optString(\"connection_label\", \"Online\")"

echo "FIPS_STATUS_NORMALIZATION_OK"
