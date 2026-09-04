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
require_file ios/Sources/IrisDriveClipboard.swift
require_file ios/Sources/IrisDriveMobileModel.swift
require_file ios/Sources/IrisDriveDeviceApproval.swift
require_file ios/Sources/IrisDriveScreenshotFixture.swift
require_file ios/Sources/IrisDriveNativeCore.swift
require_file ios/Sources/IrisDriveTypes.swift
require_file ios/Resources/PrivacyInfo.xcprivacy
require_file ios/FileProvider/FileProviderExtension.swift
require_file ios/FileProvider/PrivacyInfo.xcprivacy
require_file ios/ShareExtension/ShareItemImporter.swift
require_file ios/ShareExtension/PrivacyInfo.xcprivacy
require_file ios/ShareSource/ShareSourceApp.swift
require_file ios/UnitTests/ShareItemImporterTests.swift
require_file ios/UnitTests/ApprovalReceiptForegroundSyncTests.swift
require_file ios/UITests/IrisDriveIOSUITests.swift
require_file ios/UITests/IrisDriveDeviceApprovalUITests.swift
require_file ios/UITests/IrisDriveAppStoreScreenshotTests.swift
require_file ios/UITests/Fixtures/external-links.html
require_file scripts/ios-app-store-screenshots.sh
require_file scripts/ios-simulator-smoke.sh
require_file scripts/ios-gui-linking-smoke.sh
require_file scripts/lib/ios-linking-observer.sh
require_file scripts/lib/ios-xcuitest-accessibility-session.sh
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
require_contains ios/Info.plist "ITSAppUsesNonExemptEncryption"
require_contains ios/Resources/PrivacyInfo.xcprivacy "NSPrivacyAccessedAPICategoryUserDefaults"
require_contains ios/Resources/PrivacyInfo.xcprivacy "CA92.1"
require_contains ios/Resources/PrivacyInfo.xcprivacy "NSPrivacyAccessedAPICategoryFileTimestamp"
require_contains ios/Resources/PrivacyInfo.xcprivacy "C617.1"
require_contains ios/Resources/PrivacyInfo.xcprivacy "NSPrivacyCollectedDataTypeOtherUserContent"
require_contains ios/Resources/PrivacyInfo.xcprivacy "NSPrivacyCollectedDataTypeUserID"
require_contains ios/Resources/PrivacyInfo.xcprivacy "NSPrivacyCollectedDataTypeDeviceID"
require_contains ios/Resources/PrivacyInfo.xcprivacy "NSPrivacyCollectedDataTypePurposeAppFunctionality"
require_contains ios/FileProvider/PrivacyInfo.xcprivacy "NSPrivacyAccessedAPICategoryFileTimestamp"
require_contains ios/FileProvider/PrivacyInfo.xcprivacy "C617.1"
require_contains ios/ShareExtension/PrivacyInfo.xcprivacy "NSPrivacyAccessedAPICategoryFileTimestamp"
require_contains ios/ShareExtension/PrivacyInfo.xcprivacy "C617.1"
require_contains ios/Sources/IrisDriveIOSApp.swift "ensureFileProviderDomain"
require_contains ios/Sources/IrisDriveScreenshotFixture.swift "IRIS_DRIVE_UI_TEST_SCREENSHOT_FIXTURE"
require_contains ios/UITests/IrisDriveAppStoreScreenshotTests.swift "testCaptureAppStoreScreenshots"
require_contains scripts/ios-app-store-screenshots.sh "IPHONE_69"
require_contains scripts/ios-app-store-screenshots.sh "IPAD_PRO_13"
require_contains ios/Sources/IrisDriveMobileModel.swift "NSFileProviderManager.add"
require_contains ios/Sources/IrisDriveMobileModel.swift "pendingDeviceApprovalReceiptCount"
require_contains ios/Sources/IrisDriveBackgroundSync.swift "pendingApprovalReceiptForegroundSyncIntervalNanoseconds"
require_contains ios/UnitTests/ApprovalReceiptForegroundSyncTests.swift "testPendingApprovalReceiptBypassesOrdinaryDriveSyncMinimum"
require_contains ios/UnitTests/ApprovalReceiptForegroundSyncTests.swift "testPendingApprovalReceiptUsesFastForegroundPoll"
require_contains ios/UnitTests/ApprovalReceiptForegroundSyncTests.swift "testPendingApprovalReceiptUsesDedicatedAckSyncBeforeGeneralSync"
require_contains ios/Sources/IrisDriveMobileModel.swift '"type": "sync_approval_acks"'
require_contains ios/Sources/IrisDriveMobileModel.swift "fileProviderRegistrationIdentity"
require_contains ios/Sources/IrisDriveMobileModel.swift "shouldRepairFileProviderRegistration"
require_contains ios/Sources/IrisDriveMobileModel.swift "repairFileProviderRegistration"
require_contains ios/Sources/IrisDriveClipboard.swift "copyLinkRequest"
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
require_contains ios/Sources/IrisDriveRootView.swift "model.startJoinRequest()"
require_contains ios/Sources/IrisDriveRootView.swift "copyRequestLink"
require_contains ios/Sources/IrisDriveRootView.swift "model.requestDeviceApprovalConfirmation(trimmed)"
require_contains ios/Sources/IrisDriveDeviceApproval.swift '"Approve this device?"'
require_contains ios/Sources/IrisDriveMobileModel.swift "requestDeviceApprovalConfirmation(url.absoluteString)"
require_contains ios/Sources/IrisDriveRootView.swift "IrisDriveNativeLinkInput.isCompleteDeviceApproval(model.approveDeviceKey"
require_contains ios/Sources/IrisDriveRootView.swift "device.isCurrentDevice"
require_absent ios/Sources/IrisDriveRootView.swift "LabeledContent(\"Device\""
require_absent ios/Sources/IrisDriveRootView.swift "Label(\"Copy Device\","
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
require_contains ios/Sources/IrisDriveRootView.swift "https://getdrive.iris.to/privacy/"
require_contains ios/Sources/IrisDriveRootView.swift "https://getdrive.iris.to/support/"
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
require_contains crates/iris-drive-core/src/fips_sync.rs "pub fn subscribe_presence_changes"
require_contains crates/iris-drive-app-core/src/ffi.rs "let mut presence_changes = sync.subscribe_presence_changes();"
require_contains crates/iris-drive-app-core/src/ffi.rs "presence_changed = presence_changes.changed()"
require_contains crates/iris-drive-app-core/src/ffi.rs "writing native FIPS presence status failed"
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
require_contains ios/Sources/IrisDriveRootView.swift "Copy Request Link"
require_contains ios/Sources/IrisDriveRootView.swift "awaitingApprovalBack"
require_absent ios/Sources/IrisDriveRootView.swift "Start over"
require_contains ios/Sources/IrisDriveMobileModel.swift '"type": "start_join_request"'
require_contains ios/Sources/IrisDriveRootView.swift "Request link or device ID"
require_contains ios/Sources/IrisDriveRootView.swift "copyRequestLink"
require_contains ios/Sources/IrisDriveRootView.swift "scanApprovalRequestQr"
require_absent ios/Sources/IrisDriveRootView.swift "Text(model.appKeyLinkRequest)"
require_absent ios/Sources/IrisDriveRootView.swift "Device invite link"
require_absent ios/Sources/IrisDriveRootView.swift "manualDeviceName"
require_absent ios/Sources/IrisDriveRootView.swift "manualDeviceAdd"
require_absent ios/Sources/IrisDriveRootView.swift ".navigationTitle(\"Setup\")"
require_absent ios/Sources/IrisDriveRootView.swift "UIDocumentPickerViewController"
require_absent ios/Sources/IrisDriveRootView.swift "DriveFolderBrowser"
require_absent ios/Sources/IrisDriveRootView.swift "Copy invite link"
require_absent ios/Sources/IrisDriveRootView.swift "Reset invite"
require_contains ios/Sources/IrisDriveRootView.swift "Device requests"
require_absent ios/Sources/IrisDriveRootView.swift "Approve device"
require_contains scripts/ios-simulator-smoke.sh "xcrun simctl"
require_contains scripts/ios-simulator-smoke.sh "IRIS_DRIVE_IOS_SIMULATOR_BOOT_TIMEOUT_SECONDS"
require_contains scripts/ios-simulator-smoke.sh "wait_for_simulator_boot"
require_contains scripts/ios-simulator-smoke.sh "SIMCTL_CHILD_IRIS_DRIVE_DEBUG_ACTION"
require_contains scripts/ios-gui-linking-smoke.sh "testLinkThisDeviceFromWelcome"
require_contains scripts/ios-gui-linking-smoke.sh "testAddLinkedDeviceFromDevices"
require_contains scripts/ios-gui-linking-smoke.sh "testDeviceApprovalUniversalLinkCancelDoesNotApprove"
require_contains scripts/ios-gui-linking-smoke.sh "testDeviceApprovalUniversalLinkApprovesOnlyAfterTap"
require_contains scripts/ios-gui-linking-smoke.sh 'assert_config_link_state "$SIM_APP_BASE_DIR" 1 0'
require_contains scripts/ios-gui-linking-smoke.sh 'assert_config_link_state "$SIM_APP_BASE_DIR" 2 any'
require_contains scripts/ios-gui-linking-smoke.sh 'assert_config_link_state "$SIM_APP_BASE_DIR" 3 any'
require_contains scripts/lib/ios-linking-observer.sh "ios_config_mutation_audit"
require_contains scripts/ios-gui-linking-smoke.sh "start_linked_device_sync_observer"
require_contains scripts/ios-gui-linking-smoke.sh "assert_linked_device_exchange_observed_before"
require_contains scripts/lib/ios-linking-observer.sh 'IOS_APPROVAL_EXCHANGE_OBSERVED phase=$phase'
require_contains scripts/lib/ios-linking-observer.sh 'product_ack_persisted_at=$product_ack_persisted_at'
require_contains scripts/lib/ios-linking-observer.sh 'approval_receipt_consumed_at "$state_file" "$deadline"'
require_contains scripts/lib/ios-linking-observer.sh 'event.get("phase") != "pending_receipt_transition"'
require_contains scripts/ios-gui-linking-smoke.sh '--event-log "$LOCAL_RELAY_EVENT_LOG"'
require_contains scripts/lib/ios-linking-observer.sh "python3 -c 'import time; print(time.time())'"
require_contains scripts/lib/ios-linking-observer.sh 'float(sys.argv[1]) <= float(sys.argv[2])'
require_contains scripts/lib/ios-linking-observer.sh 'pending_device_approval_receipt_count'
require_contains scripts/lib/ios-linking-observer.sh 'expected_owner_roster'
require_contains scripts/lib/ios-linking-observer.sh 'xcrun simctl launch "$DEVICE_UDID" "$BUNDLE_ID"'
require_contains scripts/lib/ios-linking-observer.sh 'kill "$observer_pid"'
require_absent scripts/lib/ios-linking-observer.sh 'if ! kill -0 "$observer_pid"'
require_contains scripts/ios-gui-linking-smoke.sh 'rm -f "$OWNER_DAEMON_LOG" "$LOCAL_RELAY_READY" "$LOCAL_RELAY_LOG"'
require_absent scripts/ios-gui-linking-smoke.sh "sync_linked_device_before"
require_absent scripts/ios-gui-linking-smoke.sh 'wait_for_approval_ack "$SIM_APP_BASE_DIR" "universal-link"'
require_absent scripts/ios-gui-linking-smoke.sh 'wait_for_approval_ack "$SIM_APP_BASE_DIR" "manual-link"'
require_absent scripts/lib/ios-linking-observer.sh '"$IDRIVE" --config-dir "$owner_config_dir" sync'
require_contains scripts/ios-gui-linking-smoke.sh 'cli_owner_approval_deadline=$((cli_owner_approval_started + 15))'
require_contains scripts/ios-gui-linking-smoke.sh 'wait_for_config_status_before'
require_contains scripts/ios-gui-linking-smoke.sh 'wait_for_approval_ack "$OWNER_CONFIG" "CLI-owner-to-iOS" "$cli_owner_approval_deadline"'
require_contains scripts/lib/ios-linking-observer.sh 'IOS_CLI_OWNER_ACK_DAEMON_TIMELINE'
require_contains scripts/lib/ios-linking-observer.sh 'IOS_NATIVE_APP_KEY_LINK_AUDIT'
require_contains scripts/lib/ios-linking-observer.sh 'native-app-key-link-audit.json'
require_contains scripts/lib/ios-linking-observer.sh 'IOS_CLI_OWNER_ACK_OBSERVED phase=$phase'
require_contains scripts/ios-gui-linking-smoke.sh 'IRIS_DRIVE_IOS_CLI_OWNER_ACK_ONLY'
require_contains scripts/ios-gui-linking-smoke.sh "testOpenIrisAppsLoadsBrowserWithoutConnectionError"
require_contains scripts/ios-gui-linking-smoke.sh "testMyDriveShowsSyncStatusWithoutMobilePauseControls"
require_contains scripts/ios-gui-linking-smoke.sh "testShareSheetImportsFileFromExternalSender"
require_contains scripts/ios-gui-linking-smoke.sh "Iris Drive Share Source.app"
require_contains scripts/ios-gui-linking-smoke.sh "--app-group"
require_contains scripts/ios-gui-linking-smoke.sh "IrisDriveIOSShareExtensionTests"

