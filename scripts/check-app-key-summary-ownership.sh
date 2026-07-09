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

require_contains crates/iris-drive-core/src/app_key_summary.rs "pub fn app_key_connection_label"
require_contains crates/iris-drive-core/src/app_key_summary.rs "pub fn app_key_display_label"
require_contains crates/iris-drive-core/src/app_key_summary.rs "pub fn app_key_management_actions"
require_contains crates/iris-drive-core/src/app_key_summary.rs "pub fn app_key_roster_rows"
require_contains crates/iris-drive-core/src/app_key_summary.rs "pub fn primary_status_for_setup_state"
require_contains crates/iris-drive-core/src/app_key_summary.rs "pub fn setup_state_flags"
require_contains crates/iris-drive-app-core/src/ffi.rs "use iris_drive_core::app_key_summary"
require_contains crates/iris-drive-app-core/src/state.rs "pub setup_complete: bool"
require_contains crates/iris-drive-app-core/src/state.rs "pub awaiting_approval: bool"
require_contains crates/iris-drive-app-core/src/state.rs "pub revoked: bool"
require_contains crates/iris-drive-cli/src/status.rs '"setup_complete": setup_flags.setup_complete'
require_contains crates/iris-drive-cli/src/status.rs '"awaiting_approval": setup_flags.awaiting_approval'
require_contains crates/iris-drive-cli/src/status.rs '"revoked": setup_flags.revoked'
require_contains crates/iris-drive-cli/src/status/peers.rs "use iris_drive_core::app_key_summary"
require_contains crates/iris-drive-cli/src/status/peers.rs "app_key_roster_rows("
require_contains crates/iris-drive-cli/src/status/peers.rs '"can_revoke": app_key.can_revoke'
require_contains crates/iris-drive-cli/src/status/peers.rs '"can_appoint_admin": app_key.can_appoint_admin'
require_contains crates/iris-drive-cli/src/status/peers.rs '"can_demote_admin": app_key.can_demote_admin'
require_contains crates/iris-drive-cli/src/status/peers.rs '"detail": detail'
require_contains crates/iris-drive-app-core/src/ffi/app_key_link_flow_tests.rs "revoked_current_device_refresh_logs_out_and_allows_fresh_relink"
require_contains macos/Sources/IrisDriveStatus.swift 'json["display_label"]'
require_contains macos/Sources/IrisDriveStatus.swift '@Published var setupComplete = false'
require_contains macos/Sources/IrisDriveStatus.swift '@Published var awaitingApproval = false'
require_contains macos/Sources/IrisDriveStatus.swift '@Published var revoked = false'
require_contains macos/Sources/IrisDriveMacApp.swift 'summary["setup_complete"] as? Bool'
require_contains macos/Sources/IrisDriveMacApp.swift 'status.revoked = ui["revoked"] as? Bool ?? false'
require_contains macos/Sources/IrisDriveControlPanel.swift 'RevokedDeviceSetupView(status: status, controller: controller)'
require_contains macos/Sources/IrisDriveStatus.swift 'json["role_label"]'
require_contains macos/Sources/IrisDriveStatus.swift 'json["connection_state"]'
require_contains macos/Sources/IrisDriveStatus.swift 'json["connection_label"]'
require_contains windows/IrisDriveModels.cs '"can_revoke"'
require_contains windows/IrisDriveModels.cs '"can_appoint_admin"'
require_contains windows/IrisDriveModels.cs '"can_demote_admin"'
require_contains windows/IrisDriveModels.cs 'SetupComplete = setupComplete'
require_contains windows/IrisDriveModels.cs 'AwaitingApproval = Bool(ui, "awaiting_approval")'
require_contains windows/IrisDriveModels.cs 'Revoked = Bool(ui, "revoked")'
require_contains windows/IrisDriveModels.cs 'ui.ValueKind == JsonValueKind.Object && Bool(ui, "setup_complete")'
require_contains windows/IrisDriveModels.cs 'String(device, "display_label") ?? ""'
require_contains windows/IrisDriveModels.cs 'String(device, "detail")'
require_contains windows/MainWindow.xaml.cs 'RenderRevokedDevice(status'
require_contains ios/Sources/IrisDriveNativeCore.swift "var setupComplete: Bool"
require_contains ios/Sources/IrisDriveNativeCore.swift 'case setupComplete = "setup_complete"'
require_contains ios/Sources/IrisDriveMobileModel.swift "lastState?.ui.setupComplete"
require_contains ios/Sources/IrisDriveMobileModel.swift "lastState?.ui.revoked"
require_contains ios/Sources/IrisDriveRootView.swift "RevokedDeviceSetupView(model: model)"
require_contains android/app/src/main/java/to/iris/drive/app/core/AppState.kt 'val isSetupComplete: Boolean = false'
require_contains android/app/src/main/java/to/iris/drive/app/core/AppState.kt 'isSetupComplete = ui.optBoolean("setup_complete")'
require_contains android/app/src/main/java/to/iris/drive/app/core/AppState.kt 'isRevoked = ui.optBoolean("revoked")'
require_contains android/app/src/main/java/to/iris/drive/app/IrisDriveAndroidApp.kt 'RevokedDeviceContent('
require_contains linux/src/data.rs 'state.ui.awaiting_approval'
require_contains linux/src/data.rs 'state.ui.revoked'
require_contains linux/src/setup.rs 'render_revoked_device'

