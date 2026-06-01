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
require_contains crates/iris-drive-core/src/device_summary.rs "pub fn primary_status_for_setup_state"
require_contains crates/iris-drive-app-core/src/ffi.rs "use iris_drive_core::device_summary"
require_contains crates/iris-drive-cli/src/status/peers.rs "use iris_drive_core::device_summary"

require_absent crates/iris-drive-app-core/src/ffi.rs "fn device_connection_label("
require_absent crates/iris-drive-app-core/src/ffi.rs "fn device_connection_state("
require_absent crates/iris-drive-cli/src/status/peers.rs "fn peer_connection_label("
require_absent crates/iris-drive-cli/src/status/peers.rs "fn peer_connection_state("
require_absent crates/iris-drive-cli/src/status/peers.rs "fn peer_display_label("
require_absent macos/Sources/IrisDriveStatus.swift "private static func connectionLabel"
require_absent macos/Sources/IrisDriveStatus.swift "private static func connectionState"
require_absent macos/Sources/IrisDriveStatus.swift "private static func roleLabel"
require_absent macos/Sources/IrisDriveStatus.swift "private static func displayLabel"
require_absent android/app/src/main/java/to/iris/drive/app/core/AppState.kt "connectionLabelFor"
require_absent android/app/src/main/java/to/iris/drive/app/core/AppState.kt "optNonBlankString"

echo "DEVICE_SUMMARY_OWNERSHIP_OK"