python3 - \
  "$ROOT/scripts/ios-gui-linking-smoke.sh" \
  "$ROOT/scripts/lib/ios-linking-observer.sh" <<'PY'
import pathlib
import sys

main_source = pathlib.Path(sys.argv[1]).read_text(encoding="utf-8")
observer_source = pathlib.Path(sys.argv[2]).read_text(encoding="utf-8")
source = observer_source + "\n" + main_source

initial_mobile_local_relay = source.index(
    '"$IDRIVE" --config-dir "$SIM_APP_BASE_DIR" relays add "$LOCAL_RELAY_URL"'
)
awaiting_approval_ui = source.index(
    '"IrisDriveIOSUITests/IrisDriveIOSUITests/testAwaitingApprovalViewVisible"'
)
if initial_mobile_local_relay > awaiting_approval_ui:
    raise SystemExit(
        "the initial iOS linked profile must use the local relay before XCTest teardown"
    )

observer_function = source.index("start_linked_device_sync_observer()")
observer_assertion = source.index(
    "assert_linked_device_exchange_observed_before()", observer_function
)
linked_authorized_branch = source.index(
    'if [[ -s "$linked_observation_file" ]]; then', observer_function, observer_assertion
)
linked_sync = source.index(
    '"$IDRIVE" --config-dir "$linked_config_dir" sync',
    observer_function,
    linked_authorized_branch,
)
local_relay_override = source.index(
    '--relay "$LOCAL_RELAY_URL" --timeout 2',
    linked_sync,
    linked_authorized_branch,
)
owner_pending_status = source.index(
    '"$IDRIVE" --config-dir "$owner_config_dir" status',
    local_relay_override,
    linked_authorized_branch,
)
owner_receipt_keepalive = source.index(
    'xcrun simctl launch "$DEVICE_UDID" "$BUNDLE_ID"',
    owner_pending_status,
    linked_authorized_branch,
)
linked_status = source.index(
    '"$IDRIVE" --config-dir "$linked_config_dir" status',
    owner_receipt_keepalive,
    linked_authorized_branch,
)
owner_ack_keepalive = source.index(
    'xcrun simctl launch "$DEVICE_UDID" "$BUNDLE_ID"',
    linked_authorized_branch,
    observer_assertion,
)
owner_ack_status = source.index(
    '"$IDRIVE" --config-dir "$owner_config_dir" status',
    owner_ack_keepalive,
    observer_assertion,
)
product_ack_poll = source.index(
    'product_ack_persisted_at="$(approval_receipt_consumed_at',
    observer_assertion,
)
if not (
    linked_sync
    < local_relay_override
    < owner_pending_status
    < owner_receipt_keepalive
    < linked_status
    < linked_authorized_branch
    < owner_ack_keepalive
    < owner_ack_status
    < observer_assertion
    < product_ack_poll
):
    raise SystemExit(
        "linked CLI observer must sync only against the deterministic local relay, "
        "keep the iOS owner alive after a durable pending receipt, then observe real "
        "mobile approval publication and ACK ingestion"
    )


