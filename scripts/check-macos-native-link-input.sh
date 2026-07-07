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

require_contains crates/iris-drive-app-core/src/lib.rs "classify_link_input"
require_contains crates/iris-drive-core/src/app_key_link_transport.rs "APP_KEY_APPROVAL_COMPACT_PREFIX"
require_contains crates/iris-drive-core/src/app_key_link_transport.rs "iris-drive://app-key-link"
require_contains crates/iris-drive-app-core/src/c_abi.rs "iris_drive_validate_link_input_json"
require_contains crates/iris-drive-app-core/src/c_abi.rs "iris_drive_validate_device_invite_input_json"
require_contains crates/iris-drive-app-core/src/c_abi.rs "iris_drive_validate_device_approval_input_json"
require_contains crates/iris-drive-app-core/src/c_abi.rs "validateLinkInputJson"
require_contains crates/iris-drive-app-core/src/c_abi.rs "validateDeviceInviteInputJson"
require_contains crates/iris-drive-app-core/src/c_abi.rs "validateDeviceApprovalInputJson"
require_contains crates/iris-drive-cli/src/commands.rs "LinkInput"
require_contains crates/iris-drive-cli/src/commands.rs "Validate"
require_contains macos/Sources/IrisDriveDesktopCore.swift "iris_drive_validate_link_input_json"
require_contains macos/Sources/IrisDriveDesktopCore.swift "iris_drive_validate_device_invite_input_json"
require_contains macos/Sources/IrisDriveDesktopCore.swift "iris_drive_validate_device_approval_input_json"
require_contains macos/Sources/IrisDriveDesktopCore.swift "static func validateLinkInput"
require_contains macos/Sources/IrisDriveDesktopCore.swift "static func validateDeviceInviteInput"
require_contains macos/Sources/IrisDriveDesktopCore.swift "static func validateDeviceApprovalInput"
require_contains macos/Sources/IrisDriveMacApp.swift 'IrisDriveDesktopCore.validateDeviceInviteInput(text)'
require_contains macos/Sources/IrisDriveMacApp.swift 'IrisDriveDesktopCore.validateDeviceApprovalInput(text)'
require_contains macos/Sources/AppKeyLinkInput.swift "inputIsComplete"
require_contains macos/Sources/IrisDriveControlPanel.swift "controller.startJoinRequest()"
require_contains macos/Sources/IrisDriveControlPanel.swift "approveDeviceKeyIsComplete"
require_contains macos/Sources/IrisDriveControlPanel.swift 'setupSubmit("Show join QR")'
require_contains macos/Sources/IrisDriveControlPanel.swift "IrisDriveDesktopCore.validateDeviceApprovalInput(request)"
require_contains windows/IrisDriveNativeCore.cs "iris_drive_validate_device_invite_input_json"
require_contains windows/IrisDriveNativeCore.cs "iris_drive_validate_device_approval_input_json"
require_contains windows/IrisDriveNativeCore.cs "public static bool IsCompleteDeviceInviteInput"
require_contains windows/IrisDriveNativeCore.cs "public static bool IsCompleteDeviceApprovalInput"
require_contains windows/IrisDriveService.cs "IrisDriveNativeCore.IsCompleteDeviceInviteInput(input)"
require_contains windows/IrisDriveService.cs "IrisDriveNativeCore.IsCompleteDeviceApprovalInput(input)"
require_contains windows/AppKeyLinkInput.cs "StartJoinRequestAsync"
require_contains windows/MainWindowDevices.cs "RefreshAddDeviceInputAsync"
require_contains windows/MainWindowDevices.cs "IsCompleteDeviceApprovalInputAsync(deviceBox.Text)"
require_absent macos/Sources/IrisDriveControlPanel.swift ".disabled(setupOwner.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty)"
require_absent macos/Sources/IrisDriveControlPanel.swift ".disabled(approveDeviceKey.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty)"
require_absent windows/AppKeyLinkInput.cs "IsCompleteOwnerKeyInput"
require_absent windows/AppKeyLinkInput.cs "iris-drive://invite/"
require_absent windows/AppKeyLinkInput.cs "iris-drive://link-device?"
require_contains ios/Sources/IrisDriveNativeCore.swift "irisDriveValidateLinkInputJson"
require_contains ios/Sources/IrisDriveNativeCore.swift "irisDriveValidateDeviceInviteInputJson"
require_contains ios/Sources/IrisDriveNativeCore.swift "irisDriveValidateDeviceApprovalInputJson"
require_contains android/app/src/main/java/to/iris/drive/app/core/NativeCore.kt "validateDeviceInviteInputJson"
require_contains android/app/src/main/java/to/iris/drive/app/core/NativeCore.kt "validateDeviceApprovalInputJson"

echo "MACOS_NATIVE_LINK_INPUT_OK"
