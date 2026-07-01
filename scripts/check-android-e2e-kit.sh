#!/usr/bin/env bash

set -Eeuo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"

require_file() {
  local path="$1"
  if [[ ! -f "$ROOT/$path" ]]; then
    echo "missing required Android e2e kit file: $path" >&2
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

require_file android/settings.gradle.kts
require_file android/app/build.gradle.kts
require_file android/app/src/main/AndroidManifest.xml
require_file android/app/src/main/java/to/iris/drive/app/MainActivity.kt
require_file android/app/src/main/java/to/iris/drive/app/IrisWebActivity.kt
require_file android/app/src/main/java/to/iris/drive/app/IrisDriveAndroidApp.kt
require_file android/app/src/main/java/to/iris/drive/app/IrisDriveDevicesPanel.kt
require_file android/app/src/androidTest/java/to/iris/drive/app/IrisDriveAndroidGuiFlowTest.kt
require_file android/app/src/androidTest/java/to/iris/drive/app/ShareActivityInstrumentedTest.kt
require_file android/app/src/androidTest/java/to/iris/drive/app/provider/IrisDriveDocumentsProviderContractTest.kt
require_file android/app/src/main/java/to/iris/drive/app/provider/IrisDriveDocumentsProvider.kt
require_file scripts/mobile-android-smoke.sh
require_file scripts/android-gui-linking-smoke.sh
require_file scripts/cross-vm-five-platform-e2e.sh
require_file tools/run-android

require_contains android/app/build.gradle.kts "iris-drive-app-core"
require_contains android/app/src/main/AndroidManifest.xml "android.content.action.DOCUMENTS_PROVIDER"
require_contains android/app/src/main/AndroidManifest.xml "iris-drive"
require_contains android/app/src/main/java/to/iris/drive/app/core/AppState.kt "create_profile"
require_contains android/app/src/main/java/to/iris/drive/app/core/AppState.kt "approve_device"
require_contains android/app/src/main/java/to/iris/drive/app/core/AppState.kt "refresh_profile"
require_contains android/app/src/main/java/to/iris/drive/app/core/AppState.kt "setupLabel"
require_contains android/app/src/main/java/to/iris/drive/app/core/AppState.kt "primaryStatusLabel"
require_contains android/app/src/main/java/to/iris/drive/app/core/AppState.kt "roleLabel"
require_contains android/app/src/main/java/to/iris/drive/app/core/AppState.kt "stateLabel"
require_contains android/app/src/main/java/to/iris/drive/app/provider/IrisDriveDocumentsProvider.kt "createDocument"
require_contains android/app/src/main/java/to/iris/drive/app/provider/IrisDriveDocumentsProvider.kt "openDocument"
require_contains android/app/src/main/java/to/iris/drive/app/provider/IrisDriveDocumentsProvider.kt "renameDocument"
require_contains android/app/src/main/java/to/iris/drive/app/provider/IrisDriveDocumentsProvider.kt "deleteDocument"
require_contains android/app/src/androidTest/java/to/iris/drive/app/provider/IrisDriveDocumentsProviderContractTest.kt "DocumentsContract.createDocument"
require_contains android/app/src/androidTest/java/to/iris/drive/app/provider/IrisDriveDocumentsProviderContractTest.kt "openOutputStream"
require_contains android/app/src/androidTest/java/to/iris/drive/app/provider/IrisDriveDocumentsProviderContractTest.kt "openInputStream"
require_contains android/app/src/androidTest/java/to/iris/drive/app/provider/IrisDriveDocumentsProviderContractTest.kt "DocumentsContract.renameDocument"
require_contains android/app/src/androidTest/java/to/iris/drive/app/provider/IrisDriveDocumentsProviderContractTest.kt "DocumentsContract.deleteDocument"
require_contains android/app/src/main/java/to/iris/drive/app/provider/IrisDriveDocumentStore.kt "isChildDocument"
require_contains android/app/src/main/java/to/iris/drive/app/IrisDriveAndroidApp.kt "SetupRoute.Welcome ->"
require_contains android/app/src/main/java/to/iris/drive/app/IrisDriveAndroidApp.kt "route = SetupRoute.CreateProfile"
require_contains android/app/src/main/java/to/iris/drive/app/IrisDriveAndroidApp.kt "route = SetupRoute.RestoreOptions"
require_contains android/app/src/main/java/to/iris/drive/app/IrisDriveDevicesPanel.kt "Add Device"
require_contains android/app/src/main/java/to/iris/drive/app/IrisDriveDevicesPanel.kt "device.displayLabel"
require_contains android/app/src/main/java/to/iris/drive/app/IrisDriveDevicesPanel.kt "device.roleLabel"
require_contains android/app/src/main/java/to/iris/drive/app/IrisDriveDevicesPanel.kt "device.stateLabel"
require_contains android/app/src/main/java/to/iris/drive/app/IrisDriveMainContent.kt "Copy Device"
require_contains android/app/src/main/java/to/iris/drive/app/IrisDriveMainContent.kt "Open in Files"
require_contains android/app/src/main/java/to/iris/drive/app/IrisDriveMainContent.kt "View on drive.iris.to"
require_contains android/app/src/main/java/to/iris/drive/app/IrisDriveMainContent.kt "Reset relay"
require_contains android/app/src/main/java/to/iris/drive/app/IrisWebActivity.kt "EXTRA_PORTAL_URL"
require_contains android/app/src/main/java/to/iris/drive/app/IrisWebActivity.kt "browserAddressUrl"
require_contains android/app/src/main/java/to/iris/drive/app/IrisWebActivity.kt "shareCurrentUrl"
require_contains android/app/src/main/java/to/iris/drive/app/IrisWebActivity.kt "localGatewayUrl"
require_contains android/app/src/main/java/to/iris/drive/app/IrisWebActivity.kt "IME_ACTION_GO"
require_contains android/app/src/main/java/to/iris/drive/app/MainActivity.kt "stateFlow.value.sitesPortalUrl"
require_contains android/app/src/main/java/to/iris/drive/app/MainActivity.kt "waitForIrisPortalUrl"
require_contains android/app/src/main/java/to/iris/drive/app/MainActivity.kt "localGatewayResponds"
require_contains android/app/src/main/java/to/iris/drive/app/MainActivity.kt "HttpURLConnection"
require_contains android/app/src/main/java/to/iris/drive/app/IrisDriveMainContent.kt "Opening Iris Apps"
require_contains android/app/src/main/AndroidManifest.xml "android.intent.action.SEND"
require_contains android/app/src/androidTest/java/to/iris/drive/app/ShareActivityInstrumentedTest.kt "Intent.ACTION_SEND"
require_contains android/app/src/androidTest/java/to/iris/drive/app/ShareActivityInstrumentedTest.kt "ActivityScenario.launch<ShareActivity>"
require_contains android/app/src/androidTest/java/to/iris/drive/app/ShareActivityInstrumentedTest.kt "Mobile share API.txt"
require_file android/app/src/androidTest/java/to/iris/drive/app/IrisDriveAndroidIrisAppsButtonTest.kt
require_contains android/app/src/androidTest/java/to/iris/drive/app/IrisDriveAndroidIrisAppsButtonTest.kt "openIrisAppsButtonStartsGatewayReadinessEvenBeforePortalUrlExists"
require_contains android/app/src/main/java/to/iris/drive/app/MainActivity.kt "NativeActions.refreshProfile()"
require_contains crates/iris-drive-app-core/src/actions.rs "RefreshProfile"
require_absent android/app/src/main/java/to/iris/drive/app/IrisDriveAndroidApp.kt "Text(\"Setup\")"
require_contains android/app/src/main/java/to/iris/drive/app/IrisDriveAndroidApp.kt "Copy Request Link"
require_contains android/app/src/main/java/to/iris/drive/app/IrisDriveAndroidApp.kt "Show join QR"
require_contains android/app/src/main/java/to/iris/drive/app/core/AppState.kt '"start_join_request"'
require_contains android/app/src/main/java/to/iris/drive/app/IrisDriveDevicesPanel.kt "Request link or device key"
require_contains android/app/src/main/java/to/iris/drive/app/IrisDriveDevicesPanel.kt "QrScannerDialog"
require_contains android/app/src/main/java/to/iris/drive/app/IrisDriveDevicesPanel.kt "scanApprovalRequestQr"
require_contains android/app/src/main/java/to/iris/drive/app/IrisDriveDevicesPanel.kt "Approve this device?"
require_absent android/app/src/main/java/to/iris/drive/app/IrisDriveAndroidApp.kt "Device invite link"
require_absent android/app/src/main/java/to/iris/drive/app/IrisDriveDevicesPanel.kt "manualDeviceName"
require_absent android/app/src/main/java/to/iris/drive/app/IrisDriveDevicesPanel.kt "manualDeviceAdd"
require_absent android/app/src/main/java/to/iris/drive/app/IrisDriveDevicesPanel.kt "Name (optional)"
require_absent android/app/src/main/java/to/iris/drive/app/IrisDriveDevicesPanel.kt "Copy invite link"
require_absent android/app/src/main/java/to/iris/drive/app/IrisDriveDevicesPanel.kt "Reset invite"
require_absent android/app/src/main/java/to/iris/drive/app/IrisDriveDevicesPanel.kt "linkInvite"
require_absent android/app/src/main/java/to/iris/drive/app/IrisDriveAndroidApp.kt "Approve Device"
require_absent android/app/src/main/java/to/iris/drive/app/IrisDriveDevicesPanel.kt "Approve Device"
require_contains scripts/mobile-android-smoke.sh "PROVIDER_AUTHORITY"
require_contains scripts/mobile-android-smoke.sh "to.iris.drive.debug.documents"
require_contains scripts/mobile-android-smoke.sh 'Authority: "share"'
require_contains scripts/mobile-android-smoke.sh "create-profile"
require_contains android/app/build.gradle.kts "create(\"uiTest\")"
require_contains scripts/android-gui-linking-smoke.sh "connectedUiTestAndroidTest"
require_contains scripts/android-gui-linking-smoke.sh "to.iris.drive.uitest"
require_contains scripts/android-gui-linking-smoke.sh "IrisDriveAndroidGuiFlowTest"
require_contains scripts/android-gui-linking-smoke.sh "linkDeviceFlowDoesNotRenderInviteInput"
require_contains scripts/android-gui-linking-smoke.sh "addDeviceSectionRequiresCompleteNativeLinkInput"
require_contains scripts/android-gui-linking-smoke.sh "addDeviceSectionDispatchesManualDeviceApproval"
require_contains scripts/android-gui-linking-smoke.sh "ShareActivityInstrumentedTest"
require_absent scripts/android-gui-linking-smoke.sh "linkDeviceSubmitRequiresCompleteNativeLinkInput"
require_absent scripts/android-gui-linking-smoke.sh "addDeviceDialogRequiresCompleteNativeLinkInput"
require_absent android/app/src/androidTest/java/to/iris/drive/app/IrisDriveAndroidGuiFlowTest.kt "linkDeviceSubmit\").assertIsEnabled().performClick()"
require_contains android/app/src/androidTest/java/to/iris/drive/app/IrisDriveAndroidGuiFlowTest.kt "onNodeWithText(\"Approve\")"
require_contains scripts/cross-vm-five-platform-e2e.sh "IRIS_DRIVE_E2E_ANDROID_HOST"
require_contains scripts/cross-vm-five-platform-e2e.sh "scripts/android-gui-linking-smoke.sh"
require_contains scripts/cross-vm-five-platform-e2e.sh 'run_host_repo_command "$ANDROID_HOST"'
require_contains scripts/cross-vm-five-platform-e2e.sh "IRIS_DRIVE_ANDROID_USE_DIRECT_STATIC_PEER"
require_contains scripts/cross-vm-e2e.sh '"local"'
require_contains Justfile "android-build"
require_contains Justfile "android-smoke"
require_contains Justfile "android-gui-smoke"
require_contains Justfile "e2e-5devices"

echo "ANDROID_E2E_KIT_OK"