def require_ordered_flow(config, expected_owner_roster, test_name, deadline_assignment, phase):
    test = source.index(test_name)
    observer = source.rfind("start_linked_device_sync_observer", 0, test)
    observer_config = source.index(f'"${config}"', observer, test)
    owner_config = source.index('"$SIM_APP_BASE_DIR"', observer_config, test)
    expected_roster = source.index(
        f"\n  {expected_owner_roster} {chr(92)}\n", owner_config, test
    )
    deadline = source.index(deadline_assignment, test)
    assertion = source.index('assert_linked_device_exchange_observed_before \\', deadline)
    cleanup = source.index('SYNC_OBSERVER_PID=""', assertion)
    if observer < 0 or not (
        observer
        < observer_config
        < owner_config
        < expected_roster
        < test
        < deadline
        < assertion
        < cleanup
    ):
        raise SystemExit(
            f"{phase} approval must start its linked-CLI observer before the UI tap, "
            "then compare authorization and ACK observations with the audit deadline"
        )


require_ordered_flow(
    "LINKED_CONFIG",
    2,
    "testDeviceApprovalUniversalLinkApprovesOnlyAfterTap",
    'linked_deadline="$(approval_deadline "$STATE_FILE")"',
    "universal-link",
)
require_ordered_flow(
    "MANUAL_LINKED_CONFIG",
    3,
    "testAddLinkedDeviceFromDevices",
    'manual_linked_deadline="$(approval_deadline "$STATE_FILE")"',
    "manual-link",
)
PY