require_absent crates/iris-drive-app-core/src/ffi.rs "fn app_key_connection_label("
require_absent crates/iris-drive-app-core/src/ffi.rs "fn app_key_connection_state("
require_absent crates/iris-drive-app-core/src/ffi.rs "fn refresh_device_actions("
require_absent crates/iris-drive-cli/src/status/peers.rs "fn peer_connection_label("
require_absent crates/iris-drive-cli/src/status/peers.rs "fn peer_connection_state("
require_absent crates/iris-drive-cli/src/status/peers.rs "fn peer_display_label("
require_absent macos/Sources/IrisDriveStatus.swift "private static func connectionLabel"
require_absent macos/Sources/IrisDriveStatus.swift "private static func connectionState"
require_absent macos/Sources/IrisDriveStatus.swift "private static func roleLabel"
require_absent macos/Sources/IrisDriveStatus.swift "private static func displayLabel"
require_absent macos/Sources/IrisDriveStatus.swift 'setupState == "authorized"'
require_absent macos/Sources/IrisDriveStatus.swift 'setupState == "awaiting_approval"'
require_absent macos/Sources/IrisDriveStatus.swift 'setupState == "revoked"'
require_absent macos/Sources/IrisDriveStatus.swift '"fips_online_via"'
require_absent macos/Sources/IrisDriveStatus.swift '"fips_transport_type"'
require_absent macos/Sources/IrisDriveStatus.swift '"fips_srtt_ms"'
require_absent windows/IrisDriveModels.cs 'var adminCount'
require_absent windows/IrisDriveModels.cs '(isOnline ? "Online" : "Offline")'
require_absent windows/IrisDriveModels.cs 'String(peer, "connection_label") ?? "Online"'
require_absent windows/IrisDriveModels.cs 'String(peer, "role_label") ?? role'
require_absent windows/IrisDriveModels.cs '?? "Device"'
require_absent windows/IrisDriveModels.cs 'network.HasValue ? Int(network.Value, "authorized_device_count") : 0'
require_absent windows/IrisDriveModels.cs 'string.Equals(SetupState, "authorized"'
require_absent windows/IrisDriveModels.cs 'string.Equals(SetupState, "awaiting_approval"'
require_absent windows/IrisDriveModels.cs 'string.Equals(SetupState, "revoked"'
require_absent windows/IrisDriveModels.cs '"authorized",'
require_absent windows/IrisDriveModels.cs 'public static IrisDriveStatusData FromJson'
require_absent windows/IrisDriveModels.cs 'summary.HasValue'
require_absent windows/IrisDriveModels.cs 'hashtree.HasValue ? Int(hashtree.Value, "file_count") : 0'
require_absent windows/IrisDriveModels.cs 'hashtree.HasValue ? Long(hashtree.Value, "visible_file_bytes") : 0'
require_absent windows/IrisDriveModels.cs 'var details = new List<string>()'
require_absent windows/IrisDriveModels.cs 'details.Add('
require_absent windows/IrisDriveModels.cs 'Object(peer, "last_block_sync")'
require_absent windows/IrisDriveModels.cs 'String(peer, "sync_state")'
require_absent windows/IrisDriveModels.cs 'Int(peer, "dck_generation")'
require_absent android/app/src/main/java/to/iris/drive/app/core/AppState.kt "connectionLabelFor"
require_absent android/app/src/main/java/to/iris/drive/app/core/AppState.kt "optNonBlankString"
require_absent android/app/src/main/java/to/iris/drive/app/core/AppState.kt 'get() = setupState == "authorized"'
require_absent android/app/src/main/java/to/iris/drive/app/core/AppState.kt 'get() = setupState == "awaiting_approval"'
require_absent android/app/src/main/java/to/iris/drive/app/core/AppState.kt 'get() = setupState == "revoked"'

echo "APP_KEY_SUMMARY_OWNERSHIP_OK"
