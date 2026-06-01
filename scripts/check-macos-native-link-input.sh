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
require_contains crates/iris-drive-app-core/src/c_abi.rs "iris_drive_validate_link_input_json"
require_contains crates/iris-drive-app-core/src/c_abi.rs "validateLinkInputJson"
require_contains crates/iris-drive-cli/src/commands.rs "LinkInput"
require_contains crates/iris-drive-cli/src/commands.rs "Validate"
require_contains macos/Sources/IrisDriveMacApp.swift '["link-input", "validate", trimmed]'
require_contains macos/Sources/DeviceLinkInput.swift "inputIsComplete"
require_contains macos/Sources/IrisDriveControlPanel.swift "setupOwnerLinkInputIsComplete"
require_contains macos/Sources/IrisDriveControlPanel.swift "approveDeviceKeyIsComplete"
require_contains macos/Sources/IrisDriveControlPanel.swift ".disabled(!setupOwnerLinkInputIsComplete)"
require_contains macos/Sources/IrisDriveControlPanel.swift ".disabled(!approveDeviceKeyIsComplete)"
require_contains windows/IrisDriveService.cs '"link-input", "validate"'
require_contains windows/DeviceLinkInput.cs "IsCompleteLinkInputAsync"
require_contains windows/MainWindow.xaml.cs "RefreshAddDeviceInputAsync"
require_contains windows/MainWindow.xaml.cs "IsCompleteLinkInputAsync(deviceBox.Text)"
require_absent macos/Sources/IrisDriveControlPanel.swift ".disabled(setupOwner.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty)"
require_absent macos/Sources/IrisDriveControlPanel.swift ".disabled(approveDeviceKey.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty)"
require_absent windows/DeviceLinkInput.cs "IsCompleteDeviceLinkOwnerInput"
require_absent windows/DeviceLinkInput.cs "iris-drive://invite/"
require_absent windows/DeviceLinkInput.cs "iris-drive://link-device?"
require_contains ios/Sources/IrisDriveNativeCore.swift "irisDriveValidateLinkInputJson"
require_contains android/app/src/main/java/to/iris/drive/app/core/NativeCore.kt "validateLinkInputJson"

echo "MACOS_NATIVE_LINK_INPUT_OK"