require_contains scripts/ios-device-smoke.sh "IrisDriveIOSShareExtensionTests"
require_contains scripts/ios-device-smoke.sh "IOS_DEVICE_SHARE_EXTENSION_TESTS_OK"
require_contains scripts/ios-device-smoke.sh 'TARGET_DIR="${CARGO_TARGET_DIR:-$ROOT/ios/.build/RustDeviceTarget}"'
require_contains scripts/ios-device-smoke.sh 'RUST_LIB_DIR="$TARGET_DIR/$RUST_IOS_TARGET/release"'
require_contains scripts/ios-device-smoke.sh 'cargo build -p iris-drive-app-core --target "$RUST_IOS_TARGET" --release'
require_contains scripts/ios-device-smoke.sh 'CARGO_TARGET_DIR="$TARGET_DIR"'
require_contains scripts/ios-device-smoke.sh 'IPHONEOS_DEPLOYMENT_TARGET="$RUST_IOS_DEPLOYMENT_TARGET"'
require_contains scripts/ios-build 'RUST_IOS_DEPLOYMENT_TARGET="${IRIS_DRIVE_IOS_DEPLOYMENT_TARGET:-17.0}"'
require_contains scripts/ios-build 'IPHONEOS_DEPLOYMENT_TARGET="$RUST_IOS_DEPLOYMENT_TARGET"'
require_contains scripts/ios-app-store-screenshots.sh 'IPHONEOS_DEPLOYMENT_TARGET="$RUST_IOS_DEPLOYMENT_TARGET"'
require_contains scripts/ios-device-smoke.sh 'local status'
require_contains scripts/ios-device-smoke.sh 'return "$status"'
require_contains scripts/ios-device-iris-apps-smoke.sh 'local status'
require_contains scripts/ios-device-iris-apps-smoke.sh 'return "$status"'
require_contains scripts/ios-device-iris-apps-smoke.sh "assert_device_awake_for_launch"
require_contains scripts/ios-device-iris-apps-smoke.sh 'data.get("webview_ready_state") != "complete"'
require_contains scripts/ios-device-iris-apps-smoke.sh 'required_http_200'
require_contains scripts/ios-gui-linking-smoke.sh 'ios_xcuitest_accessibility_session_disabled_after "$BUILD_LOG" "$build_log_offset"'
require_contains scripts/ios-gui-linking-smoke.sh 'restart_ios_simulator_accessibility_session "$DEVICE_UDID"'
require_contains ios/Sources/IrisDriveMobileBrowser.swift "irisDebugWebViewIsMaterialized"
require_contains ios/UnitTests/IrisWebGatewayRetryTests.swift "testMaterializedLauncherDoesNotRequireNavigationDelegateCompletion"
require_contains ios/UITests/IrisDriveIOSUITests.swift "testShareSheetImportsFileFromExternalSender"
require_contains ios/UITests/IrisDriveIOSUITests.swift "assertSharedFileVisibleInFiles(sharedFile, in: refreshed)"
require_contains ios/UITests/IrisDriveIOSUITests.swift "assertFilesOpen(in: app, files: files, timeout: 25, expectedItem: sharedFile)"
require_contains ios/UITests/IrisDriveIOSUITests.swift "files.activate()"
require_contains ios/UITests/IrisDriveIOSUITests.swift "files.buttons[\"BackButton\"]"
require_contains ios/UITests/IrisDriveIOSUITests.swift "CGVector(dx: 0.095, dy: 0.096)"
require_contains ios/UITests/IrisDriveIOSUITests.swift "Simulator Files did not expose the Iris Drive location."
require_contains ios/UITests/IrisDriveIOSUITests.swift "Save to Iris Drive"
require_contains ios/UITests/IrisDriveIOSUITests.swift "testMyDriveShowsSyncStatusWithoutMobilePauseControls"
require_contains ios/UITests/IrisDriveIOSUITests.swift "assertIrisAppsLauncherContentLoaded"
require_contains ios/UITests/IrisDriveIOSUITests.swift "assertNoFilesProviderTrouble"
require_contains ios/UITests/IrisDriveIOSUITests.swift "syncing with iris drive paused"
require_contains ios/UITests/IrisDriveIOSUITests.swift "testIrisWebLauncherExternalLinksOpenSystemBrowser"
require_absent scripts/ios-gui-linking-smoke.sh "simctl pbcopy"
require_absent ios/UITests/IrisDriveIOSUITests.swift "linkTargetInput\"].typeText"
require_absent ios/UITests/IrisDriveIOSUITests.swift "manualDeviceId\"].typeText"
require_absent ios/UITests/IrisDriveIOSUITests.swift "manualDeviceName\"].typeText"
require_contains ios/UITests/IrisDriveIOSUITests.swift "app.buttons[\"Approve\"].tap()"
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

