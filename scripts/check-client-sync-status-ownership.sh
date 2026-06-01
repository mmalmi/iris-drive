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

require_contains crates/iris-drive-core/src/device_summary.rs "pub fn sync_status_label"
require_contains crates/iris-drive-app-core/src/state.rs "pub status_label: String"
require_contains crates/iris-drive-app-core/src/ffi.rs "sync_status_label(status)"
require_contains crates/iris-drive-cli/src/daemon/runtime.rs "normalize_daemon_status_for_clients(config_dir, &mut payload)"
require_contains crates/iris-drive-cli/src/status.rs '"sync_status": sync_status'
require_contains crates/iris-drive-cli/src/status.rs '"sync_status_label": sync_status_label(sync_status)'

require_contains android/app/src/main/java/to/iris/drive/app/core/AppState.kt "val statusLabel: String = \"Sync paused\""
require_contains android/app/src/main/java/to/iris/drive/app/core/AppState.kt 'statusLabel = optString("status_label", "Sync paused")'
require_contains android/app/src/main/java/to/iris/drive/app/IrisDriveMainContent.kt "state.sync.statusLabel"
require_absent android/app/src/main/java/to/iris/drive/app/IrisDriveMainContent.kt "state.sync.status.ifBlank"

require_contains ios/Sources/IrisDriveNativeCore.swift "var statusLabel: String"
require_contains ios/Sources/IrisDriveNativeCore.swift 'case statusLabel = "status_label"'
require_contains ios/Sources/IrisDriveMobileModel.swift "lastState?.ui.sync.statusLabel"
require_contains ios/Sources/IrisDriveMobileModel.swift "statusDetail = state.error.isEmpty ? state.ui.sync.statusLabel : state.error"
require_absent ios/Sources/IrisDriveMobileModel.swift 'syncRunning ? "Sync on" : "Sync paused"'

require_contains macos/Sources/IrisDriveStatus.swift '@Published var syncStatusLabel = "Sync paused"'
require_contains macos/Sources/IrisDriveMacApp.swift 'status.syncStatusLabel = syncStatusLabel'
require_contains macos/Sources/IrisDriveMacApp.swift 'json["sync"] as? [String: Any]'
require_contains macos/Sources/IrisDriveControlPanel.swift "status.syncStatusLabel"
require_absent macos/Sources/IrisDriveControlPanel.swift 'return "Up to date"'
require_absent macos/Sources/IrisDriveControlPanel.swift 'return "Paused"'
require_absent macos/Sources/IrisDriveControlPanel.swift 'return "Sync failed"'

echo "CLIENT_SYNC_STATUS_OWNERSHIP_OK"
