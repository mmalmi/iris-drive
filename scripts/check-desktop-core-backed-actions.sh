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
require_absent linux/src/setup.rs 'run_idrive(["revoke", device])'
require_absent linux/src/setup.rs 'run_idrive(["devices", "appoint-admin", device])'
require_absent linux/src/setup.rs 'run_idrive(["devices", "demote-admin", device])'

require_contains macos/Sources/IrisDriveDesktopCore.swift "final class IrisDriveDesktopCore"
require_contains macos/Sources/IrisDriveMacApp.swift "desktopCore.refreshJson()"
require_contains macos/Sources/IrisDriveMacApp.swift "applyNativeStatePayload"
require_contains macos/Sources/IrisDriveControlPanel.swift "Reject"
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
require_contains windows/IrisDriveService.cs "nativeCore.RefreshJson()"
require_contains windows/IrisDriveModels.cs "FromNativeJson"
require_contains windows/MainWindowDevices.cs "RejectDeviceAsync"
require_absent windows/IrisDriveService.cs 'RunJsonAsync("status")'
require_absent windows/IrisDriveService.cs 'RunJsonAsync("link-input", "validate"'
require_absent windows/IrisDriveService.cs 'RunAsync(BuildLabelArgs(new[] { "approve"'
require_absent windows/IrisDriveService.cs 'RunAsync("devices", "appoint-admin"'

echo "DESKTOP_CORE_BACKED_ACTIONS_OK"
