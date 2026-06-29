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
  if ! grep -F -- "$pattern" "$ROOT/$path" >/dev/null; then
    echo "missing '$pattern' in $path" >&2
    exit 1
  fi
}

require_absent() {
  local path="$1"
  local pattern="$2"
  if grep -F -- "$pattern" "$ROOT/$path" >/dev/null; then
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
require_file ios/ShareExtension/ShareItemImporter.swift
require_file ios/ShareSource/ShareSourceApp.swift
require_file ios/UnitTests/ShareItemImporterTests.swift
require_file ios/UITests/IrisDriveIOSUITests.swift
require_file ios/UITests/Fixtures/external-links.html
require_file scripts/ios-simulator-smoke.sh
require_file scripts/ios-gui-linking-smoke.sh
require_file scripts/cross-vm-four-platform-e2e.sh

require_contains ios/project.yml "IrisDriveIOS"
require_contains ios/project.yml "IrisDriveFileProvider"
require_contains ios/project.yml "IrisDriveIOSShareExtensionTests"
require_contains ios/project.yml "IrisDriveShareSource"
require_contains ios/project.yml 'TEST_HOST: "$(BUILT_PRODUCTS_DIR)/Iris Drive.app/Iris Drive"'
require_contains ios/project.yml 'BUNDLE_LOADER: "$(TEST_HOST)"'
require_contains ios/Info.plist "CFBundleURLSchemes"
require_contains ios/Info.plist "iris-drive"
require_contains ios/Info.plist "NSAppTransportSecurity"
require_contains ios/Info.plist "NSAllowsLocalNetworking"
require_contains ios/Info.plist "NSExceptionDomains"
require_contains ios/Info.plist "iris.localhost"
require_contains ios/Info.plist "hash.localhost"
require_contains ios/Info.plist "NSExceptionAllowsInsecureHTTPLoads"
require_contains ios/Info.plist "NSIncludesSubdomains"
require_contains ios/Sources/IrisDriveIOSApp.swift "ensureFileProviderDomain"
require_contains ios/Sources/IrisDriveMobileModel.swift "NSFileProviderManager.add"
require_contains ios/Sources/IrisDriveMobileModel.swift "fileProviderRegistrationIdentity"
require_contains ios/Sources/IrisDriveMobileModel.swift "shouldRepairFileProviderRegistration"
require_contains ios/Sources/IrisDriveMobileModel.swift "repairFileProviderRegistration"
require_contains ios/Sources/IrisDriveMobileModel.swift "copyLinkInvite"
require_contains ios/Sources/IrisDriveNativeCore.swift "iris_drive_provider_compose_path_json"
require_contains ios/FileProvider/FileProviderStorage.swift "IrisDriveNativeProvider.composePath"
require_contains ios/FileProvider/FileProviderStorage.swift "create mayAlreadyExist absent path="
require_contains ios/FileProvider/FileProviderStorage.swift "existingPlaceholderFamilyItem"
require_absent ios/FileProvider/FileProviderStorage.swift "create mayAlreadyExist rejected absent"
require_contains ios/Sources/IrisDriveMobileModel.swift "openDriveFolder"
require_contains ios/Sources/IrisDriveMobileModel.swift "UIApplication.shared.open(filesURL, options: [:])"
require_contains ios/Sources/IrisDriveMobileModel.swift "scheduleFilesRootFallbackIfStillActive"
require_contains ios/Sources/IrisDriveMobileModel.swift "shareddocuments://"
require_contains ios/Sources/IrisDriveMobileModel.swift "addRelay"
require_contains ios/Sources/IrisDriveMobileModel.swift "IrisDriveNativeLinkInput.classify"
require_contains ios/Sources/IrisDriveMobileModel.swift "func localGatewayURL"
require_contains ios/Sources/IrisDriveMobileModel.swift "func browserAddressURL"
require_contains ios/Sources/IrisDriveMobileBrowser.swift "readyIrisBrowserURL"
require_contains ios/Sources/IrisDriveMobileBrowser.swift "localGatewayResponds"
require_contains ios/Sources/IrisDriveMobileBrowser.swift "irisWebShouldOpenExternally"
require_contains ios/Sources/IrisDriveMobileBrowser.swift "silent.link"
require_contains ios/Sources/IrisDriveMobileBrowser.swift "protonmail.com"
require_contains ios/Sources/IrisDriveMobileBrowser.swift "URLSession.shared.data"
require_contains ios/Sources/IrisDriveMobileModel.swift '"type": "refresh_profile"'
require_contains ios/Sources/IrisDriveMobileBrowser.swift "URLComponents(string: activePortalUrl)?.port"
require_contains ios/Sources/IrisDriveRootView.swift ".fullScreenCover(item: \$model.webRoute)"
require_absent ios/Sources/IrisDriveRootView.swift ".sheet(item: \$model.webRoute)"
require_contains ios/Sources/IrisDriveRootView.swift "let linkInput = IrisDriveNativeLinkInput.validate(trimmed)"
require_contains ios/Sources/IrisDriveRootView.swift "linkDeviceErrorMessage"
require_contains ios/Sources/IrisDriveRootView.swift "submitLinkDevice(code, force: true)"
require_contains ios/Sources/IrisDriveRootView.swift "IrisDriveNativeLinkInput.isComplete(model.approveDeviceKey"
require_contains ios/Sources/IrisDriveRootView.swift "irisWebLoading"
require_contains ios/Sources/IrisDriveRootView.swift "irisWebError"
require_contains ios/Sources/IrisDriveRootView.swift "irisWebAddressField"
require_contains ios/Sources/IrisDriveRootView.swift "irisWebBackButton"
require_contains ios/Sources/IrisDriveRootView.swift "irisWebCloseButton"
require_contains ios/Sources/IrisDriveRootView.swift "irisWebReloadButton"
require_contains ios/Sources/IrisDriveRootView.swift "irisWebMoreButton"
require_contains ios/Sources/IrisDriveRootView.swift "irisWebCompactTitle"
require_contains ios/Sources/IrisDriveRootView.swift "irisWebNavigationAction(for: url)"
require_absent ios/Sources/IrisDriveRootView.swift ".navigationTitle(\"Iris Apps\")"
require_contains ios/Sources/IrisDriveTypes.swift "storageDirectoryName = \"IrisDrive\""
require_absent ios/Sources/IrisDriveMobileModel.swift "applicationSupportDirectory"
require_absent ios/Sources/IrisDriveMobileModel.swift "UIDocumentPickerViewController"
require_contains ios/FileProvider/FileProviderStorage.swift "storageDirectoryName = \"IrisDrive\""
require_absent ios/FileProvider/FileProviderStorage.swift "applicationSupportDirectory"
require_contains ios/ShareExtension/ShareItemImporter.swift "loadFileRepresentation"
require_contains ios/ShareExtension/ShareItemImporter.swift "provider.registeredTypeIdentifiers"
require_contains ios/ShareSource/ShareSourceApp.swift "shareFileToIrisDriveButton"
require_contains ios/ShareSource/ShareSourceApp.swift "UIActivityViewController"
require_contains ios/ShareSource/ShareSourceApp.swift "NSItemProvider(contentsOf:"
require_contains ios/UnitTests/ShareItemImporterTests.swift "testWebURLImportCreatesUrlFile"
require_contains ios/UnitTests/ShareItemImporterTests.swift "testDataImportUsesSuggestedImageExtension"
require_contains ios/Sources/IrisDriveNativeCore.swift "iris_drive_app_dispatch_json"
require_contains crates/iris-drive-app-core/src/ffi.rs "start_browser_gateway_if_needed"
require_contains crates/iris-drive-app-core/src/ffi.rs "EmbeddedHashtreeHost::start"
require_contains crates/iris-drive-app-core/src/ffi.rs "GatewayServer::bind_with_tree_and_htree_daemon"
require_contains crates/iris-drive-app-core/src/ffi.rs "GatewayBind::loopback_v4(0)"
require_contains crates/iris-drive-app-core/src/ffi.rs "native_browser_gateway_port_for_state"
require_contains crates/iris-drive-app-core/src/ffi.rs "native_browser_gateway_status_port"
require_contains crates/iris-drive-app-core/src/actions.rs "RefreshProfile"
require_contains ios/Sources/IrisDriveNativeCore.swift "setupLabel = \"setup_label\""
require_contains ios/Sources/IrisDriveNativeCore.swift "primaryStatusLabel = \"primary_status_label\""
require_contains ios/Sources/IrisDriveNativeCore.swift "roleLabel = \"role_label\""
require_contains ios/Sources/IrisDriveNativeCore.swift "stateLabel = \"state_label\""
require_contains ios/Sources/IrisDriveMobileModel.swift "authorizationState = state.ui.setupLabel"
require_contains ios/Sources/IrisDriveMobileModel.swift "statusTitle = state.ui.primaryStatusLabel"
require_contains ios/Sources/IrisDriveMobileModel.swift "startNativeFipsStatusWatcher"
require_contains ios/Sources/IrisDriveMobileModel.swift "native-fips-status.json"
require_contains ios/Sources/IrisDriveMobileModel.swift "Iris Drive native FIPS status file changed"
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
require_contains ios/Sources/IrisDriveRootView.swift "path.append(.restoreOptions)"
require_contains ios/Sources/IrisDriveRootView.swift "Copy invite link"
require_contains ios/Sources/IrisDriveRootView.swift "Scan invite QR"
require_absent ios/Sources/IrisDriveRootView.swift ".navigationTitle(\"Setup\")"
require_absent ios/Sources/IrisDriveRootView.swift "UIDocumentPickerViewController"
require_absent ios/Sources/IrisDriveRootView.swift "DriveFolderBrowser"
require_absent ios/Sources/IrisDriveRootView.swift "Copy request link"
require_absent ios/Sources/IrisDriveRootView.swift "Device Requests"
require_absent ios/Sources/IrisDriveRootView.swift "Approve device"
require_contains scripts/ios-simulator-smoke.sh "xcrun simctl"
require_contains scripts/ios-simulator-smoke.sh "IRIS_DRIVE_IOS_SIMULATOR_BOOT_TIMEOUT_SECONDS"
require_contains scripts/ios-simulator-smoke.sh "wait_for_simulator_boot"
require_contains scripts/ios-simulator-smoke.sh "SIMCTL_CHILD_IRIS_DRIVE_DEBUG_ACTION"
require_contains scripts/ios-gui-linking-smoke.sh "testLinkThisDeviceFromWelcome"
require_contains scripts/ios-gui-linking-smoke.sh "testAddLinkedDeviceFromDevices"
require_contains scripts/ios-gui-linking-smoke.sh "testOpenIrisAppsLoadsBrowserWithoutConnectionError"
require_contains scripts/ios-gui-linking-smoke.sh "testOpenIrisAppsLoadsBrowserWhenSyncPaused"
require_contains scripts/ios-gui-linking-smoke.sh "testShareSheetImportsFileFromExternalSender"
require_contains scripts/ios-gui-linking-smoke.sh "Iris Drive Share Source.app"
require_contains scripts/ios-gui-linking-smoke.sh "--app-group"
require_contains scripts/ios-gui-linking-smoke.sh "IrisDriveIOSShareExtensionTests"
require_contains scripts/ios-device-smoke.sh "IrisDriveIOSShareExtensionTests"
require_contains scripts/ios-device-smoke.sh "IOS_DEVICE_SHARE_EXTENSION_TESTS_OK"
require_contains scripts/ios-device-smoke.sh 'local status'
require_contains scripts/ios-device-smoke.sh 'return "$status"'
require_contains scripts/ios-device-iris-apps-smoke.sh 'local status'
require_contains scripts/ios-device-iris-apps-smoke.sh 'return "$status"'
require_contains scripts/ios-device-iris-apps-smoke.sh "assert_device_awake_for_launch"
require_contains ios/UITests/IrisDriveIOSUITests.swift "testShareSheetImportsFileFromExternalSender"
require_contains ios/UITests/IrisDriveIOSUITests.swift "assertSharedFileVisibleInFiles(sharedFile, in: refreshed)"
require_contains ios/UITests/IrisDriveIOSUITests.swift "assertFilesOpen(in: app, files: files, timeout: 25, expectedItem: sharedFile)"
require_contains ios/UITests/IrisDriveIOSUITests.swift "files.activate()"
require_contains ios/UITests/IrisDriveIOSUITests.swift "files.buttons[\"BackButton\"]"
require_contains ios/UITests/IrisDriveIOSUITests.swift "CGVector(dx: 0.095, dy: 0.096)"
require_contains ios/UITests/IrisDriveIOSUITests.swift "Simulator Files did not expose the Iris Drive location."
require_contains ios/UITests/IrisDriveIOSUITests.swift "Save to Iris Drive"
require_contains ios/UITests/IrisDriveIOSUITests.swift "testOpenIrisAppsLoadsBrowserWhenSyncPaused"
require_contains ios/UITests/IrisDriveIOSUITests.swift "assertIrisAppsLauncherContentLoaded"
require_contains ios/UITests/IrisDriveIOSUITests.swift "assertNoFilesProviderTrouble"
require_contains ios/UITests/IrisDriveIOSUITests.swift "syncing with iris drive paused"
require_contains ios/UITests/IrisDriveIOSUITests.swift "testIrisWebLauncherExternalLinksOpenSystemBrowser"
require_absent scripts/ios-gui-linking-smoke.sh "simctl pbcopy"
require_absent ios/UITests/IrisDriveIOSUITests.swift "linkTargetInput\"].typeText"
require_absent ios/UITests/IrisDriveIOSUITests.swift "manualDeviceId\"].typeText"
require_absent ios/UITests/IrisDriveIOSUITests.swift "manualDeviceName\"].typeText"
require_absent ios/UITests/IrisDriveIOSUITests.swift "UIPasteboard"
require_absent ios/UITests/IrisDriveIOSUITests.swift "app.buttons[\"linkDeviceSubmit\"].tap()"
require_contains scripts/cross-vm-four-platform-e2e.sh "IRIS_DRIVE_E2E_IOS_HOST"
require_contains scripts/cross-vm-four-platform-e2e.sh "scripts/ios-gui-linking-smoke.sh"
require_contains scripts/cross-vm-four-platform-e2e.sh 'run_host_repo_command "$IOS_HOST"'
require_contains scripts/cross-vm-five-platform-e2e.sh 'run_host_repo_command "$IOS_HOST"'
require_contains scripts/cross-vm-five-platform-e2e.sh "scripts/ios-device-iris-apps-smoke.sh"
require_contains scripts/cross-vm-e2e.sh '"local"'
require_contains scripts/cross-vm-e2e.sh 'CARGO_TARGET_DIR'
require_contains Justfile "ios-build"
require_contains Justfile "ios-smoke"
require_contains Justfile "ios-gui-smoke"
require_contains Justfile "e2e-4devices"

echo "IOS_E2E_KIT_OK"
