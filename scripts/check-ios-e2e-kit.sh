#!/usr/bin/env bash

set -Eeuo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"

require_file() {
  local path="$1"
  if [[ ! -f "$ROOT/$path" ]]; then
    echo "missing required iOS e2e kit file: $path" >&2
    exit 1
  fi
}

require_contains() {
  local path="$1"
  local pattern="$2"
  if ! grep -F "$pattern" "$ROOT/$path" >/dev/null; then
    echo "missing '$pattern' in $path" >&2
    exit 1
  fi
}

require_absent() {
  local path="$1"
  local pattern="$2"
  if grep -F "$pattern" "$ROOT/$path" >/dev/null; then
    echo "unexpected '$pattern' in $path" >&2
    exit 1
  fi
}

require_file ios/project.yml
require_file ios/Sources/IrisDriveIOSApp.swift
require_file ios/Sources/IrisDriveMobileModel.swift
require_file ios/Sources/IrisDriveNativeCore.swift
require_file ios/Sources/IrisDriveTypes.swift
require_file ios/FileProvider/FileProviderExtension.swift
require_file ios/UITests/IrisDriveIOSUITests.swift
require_file scripts/ios-simulator-smoke.sh
require_file scripts/ios-gui-linking-smoke.sh
require_file scripts/cross-vm-four-platform-e2e.sh

require_contains ios/project.yml "IrisDriveIOS"
require_contains ios/project.yml "IrisDriveFileProvider"
require_contains ios/Info.plist "CFBundleURLSchemes"
require_contains ios/Info.plist "iris-drive"
require_contains ios/Sources/IrisDriveIOSApp.swift "ensureFileProviderDomain"
require_contains ios/Sources/IrisDriveMobileModel.swift "NSFileProviderManager.add"
require_contains ios/Sources/IrisDriveMobileModel.swift "copyLinkInvite"
require_contains ios/Sources/IrisDriveMobileModel.swift "openDriveFolder"
require_contains ios/Sources/IrisDriveMobileModel.swift "UIApplication.shared.open(filesURL, options: [:])"
require_contains ios/Sources/IrisDriveMobileModel.swift "addRelay"
require_contains ios/Sources/IrisDriveMobileModel.swift "IrisDriveNativeLinkInput.classify"
require_contains ios/Sources/IrisDriveRootView.swift "IrisDriveNativeLinkInput.isComplete(ownerPublicKey.trimmingCharacters"
require_contains ios/Sources/IrisDriveRootView.swift "IrisDriveNativeLinkInput.isComplete(model.approveDeviceKey"
require_contains ios/Sources/IrisDriveTypes.swift "storageDirectoryName = \"IrisDrive\""
require_absent ios/Sources/IrisDriveMobileModel.swift "applicationSupportDirectory"
require_absent ios/Sources/IrisDriveMobileModel.swift "UIDocumentPickerViewController"
require_contains ios/FileProvider/FileProviderStorage.swift "storageDirectoryName = \"IrisDrive\""
require_absent ios/FileProvider/FileProviderStorage.swift "applicationSupportDirectory"
require_contains ios/Sources/IrisDriveNativeCore.swift "iris_drive_app_dispatch_json"
require_contains ios/Sources/IrisDriveNativeCore.swift "setupLabel = \"setup_label\""
require_contains ios/Sources/IrisDriveNativeCore.swift "primaryStatusLabel = \"primary_status_label\""
require_contains ios/Sources/IrisDriveNativeCore.swift "roleLabel = \"role_label\""
require_contains ios/Sources/IrisDriveNativeCore.swift "stateLabel = \"state_label\""
require_contains ios/Sources/IrisDriveMobileModel.swift "authorizationState = state.ui.setupLabel"
require_contains ios/Sources/IrisDriveMobileModel.swift "statusTitle = state.ui.primaryStatusLabel"
require_contains ios/Sources/IrisDriveMobileModel.swift "role: device.roleLabel"
require_contains ios/Sources/IrisDriveMobileModel.swift "relayStatuses = state.ui.relayStatuses"
require_contains ios/Sources/IrisDriveRootView.swift "ForEach(model.relayStatuses"
require_contains ios/Sources/IrisDriveRootView.swift "relay.statusLabel"
require_contains ios/Sources/IrisDriveRootView.swift "relay.health"
require_absent ios/Sources/IrisDriveMobileModel.swift "private func authorizationTitle"
require_absent ios/Sources/IrisDriveMobileModel.swift "private func statusTitle(for"
require_absent ios/Sources/IrisDriveMobileModel.swift "private func deviceStateTitle"
require_absent ios/Sources/IrisDriveMobileModel.swift "private func roleTitle"
require_absent ios/Sources/IrisDriveRootView.swift "ForEach(model.relays"
require_contains ios/Sources/IrisDriveRootView.swift "private enum SetupRoute"
require_contains ios/Sources/IrisDriveRootView.swift "path.append(.create)"
require_contains ios/Sources/IrisDriveRootView.swift "path.append(.signIn)"
require_contains ios/Sources/IrisDriveRootView.swift "Copy invite link"
require_contains ios/Sources/IrisDriveRootView.swift "Scan invite QR"
require_absent ios/Sources/IrisDriveRootView.swift ".navigationTitle(\"Setup\")"
require_absent ios/Sources/IrisDriveRootView.swift "UIDocumentPickerViewController"
require_absent ios/Sources/IrisDriveRootView.swift "DriveFolderBrowser"
require_absent ios/Sources/IrisDriveRootView.swift "Copy request link"
require_absent ios/Sources/IrisDriveRootView.swift "Device Requests"
require_absent ios/Sources/IrisDriveRootView.swift "Approve device"
require_contains scripts/ios-simulator-smoke.sh "xcrun simctl"
require_contains scripts/ios-simulator-smoke.sh "SIMCTL_CHILD_IRIS_DRIVE_DEBUG_ACTION"
require_contains scripts/ios-gui-linking-smoke.sh "testLinkThisDeviceFromWelcome"
require_contains scripts/ios-gui-linking-smoke.sh "testAddLinkedDeviceFromDevices"
require_absent scripts/ios-gui-linking-smoke.sh "simctl pbcopy"
require_absent ios/UITests/IrisDriveIOSUITests.swift "typeText"
require_absent ios/UITests/IrisDriveIOSUITests.swift "UIPasteboard"
require_absent ios/UITests/IrisDriveIOSUITests.swift "app.buttons[\"linkDeviceSubmit\"].tap()"
require_contains scripts/cross-vm-four-platform-e2e.sh "IRIS_DRIVE_E2E_IOS_HOST"
require_contains scripts/cross-vm-four-platform-e2e.sh "scripts/ios-gui-linking-smoke.sh"
require_contains Justfile "ios-build"
require_contains Justfile "ios-smoke"
require_contains Justfile "ios-gui-smoke"
require_contains Justfile "e2e-4devices"

echo "IOS_E2E_KIT_OK"