accessibility_log="$(mktemp -t iris-drive-ios-ax-session.XXXXXX)"
trap 'rm -f "$accessibility_log"' EXIT
# shellcheck source=scripts/lib/ios-xcuitest-accessibility-session.sh
source "$ROOT/scripts/lib/ios-xcuitest-accessibility-session.sh"
printf '%s\n' 'Error getting main window kAXErrorAPIDisabled' >"$accessibility_log"
offset="$(wc -c <"$accessibility_log" | tr -d ' ')"
if ios_xcuitest_accessibility_session_disabled_after "$accessibility_log" "$offset"; then
  echo "stale accessibility failures from an earlier test must not trigger recovery" >&2
  exit 1
fi
printf '%s\n' 'ordinary product assertion failure' >>"$accessibility_log"
if ios_xcuitest_accessibility_session_disabled_after "$accessibility_log" "$offset"; then
  echo "ordinary product assertions must not trigger accessibility recovery" >&2
  exit 1
fi
offset="$(wc -c <"$accessibility_log" | tr -d ' ')"
printf '%s\n' 'Failed to get matching snapshots: Error getting main window kAXErrorAPIDisabled' >>"$accessibility_log"
ios_xcuitest_accessibility_session_disabled_after "$accessibility_log" "$offset" \
  || { echo "current XCUITest accessibility failure was not classified" >&2; exit 1; }
offset="$(wc -c <"$accessibility_log" | tr -d ' ')"
printf '%s\n' 'Failed to initialize for UI testing: Timed out while enabling automation mode.' >>"$accessibility_log"
ios_xcuitest_automation_mode_timed_out_after "$accessibility_log" "$offset" \
  || { echo "current XCUITest automation-mode timeout was not classified" >&2; exit 1; }
rm -f "$accessibility_log"
trap - EXIT

echo "IOS_E2E_KIT_OK"
