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

require_contains linux/src/daemon_control.rs "iris_drive_app_core::FfiApp"
require_contains linux/src/daemon_control.rs "dispatch_desktop_action"
require_contains linux/src/actions.rs "Reject"
require_contains linux/src/render.rs "state.ui.app_actors"
require_contains linux/src/render.rs "share.source_path"
require_contains linux/src/render.rs "missing access wrap"
require_contains linux/src/render.rs "pending_invites"
require_contains linux/src/actions.rs "RecordPendingShareInvite"
require_contains linux/src/data.rs "authorized_app_key_count"
require_contains linux/src/main.rs "ApplicationFlags::HANDLES_OPEN"
require_contains linux/src/main.rs "classify_link_input"
require_contains linux/src/main.rs "apply_share_dialog_link"
require_contains linux/src/main.rs "open_content_link"
require_contains linux/src/main.rs "ImportContentLink"
require_contains linux/src/ui.rs "stack,"
require_absent linux/src/render.rs "state.ui.devices"
require_absent linux/src/data.rs "authorized_device_count"
require_absent linux/src/render.rs "missing_key_wraps.join"
require_absent linux/src/setup.rs 'run_idrive(["revoke", device])'
require_absent linux/src/setup.rs 'run_idrive(["devices", "appoint-admin", device])'
require_absent linux/src/setup.rs 'run_idrive(["devices", "demote-admin", device])'

require_contains macos/Sources/IrisDriveDesktopCore.swift "final class IrisDriveDesktopCore"
require_contains macos/Sources/IrisDriveMacApp.swift "desktopCore.refreshJson()"
require_contains macos/Sources/IrisDriveMacApp.swift "applyNativeStatePayload"
require_contains macos/Sources/IrisDriveMacApp.swift "openContentLink"
require_contains macos/Sources/IrisDriveContentLinks.swift '"import_content_link"'
require_contains macos/Sources/IrisDriveMacApp.swift '"record_pending_share_invite"'
require_contains macos/Sources/IrisDriveControlPanel.swift "Reject"
require_contains macos/Sources/IrisDriveControlPanel.swift "peer.isCurrentDevice || peer.fipsOnline"
require_contains macos/Sources/IrisDriveControlPanel.swift 'Label(approvalPending ? "Adding" : "Add", systemImage: "checkmark")'
require_contains macos/Sources/IrisDriveControlPanel.swift "pendingInvites"
require_contains scripts/macos-dev-app.sh "cargo build -p iris-drive-app-core"
require_contains scripts/macos-dev-app.sh "libiris_drive_app_core.a"
require_contains scripts/local-release.mjs "iris-drive-app-core"
require_contains scripts/local-release.mjs "libiris_drive_app_core.a"
require_contains scripts/dev-vm-update-run.sh "iris-drive-app-core"
require_contains scripts/dev-vm-update-run.sh "libiris_drive_app_core.a"
require_absent macos/Shared/IrisDriveRuntimeSupport.swift "statusPayload"
require_absent macos/Sources/IrisDriveMacApp.swift 'arguments: ["approve", device]'
require_absent macos/Sources/IrisDriveMacApp.swift 'arguments: ["devices", command, device]'

require_contains windows/IrisDriveNativeCore.cs "iris_drive_app_dispatch_json"
require_contains windows/IrisDriveService.cs "RunNativeCoreAsync"
require_contains windows/IrisDriveService.cs "core.RefreshJson()"
require_contains windows/IrisDriveServiceNativeCore.cs "DispatchNativeActionAsync"
require_contains windows/IrisDriveService.cs '["type"] = "create_share"'
require_contains windows/IrisDriveService.cs '["type"] = "invite_share_member_from_evidence"'
require_contains windows/IrisDriveService.cs '["type"] = "record_pending_share_invite"'
require_contains windows/IrisDriveService.cs '["type"] = "add_share_shortcut"'
require_contains windows/IrisDriveService.cs '["type"] = "repair_share_wraps"'
require_contains windows/IrisDriveService.cs '["type"] = "revoke_share_member"'
require_contains windows/IrisDriveService.cs '["type"] = "set_share_member_role"'
require_contains windows/IrisDriveModels.cs "FromNativeJson"
require_contains windows/IrisDriveModels.cs "NativeShareRows"
require_contains windows/MainWindow.xaml "NavSharesButton"
require_contains windows/MainWindow.xaml "SharesList"
require_contains windows/MainWindowShares.cs "RenderShares"
require_contains windows/IrisDriveNativeCore.cs "ClassifyLinkInput"
require_contains windows/App.xaml.cs "new MainWindow(e.Args)"
require_contains windows/App.xaml.cs "NamedPipeServerStream"
require_contains windows/App.xaml.cs "SendLaunchArgumentsToPrimary(e.Args)"
require_contains windows/MainWindow.xaml.cs "ApplyLaunchArguments"
require_contains windows/MainWindowShares.cs "OpenShareDialogFromLink"
require_contains windows/MainWindowShares.cs "OpenContentLinkFromLink"
require_contains windows/IrisDriveServiceContentLinks.cs '["type"] = "import_content_link"'
require_contains windows/MainWindowShares.cs "InviteShareMemberFromEvidenceAsync"
require_contains windows/MainWindowShares.cs "RecordPendingShareInviteAsync"
require_contains windows/MainWindowShares.cs "MissingKeyWrapCount"
require_contains scripts/windows-installer.iss '"URL Protocol"'
require_contains scripts/windows-installer.iss '"""{app}\IrisDrive.exe"" ""%1"""'
require_contains windows/MainWindowDevices.cs "RejectDeviceAsync"
require_absent windows/IrisDriveService.cs 'RunJsonAsync("status")'
require_absent windows/IrisDriveService.cs 'RunJsonAsync("link-input", "validate"'
require_absent windows/IrisDriveService.cs 'RunAsync("shares"'
require_absent windows/IrisDriveService.cs 'RunAsync(BuildLabelArgs(new[] { "approve"'
require_absent windows/IrisDriveService.cs 'RunAsync("devices", "appoint-admin"'

echo "DESKTOP_CORE_BACKED_ACTIONS_OK"
