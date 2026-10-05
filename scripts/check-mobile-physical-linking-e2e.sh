#!/usr/bin/env bash
set -euo pipefail

ROOT="$(cd "$(dirname "$0")/.." && pwd)"

python3 - \
  "$ROOT/scripts/mobile-ios-android-linking-e2e.sh" \
  "$ROOT/scripts/lib/mobile-ios-android-manual-linking.sh" \
  "$ROOT/scripts/lib/mobile-physical-ios-xctest.sh" \
  "$ROOT/ios/UITests/IrisDrivePhysicalLinkingUITests.swift" \
  "$ROOT/android/app/src/androidTest/java/to/iris/drive/app/provider/IrisDrivePhysicalLinkingProviderTest.kt" \
  "$ROOT/android/app/src/androidTest/java/to/iris/drive/app/provider/IrisDriveDocumentsProviderContractTest.kt" \
  "$ROOT/ios/Sources/IrisDriveRootView.swift" \
  "$ROOT/ios/Sources/QRCodeScannerView.swift" \
  "$ROOT/android/app/src/main/java/to/iris/drive/app/IrisDriveAndroidApp.kt" \
  "$ROOT/android/app/src/main/java/to/iris/drive/app/MainActivity.kt" \
  "$ROOT/android/app/src/main/java/to/iris/drive/app/IrisDriveDevicesPanel.kt" \
  "$ROOT/android/app/src/main/java/to/iris/drive/app/QrScannerDialog.kt" \
  "$ROOT/scripts/cross-vm-five-platform-e2e.sh" \
  "$ROOT/scripts/macos-vm-android-manual-link-e2e.sh" \
  "$ROOT/scripts/lib/macos-android-physical-evidence.sh" \
  "$ROOT/scripts/lib/android-ui-point.py" \
  "$ROOT/scripts/macos-android-manual-link-remote.sh" \
  "$ROOT/scripts/macos-device-link-ax.swift" \
  "$ROOT/scripts/release-gate.sh" \
  "$ROOT/Justfile" \
  "$ROOT/ios/IrisDriveIOS.xcodeproj/project.pbxproj" <<'PY'
from pathlib import Path
import re
import sys


def read(path: str) -> str:
    file = Path(path)
    if not file.is_file():
        raise SystemExit(f"physical mobile linking file is missing: {file}")
    return file.read_text(encoding="utf-8")


(
    gate,
    manual_gate,
    ios_xctest_helpers,
    ios_test,
    android_provider_test,
    android_provider_contract_test,
    ios_root,
    ios_scanner,
    android_root,
    android_activity,
    android_devices,
    android_scanner,
    full_gate,
    macos_android_gate,
    macos_android_evidence,
    android_ui_point,
    macos_android_remote,
    macos_ax,
    release_gate,
    justfile,
    xcode_project,
) = (read(path) for path in sys.argv[1:])

macos_android_contract = macos_android_gate + macos_android_evidence

physical_gate = gate + manual_gate + ios_xctest_helpers

for forbidden in (
    "IRIS_DRIVE_DEBUG_OWNER",
    "IRIS_DRIVE_DEBUG_REQUEST",
    '"link-device"',
    '"approve-device"',
    "UIPasteboard",
    ".launchArguments",
    "simctl openurl",
    "device process launch --payload",
    "https://drive.iris.to/approve-device/",
    "xcodegen generate",
    "IOS_DEVELOPMENT_TEAM",
    "IOS_CODE_SIGN_IDENTITY",
    "seed-provider-file",
    "IRIS_DRIVE_DEBUG_PROVIDER_FILE",
):
    if forbidden in physical_gate or forbidden in ios_test:
        raise SystemExit(f"physical linking gate can inject a link/request via {forbidden}")

wait_default = re.search(
    r'WAIT_SECS="\$\{IRIS_DRIVE_MOBILE_LINK_WAIT_SECS:-(\d+)\}"', gate
)
if not wait_default or int(wait_default.group(1)) > 15:
    raise SystemExit("physical approval delivery must default to at most 15 seconds")

for forbidden_timing in (
    "ios_marker_value",
    "millisecondsSinceEpoch",
    "IRIS_IOS_APPROVAL_SUBMITTED_MS",
    "host_observed_approval_submission",
):
    if forbidden_timing in physical_gate or forbidden_timing in ios_test:
        raise SystemExit(
            f"physical delivery timing mixes the host clock with iOS time via {forbidden_timing}"
        )
for required_timing in (
    "monotonic_milliseconds",
    "IRIS_IOS_APPROVAL_CONFIRMATION_READY=1",
    'signal_ios_test submit-camera-approval',
    'delivery_measurement_clock": "host_monotonic_ms"',
    'start_checkpoint": "host_released_approval_submission"',
    'finish_checkpoint": "host_observed_authorization"',
):
    if required_timing not in physical_gate:
        raise SystemExit(f"physical same-clock delivery timing is missing {required_timing}")

for required_reuse in (
    "IRIS_DRIVE_MOBILE_REUSE_ANDROID_ARTIFACTS",
    "android_artifacts_valid",
    "reusing validated Android artifacts",
):
    if required_reuse not in physical_gate + full_gate:
        raise SystemExit(f"physical Android artifact reuse is missing {required_reuse}")

for required_xctestrun in (
    'mktemp "$(dirname "$XCTESTRUN")/IrisDriveIOS-physical-$test_name.XXXXXX"',
    'verify_xctestrun_products "$run_file"',
    'value.replace("__TESTROOT__", str(test_root))',
    'value.replace("__TESTHOST__", str(test_host))',
    'IOS_TEST_RUN_FILE="$run_file"',
    'rm -f "$IOS_TEST_RUN_FILE"',
):
    if required_xctestrun not in gate:
        raise SystemExit(
            f"physical iOS XCTest relocation safety is missing {required_xctestrun}"
        )
if 'local run_file="$TMP/$test_name.xctestrun"' in gate:
    raise SystemExit("physical xctestrun must remain beside its __TESTROOT__ products")

for required_bridge in (
    'source "$ROOT/scripts/lib/ios-xcuitest-accessibility-session.sh"',
    'IRIS_DRIVE_IOS_PHYSICAL_RUNNER_START_WAIT_SECS:-60',
    'IRIS_DRIVE_IOS_PHYSICAL_BRIDGE_READY_WAIT_SECS:-65',
    'if ((IOS_BRIDGE_READY_WAIT_SECS > 65))',
    'wait_for_ios_runner_or_marker',
    'endswith("/IrisDriveIOSUITests-Runner")',
    '-resultBundlePath "$IOS_TEST_RESULT_BUNDLE"',
    'ios_xcuitest_accessibility_session_disabled_after "$IOS_TEST_LOG" 0',
    'ios_xcuitest_automation_mode_timed_out_after "$IOS_TEST_LOG" 0',
):
    if required_bridge not in physical_gate:
        raise SystemExit(f"physical iOS bridge classification is missing {required_bridge}")
runner_wait = gate.rindex("wait_for_ios_runner_or_marker")
bridge_wait = gate.rindex(
    'if ! wait_for_ios_marker "IRIS_XCUITEST_ENVIRONMENT_READY=1"'
)
if runner_wait >= bridge_wait:
    raise SystemExit("physical iOS bridge must classify runner launch before automation readiness")

for required in (
    'source "$ROOT/scripts/lib/mobile-physical-ios-xctest.sh"',
    "select_physical_android",
    "select_physical_ios",
    "ro.kernel.qemu",
    "testIosOwnerApprovesAndroidThroughPhysicalCamera",
    "testIosJoinerDisplaysPhysicalQrAndBecomesAuthorized",
    "testIosOwnerApprovesAndroidThroughManualEntry",
    "testIosJoinerWaitsForAndroidManualApproval",
    "IRIS_IOS_CAMERA_READY=1",
    "IRIS_IOS_REQUEST_QR_READY=1",
    "wait_for_android_authorized",
    "wait_for_android_provider_entry",
    "run_android_provider_test",
    "writePostLinkFileThroughDocumentsProvider",
    "readPostLinkFileThroughDocumentsProvider",
    "assert_ios_provider_content",
    "sha256_file",
    "signal_ios_test",
    "IRIS_IOS_HOST_AUTHORIZATION_OBSERVED=1",
    "IRIS_IOS_PROVIDER_WRITE_SHA256=",
    "IRIS_IOS_PEER_FILE_VISIBLE=1",
    "IRIS_IOS_LIFECYCLE_RESUMED=1",
    "IRIS_IOS_MANUAL_ENTRY_READY=1",
    "IRIS_ANDROID_REQUEST_QR_RESUMED=1",
    "assert_pending_receipts_cleared",
    "pending_device_approval_receipts",
    "IRIS_XCUITEST_FINISHED=",
    "Executed 1 test",
    "-only-testing:",
    "MOBILE_PHYSICAL_LINKING_OK",
):
    if required not in physical_gate:
        raise SystemExit(f"physical linking gate is missing {required}")

for required in (
    "app_key_link_request_from_config",
    "IRIS_XCUITEST_MANUAL_REQUEST_B64",
    "type_android_manual_approval_request",
    "run_ios_owner_android_joiner_manual",
    "run_android_owner_ios_joiner_manual",
    "IRIS_IOS_MANUAL_REQUEST_ENTERED=1",
    "IRIS_IOS_MANUAL_CONFIRMATION_READY=1",
    "IRIS_IOS_MANUAL_APPROVAL_SUBMITTED=1",
    "signal_ios_test submit-manual-approval",
    "IRIS_IOS_MANUAL_REQUEST_READY=1",
    "IRIS_IOS_MANUAL_AUTHORIZED=1",
    '"manual_entry_transfer": "complete production request typed through shipped UI"',
    '"manual_directions"',
    '"ios_owner_to_android_joiner_manual_ms"',
    '"android_owner_to_ios_joiner_manual_ms"',
):
    if required not in physical_gate:
        raise SystemExit(f"physical manual-entry gate is missing {required}")

qr_ios = gate.find("run_ios_owner_android_joiner\n")
qr_android = gate.find("run_android_owner_ios_joiner\n")
manual_ios = gate.find("run_ios_owner_android_joiner_manual\n")
manual_android = gate.find("run_android_owner_ios_joiner_manual\n")
if min(qr_ios, qr_android, manual_ios, manual_android) < 0 \
        or not (qr_ios < qr_android < manual_ios < manual_android):
    raise SystemExit("physical manual-entry directions must run after both QR directions")

qr_ios_direction = gate.split("run_ios_owner_android_joiner() {", 1)[1].split(
    "run_android_owner_ios_joiner() {", 1
)[0]
qr_ios_timing = (
    'wait_for_ios_marker "IRIS_IOS_APPROVAL_CONFIRMATION_READY=1"',
    'IOS_TO_ANDROID_STARTED_MS="$(monotonic_milliseconds)"',
    "signal_ios_test submit-camera-approval",
    'wait_for_android_authorized "$WAIT_SECS"',
    'IOS_TO_ANDROID_FINISHED_MS="$(monotonic_milliseconds)"',
    "assert_delivery_bound",
)
if not all(part in qr_ios_direction for part in qr_ios_timing) \
        or [qr_ios_direction.index(part) for part in qr_ios_timing] != sorted(
            qr_ios_direction.index(part) for part in qr_ios_timing
        ):
    raise SystemExit("iOS camera-owner timing does not span host release through authorization")

manual_ios_direction = manual_gate.split(
    "run_ios_owner_android_joiner_manual() {", 1
)[1].split("run_android_owner_ios_joiner_manual() {", 1)[0]
manual_android_direction = manual_gate.split(
    "run_android_owner_ios_joiner_manual() {", 1
)[1].split("write_mobile_linking_summary() {", 1)[0]
for direction, owner_platform in (
    (manual_ios_direction, "ios"),
    (manual_android_direction, "android"),
):
    for required in (
        "restart_android_and_assert_authorized",
        "exchange_post_link_provider_files",
        f"wait_for_empty_receipts {owner_platform}",
        "assert_delivery_bound",
        "assert_authorized_config",
        "assert_ios_provider_content",
    ):
        if required not in direction:
            raise SystemExit(f"physical manual-entry direction lacks {required}")

manual_ios_timing = (
    'wait_for_ios_marker "IRIS_IOS_MANUAL_CONFIRMATION_READY=1"',
    'IOS_TO_ANDROID_MANUAL_STARTED_MS="$(monotonic_milliseconds)"',
    "signal_ios_test submit-manual-approval",
    'wait_for_android_authorized "$WAIT_SECS"',
    'IOS_TO_ANDROID_MANUAL_FINISHED_MS="$(monotonic_milliseconds)"',
)
manual_android_timing = (
    'type_android_manual_approval_request "$request"',
    'ANDROID_TO_IOS_MANUAL_STARTED_MS="$(monotonic_milliseconds)"',
    'tap_android_ui text "Approve"',
    'wait_for_ios_marker "IRIS_IOS_MANUAL_AUTHORIZED=1"',
    'ANDROID_TO_IOS_MANUAL_FINISHED_MS="$(monotonic_milliseconds)"',
)
for direction, timing in (
    (manual_ios_direction, manual_ios_timing),
    (manual_android_direction, manual_android_timing),
):
    if not all(part in direction for part in timing) \
            or [direction.index(part) for part in timing] != sorted(
                direction.index(part) for part in timing
            ):
        raise SystemExit("physical manual-entry timing does not span submission through authorization")

for required in (
    '.get("request_url", "")',
    'copy_android_config "$config"',
    'copy_ios_config "$config"',
    'shell input text "$request"',
    'wait_for_android_ui text "Approve this device?"',
    '"post_link_provider_writes"',
):
    if required not in manual_gate:
        raise SystemExit(f"physical manual-entry production driver lacks {required}")

for test_name in (
    "testPhysicalEnvironmentBridgeIsReady",
    "testIosOwnerApprovesAndroidThroughPhysicalCamera",
    "testIosJoinerDisplaysPhysicalQrAndBecomesAuthorized",
    "testIosOwnerApprovesAndroidThroughManualEntry",
    "testIosJoinerWaitsForAndroidManualApproval",
):
    if f"func {test_name}(" not in ios_test:
        raise SystemExit(f"physical iOS XCTest is missing {test_name}")

owner_test = ios_test.split(
    "func testIosOwnerApprovesAndroidThroughPhysicalCamera()", 1
)[1].split("func testIosJoinerDisplaysPhysicalQrAndBecomesAuthorized()", 1)[0]
approve = 'confirmation.buttons["Approve"].tap()'
release = 'waitForHostSignal("submit-camera-approval")'
authorized = 'waitForHostSignal("peer-authorized")'
post_link = "sharePostLinkFileThroughSystemExtension()"
ready = 'PhysicalLinkMarker.emit("IRIS_IOS_POST_LINK_FILE_READY=1")'
exchange = "completePostLinkExchange()"
if not all(step in owner_test for step in (release, approve, exchange)):
    raise SystemExit("iOS owner test is missing its post-approval Drive write")
post_link_exchange = ios_test.split("private func completePostLinkExchange()", 1)[1].split(
    "private func createProfile()", 1
)[0]
if not all(step in post_link_exchange for step in (authorized, post_link, ready)):
    raise SystemExit("iOS post-link exchange lacks authorization coordination or provider write")
if not (
    owner_test.index(release) < owner_test.index(approve) < owner_test.index(exchange)
    and post_link_exchange.index(authorized)
    < post_link_exchange.index(post_link)
    < post_link_exchange.index(ready)
):
    raise SystemExit("iOS owner Drive write is not coordinated after observed peer authorization")

manual_owner_test = ios_test.split(
    "func testIosOwnerApprovesAndroidThroughManualEntry()", 1
)[1].split("func testIosJoinerWaitsForAndroidManualApproval()", 1)[0]
for required in (
    'decodedEnvironment("IRIS_XCUITEST_MANUAL_REQUEST_B64")',
    "manual.typeText(request)",
    'app.alerts["Approve this device?"]',
    'waitForHostSignal("submit-manual-approval")',
    'confirmation.buttons["Approve"].tap()',
    "completePostLinkExchange()",
):
    if required not in manual_owner_test:
        raise SystemExit(f"iOS manual owner journey is missing {required}")
if manual_owner_test.index('waitForHostSignal("submit-manual-approval")') \
        > manual_owner_test.index('confirmation.buttons["Approve"].tap()'):
    raise SystemExit("iOS manual owner approval is not held behind the host timing barrier")

manual_joiner_test = ios_test.split(
    "func testIosJoinerWaitsForAndroidManualApproval()", 1
)[1].split("private func ", 1)[0]
for required in (
    'startJoiner(requestReadyMarker: "IRIS_IOS_MANUAL_REQUEST_READY=1")',
    'marker: "IRIS_IOS_MANUAL_AUTHORIZED=1"',
    "completePostLinkExchange()",
):
    if required not in manual_joiner_test:
        raise SystemExit(f"iOS manual joiner journey is missing {required}")

for required in (
    "PhysicalLinkTimeouts.delivery",
    "IRIS_XCUITEST_RUN_ID",
    "IRIS_XCUITEST_SIGNAL_PREFIX",
    "IRIS_XCUITEST_STARTED=",
    "IRIS_XCUITEST_FINISHED=",
    'app.descendants(matching: .any)["qrScannerCamera"]',
    'app.descendants(matching: .any)["approvalRequestQr"]',
    "XCUIDevice.shared.press(.home)",
    'app.textFields["manualDeviceId"]',
    "shareFileToIrisDriveButton",
    "Save to Iris Drive",
    r"\(runId)-\(signalPrefix)-\(name).signal",
):
    if required not in ios_test:
        raise SystemExit(f"physical iOS XCTest is missing {required}")

if 'local name="$IOS_SIGNAL_PREFIX-$1"' not in gate:
    raise SystemExit("physical iOS coordination signals are not unique per XCTest")

if "maximum: 15" not in ios_test or "return min(requested, maximum)" not in ios_test:
    raise SystemExit("physical iOS delivery timeout is not capped at 15 seconds")
if '.accessibilityIdentifier("approvalRequestQr")' not in ios_root:
    raise SystemExit("iOS approval request QR lacks a stable shipped selector")
if 'accessibilityIdentifier = "qrScannerCamera"' not in ios_scanner:
    raise SystemExit("iOS camera preview lacks a stable shipped selector")
if 'contentDescription = "Device approval request QR"' not in android_root:
    raise SystemExit("Android approval request QR lacks a stable shipped selector")
if 'contentDescription = "QR scanner camera"' not in android_scanner:
    raise SystemExit("Android camera preview lacks a stable shipped selector")
if 'contentDescription = "Manual device approval request"' not in android_devices:
    raise SystemExit("Android manual approval field lacks a stable shipped selector")

for required in (
    "DocumentsContract.createDocument",
    "openOutputStream",
    "openInputStream",
    "MessageDigest.getInstance(\"SHA-256\")",
    "writePostLinkFileThroughDocumentsProvider",
    "readPostLinkFileThroughDocumentsProvider",
):
    if required not in android_provider_test:
        raise SystemExit(f"Android physical provider test is missing {required}")
if "coldDocumentsContractClientCreatesWritesReadsRenamesAndDeletesWithoutMainActivity" \
        not in android_provider_contract_test:
    raise SystemExit("Android DocumentsProvider lacks a focused cold-process regression")
if "initializeAndroidContext" in android_provider_contract_test:
    raise SystemExit("cold Android DocumentsProvider regression still depends on MainActivity initialization")

for required in (
    "IRIS_DRIVE_MOBILE_LINK_ISOLATED_DEVICE",
    "isolated_device_opt_in_required",
    "devices_not_colocated",
    "reserve_physical_devices",
    'python3 "$ROOT/scripts/native_lab.py" run',
    '--health "android:$ANDROID_SERIAL_SELECTED"',
    '--health "ios-device:$IOS_DEVICE_SELECTED"',
    "IRIS_DRIVE_NATIVE_MOBILE_RESERVED=1",
):
    if required not in physical_gate + full_gate:
        raise SystemExit(f"physical mobile gate lacks explicit skip/routing evidence: {required}")

if "IRIS_DRIVE_MOBILE_PHYSICAL_LINKING" not in full_gate \
        or "scripts/mobile-ios-android-linking-e2e.sh" not in full_gate:
    raise SystemExit("full five-platform gate does not invoke physical mobile linking")
for required in (
    "IRIS_DRIVE_MACOS_ANDROID_MANUAL_LINKING",
    "IRIS_DRIVE_DEV_VM_MACOS_REMOTE",
    'git -C "$ROOT" remote get-url',
    "select_physical_android",
    "reserve_physical_android",
    'python3 "$ROOT/scripts/native_lab.py" run',
    '--health "android:$SERIAL"',
    '--health "ssh:$MAC_HOST"',
    "IRIS_DRIVE_NATIVE_ANDROID_RESERVED=1",
    "--allocation-env android=IRIS_DRIVE_LAB_ALLOCATED_ANDROID",
    "redact_physical_identifiers",
    "IRIS_DRIVE_ANDROID_PHYSICAL_LINK_TEST_PACKAGE",
    'TEST_PACKAGE="${IRIS_DRIVE_ANDROID_PHYSICAL_LINK_TEST_PACKAGE:-${TEST_RUNNER%%/*}}"',
    'uninstall "$TEST_PACKAGE"',
    'uninstall "$PACKAGE"',
    "ro.kernel.qemu",
    "monotonic_milliseconds",
    "host_monotonic_ms",
    "macos-owner-to-physical-android-joiner",
    "physical-android-owner-to-macos-joiner",
    "Manual device approval request",
    "pending_device_approval_receipts",
    "native-fips-status.json",
    "write_android_evidence",
    "write_android_root_sync_evidence",
    "capture_remote_evidence",
    'fips.get("authorized_peers")',
    'fips.get("online_devices")',
    "host_released_approval_submission",
    "tap_android_until_ui",
    "writePostLinkFileThroughDocumentsProvider",
    "readPostLinkFileThroughDocumentsProvider",
    'grep -Fq "INSTRUMENTATION_STATUS: test=$method" "$log"',
    "MACOS_VM_PHYSICAL_ANDROID_BIDIRECTIONAL_MANUAL_LINK_E2E_OK",
    "start_deterministic_relay",
    "restart_deterministic_relay",
    "start_deterministic_blossom",
    "local-blossom-server.py",
    'local-nostr-relay.py" --ready-file',
    '--event-log "$LOCAL_RELAY_EVENT_LOG"',
    'reverse "tcp:$LOCAL_RELAY_PORT" "tcp:$LOCAL_RELAY_PORT"',
    'ExitOnForwardFailure=yes',
    'IRIS_DRIVE_MACOS_ANDROID_RELAY_URL',
    "configure_android_singleton_relay",
    'android_relay_action replace-relays "$LOCAL_RELAY_URL"',
    'android_relay_action replace-blossom "$LOCAL_BLOSSOM_URL"',
    "android_blossom_is_singleton",
    "write_blossom_summary",
    "android_type_text_in_chunks",
    'chunk="${remaining:0:32}"',
    "write_android_relay_evidence",
    'copy_android_file config.toml "$TMP/android-config.toml"',
    "ast.literal_eval",
    "write_relay_summary",
    '"$RESULT_DIR/android-owner-root-sync-evidence.json"',
):
    if required not in macos_android_contract:
        raise SystemExit(f"physical macOS/Android manual-link gate is missing {required}")
reverse_direction = macos_android_gate.split(
    "# Physical Android owner -> macOS joiner.", 1
)[1]
read_provider_branch = macos_android_gate.split(
    'if [[ "$method" == readPostLinkFileThroughDocumentsProvider ]]', 1
)[1].split("elif !", 1)[0]
if read_provider_branch.index("am instrument") \
        > read_provider_branch.index("start_android_app"):
    raise SystemExit("Android live-read instrumentation must start before the app sync runtime")
if "restart_deterministic_relay" not in reverse_direction.split(
    "create_android_owner", 1
)[0]:
    raise SystemExit(
        "physical macOS/Android reverse direction must discard stale first-direction relay discovery"
    )
for required in (
    "stop_first_direction_actors",
    "prepare_android_actor",
    "wait_android_process_stopped",
    "wait_actor_adverts \"$ANDROID_OWNER_NPUB\" \"$MAC_JOINER_NPUB\"",
    "assert_relay_epoch_adverts \"$ANDROID_OWNER_NPUB\" \"$MAC_JOINER_NPUB\"",
    "online and (direct or mesh)",
):
    if required not in macos_android_gate:
        raise SystemExit(
            f"physical macOS/Android epoch lifecycle is missing {required}"
        )
if "authenticated_transport=any(" not in macos_android_remote:
    raise SystemExit("macOS exact peer readiness must require direct or mesh transport")
if reverse_direction.index("stop_first_direction_actors") \
        > reverse_direction.index("restart_deterministic_relay"):
    raise SystemExit("first-direction actors must stop before the relay epoch changes")
prepare_android_actor = macos_android_gate.split(
    "prepare_android_actor() {", 1
)[1].split("\n}", 1)[0]
if prepare_android_actor.index("configure_android_singleton_relay") \
        > prepare_android_actor.rindex('am force-stop "$PACKAGE"'):
    raise SystemExit("Android singleton relay must persist before the actor is stopped for launch")
if prepare_android_actor.rindex('am force-stop "$PACKAGE"') \
        > prepare_android_actor.index("wait_android_process_stopped"):
    raise SystemExit("Android config bootstrap must be fully stopped before actor launch")
for forbidden in (
    "int(f.get(\"connected_peer_count\") or 0) > 0",
    "status_fips_ready",
):
    if forbidden in macos_android_gate + macos_android_remote:
        raise SystemExit(f"delivery readiness may not use anonymous peer counts via {forbidden}")
for forbidden in (
    'copy_android_file config.toml "$RESULT_DIR',
    'copy_android_file native-fips-status.json "$RESULT_DIR',
    "copy_remote_artifacts",
    "scp -qr",
    'local role="$1" evidence=',
    'local log="$RESULT_DIR',
    'screencap -p >"$RESULT_DIR',
    "remote status owner >",
    "remote status joiner >",
):
    if forbidden in macos_android_gate:
        raise SystemExit(f"physical macOS/Android artifacts retain private state via {forbidden}")
cleanup_branch = macos_android_gate.split("cleanup() {", 1)[1].split("trap cleanup EXIT", 1)[0]
for required in (
    "trap - EXIT",
    "set +e",
    "capture_remote_evidence owner || true",
    "capture_remote_root_sync_evidence owner || true",
    "capture_remote_root_sync_evidence joiner || true",
    "remote cleanup",
):
    if required not in cleanup_branch:
        raise SystemExit(f"physical macOS/Android failure cleanup is missing {required}")
if cleanup_branch.index("capture_remote_root_sync_evidence owner || true") \
        > cleanup_branch.index("remote cleanup"):
    raise SystemExit("macOS root-sync failure evidence must be retained before remote cleanup")
for required in (
    '"drive_root_events"',
    '"requests"',
    "transport_phase_counts",
    "write_provider_publish_phase",
    '"$RESULT_DIR/android-provider-publish-phase.json"',
    "Android provider mutation did not publish a Drive-root event",
    "Android provider mutation did not attempt a singleton Blossom upload",
):
    if required not in macos_android_gate:
        raise SystemExit(f"physical macOS/Android transport phase evidence is missing {required}")
prepare_index = macos_android_gate.index('owner_fixture="$(remote prepare)"')
if macos_android_gate.rfind("REMOTE_DIRTY=1", 0, prepare_index) < 0:
    raise SystemExit("physical macOS/Android cleanup is not armed before remote prepare")
manual_wait = re.search(
    r'WAIT_SECS="\$\{IRIS_DRIVE_MACOS_ANDROID_WAIT_SECS:-(\d+)\}"',
    macos_android_gate,
)
if not manual_wait or int(manual_wait.group(1)) > 15:
    raise SystemExit("physical macOS/Android delivery must default to at most 15 seconds")
if any('scripts/lib/android-ui-point.py' not in harness for harness in (gate, macos_android_gate)):
    raise SystemExit("physical Android UI harnesses do not share the clickable-node selector")
for required in ('target.attrib.get("clickable")', "fallback = fallback or point"):
    if required not in android_ui_point:
        raise SystemExit(f"Android UI selector does not prefer clickable nodes: {required}")
for required in (
    "start-owner",
    "start-joiner",
    "manual-prepare",
    "manual-submit",
    "link-ready",
    "peer-evidence",
    "evidence",
    "root-sync-evidence",
    "run_ax_action",
    '/usr/bin/swift "$ROOT/scripts/macos-device-link-ax.swift" "$@"',
    "assert-joined-ui",
    "configure_singleton_relay",
    'IRIS_DRIVE_MACOS_ANDROID_RELAY_URL',
    "fips_online",
    'rm -rf "$ARTIFACT_DIR"',
    'return "$status"',
):
    if required not in macos_android_remote:
        raise SystemExit(f"macOS VM manual-link remote driver is missing {required}")
if 'ARTIFACT_DIR="$ROOT/artifacts/macos-android-manual-link"' not in macos_android_remote:
    raise SystemExit("macOS VM manual-link private state must use the fixed cleanup root")
for forbidden in (
    "owner-initial-status.json",
    "tail -100 \"$log.stderr.log\"",
    "AX_DRIVER=",
    "/usr/bin/swiftc",
):
    if forbidden in macos_android_remote:
        raise SystemExit(f"macOS VM manual-link driver exposes raw private state via {forbidden}")
prepare_branch = macos_android_remote.split("  prepare)", 1)[1].split("    ;;", 1)[0]
fresh_build = 'IRIS_DRIVE_MACOS_SIGNING="${IRIS_DRIVE_MACOS_SIGNING:-none}" '
if fresh_build not in prepare_branch \
        or prepare_branch.index(fresh_build) > prepare_branch.index("load_candidate"):
    raise SystemExit("macOS VM manual-link prepare does not build the synced candidate")
for required in (
    "ManualPrepare",
    "SignIn",
    "AssertJoined",
    '"sidebarDevices"',
    '"addDeviceToggle"',
    '"manualDeviceApprovalInput"',
    '"deviceApprovalApprove"',
    '"deviceApprovalCancel"',
    "AXUIElementCopyActionNames",
    "AXUIElementGetPid",
    ".postToPid(pid)",
    "text(field, kAXValueAttribute) == value",
    "case actionOutcomeUnknown(String, AXError)",
    "error == .cannotComplete || error == .attributeUnsupported",
    "exit(76)",
):
    if required not in macos_ax:
        raise SystemExit(f"macOS shipped UI driver is missing {required}")
for required in (
    "profile_roster_size",
    "approval_roster_advanced",
    '[[ "$status" == 76 ]]',
    "MACOS_DEVICE_LINK_CONFIRMATION_APPROVE_STATE_ADVANCED_OK",
):
    if required not in macos_android_remote:
        raise SystemExit(
            f"macOS VM manual-link driver does not classify an AX-invalidated successful approval: {required}"
        )
manual_submit = macos_android_remote.split("  manual-submit)", 1)[1].split("    ;;", 1)[0]
if manual_submit.count('run_ax_action "macOS shipped approval submission"') != 1:
    raise SystemExit("macOS approval recovery must never press Approve more than once")
if 'if run_ax_action "macOS shipped approval submission"' not in manual_submit:
    raise SystemExit("macOS approval recovery must capture ambiguous AX status without errexit")
if '"replace-relays" ->' not in android_activity:
    raise SystemExit("physical Android harness cannot atomically replace public relay fallback through production actions")
if '"replace-blossom" ->' not in android_activity:
    raise SystemExit("physical Android harness cannot atomically replace public Blossom fallback through production actions")
for required in (
    "IRIS_MACOS_AX_STAGE=",
    '"manual_devices"',
    'reportStage("\\(stage)_waiting")',
    '"manual_add_device"',
    'reportStage("\\(stage)_pressed")',
    "manual_request_field_waiting",
    "manual_request_entered",
    "manual_confirmation_ready",
    "category=missing_element",
    "category=missing_dialog",
    "catch LinkDriverError.accessibilityPermission",
    "exit(75)",
):
    if required not in macos_ax:
        raise SystemExit(f"macOS shipped UI driver lacks sanitized stage evidence: {required}")
if "visibleSummary" in macos_ax:
    raise SystemExit("macOS shipped UI driver can expose raw visible UI text")
for forbidden in (
    'dialogButton(',
    'kAXWindowsAttribute',
    '"AXDialog"',
    '.post(tap: .cghidEventTap)',
):
    if forbidden in macos_ax:
        raise SystemExit(f"macOS shipped UI driver retains brittle global AX behavior: {forbidden}")
if "scripts/macos-vm-android-manual-link-e2e.sh" not in full_gate:
    raise SystemExit("five-platform gate does not route macOS/Android manual linking")
for required in ("scripts/macos-vm-smoke.sh", "scripts/macos-vm-idle-cpu.sh"):
    if required not in release_gate:
        raise SystemExit(f"release gate does not route macOS UI work through {required}")
if "./scripts/check-mobile-physical-linking-e2e.sh" not in justfile:
    raise SystemExit("structure gate does not enforce physical mobile linking")
if "IrisDrivePhysicalLinkingUITests.swift" not in xcode_project:
    raise SystemExit("physical linking XCTest is absent from the checked-in Xcode project")

PY

contract_tmp="$(mktemp -d "${TMPDIR:-/tmp}/iris-drive-mobile-cleanup-contract.XXXXXX")"
cleanup_contract_tmp() {
  [[ ! -d "$contract_tmp" ]] || rm -rf "$contract_tmp"
}
trap cleanup_contract_tmp EXIT

capture_function="$(sed -n '/^capture_remote_evidence() {/,/^}/p' \
  "$ROOT/scripts/macos-vm-android-manual-link-e2e.sh")"
bash -Eeuo pipefail -c '
eval "$1"
TMP="$2/private"
RESULT_DIR="$2/result"
mkdir -p "$TMP" "$RESULT_DIR"
remote() { printf "{\"ok\":true}\n"; }
capture_remote_evidence owner
[[ -s "$RESULT_DIR/macos-owner-evidence.json" ]]
' bash "$capture_function" "$contract_tmp"

cleanup_function="$(sed -n '/^cleanup() {/,/^}/p' \
  "$ROOT/scripts/macos-vm-android-manual-link-e2e.sh")"
bash -Eeuo pipefail -c '
eval "$1"
TMP="$2/private"
RESULT_DIR="$2/result"
CONTRACT_TMP="$2"
REMOTE_DIRTY=1
ADB=""
SERIAL=""
PACKAGE=""
capture_remote_evidence() { return 127; }
remote() { [[ "$1" != cleanup ]] || : >"$CONTRACT_TMP/remote-cleanup-ran"; }
cleanup
' bash "$cleanup_function" "$contract_tmp"
[[ -f "$contract_tmp/remote-cleanup-ran" ]]

mock_adb="$contract_tmp/mock-adb"
cat >"$mock_adb" <<'SH'
#!/usr/bin/env bash
printf '%s\n' "$*" >>"$CONTRACT_ADB_LOG"
SH
chmod +x "$mock_adb"
CONTRACT_ADB_LOG="$contract_tmp/adb.log" bash -Eeuo pipefail -c '
eval "$1"
TMP="$2/private-device-cleanup"
RESULT_DIR="$2/result"
REMOTE_DIRTY=0
ADB="$3"
SERIAL=contract-android
PACKAGE=to.example.irisdrive.uitest
TEST_PACKAGE=to.example.irisdrive.uitest.test
cleanup
' bash "$cleanup_function" "$contract_tmp" "$mock_adb"
grep -Fq 'uninstall to.example.irisdrive.uitest.test' "$contract_tmp/adb.log"
grep -Fq 'uninstall to.example.irisdrive.uitest' "$contract_tmp/adb.log"

physical_functions="$(sed -n \
  -e '/^ANDROID_FIXTURE_OWNED=/p' \
  -e '/^cleanup() {/,/^}/p' \
  -e '/^prepare_android_fresh() {/,/^}/p' \
  -e '/^start_android_app() {/,/^}/p' \
  -e '/^bool_true() {/,/^}/p' \
  -e '/^emit_skip() {/,/^}/p' \
  -e '/^reserve_physical_devices() {/,/^}/p' \
  "$ROOT/scripts/mobile-ios-android-linking-e2e.sh")"
for scenario in unowned auto-skip readiness-failure delegated-parent owned owned-keep; do
  adb_log="$contract_tmp/$scenario-adb.log"
  : >"$adb_log"
  expected_status=0
  case "$scenario" in
    readiness-failure) expected_status=23 ;;
    delegated-parent) expected_status=17 ;;
  esac
  if CONTRACT_ADB_LOG="$adb_log" bash -Eeuo pipefail -c '
    eval "$1"
    ROOT="$2"
    TMP="$2/$4-private"
    RESULT_DIR="$2/results"
    RUN_ID="$4"
    MODE=auto
    IOS_TEST_PID=""
    IOS_TEST_RUN_FILE=""
    IOS_DEVICE_SELECTED=contract-ios
    ADB="$3"
    ANDROID_SERIAL_SELECTED=contract-android
    ANDROID_PACKAGE=to.example.irisdrive.uitest
    ANDROID_ACTIVITY="$ANDROID_PACKAGE/.MainActivity"
    IRIS_DRIVE_MOBILE_LINK_KEEP_ANDROID_APP=0
    mkdir -p "$TMP"
    trap cleanup EXIT
    case "$4" in
      unowned) ;;
      auto-skip) emit_skip isolated_device_opt_in_required ;;
      readiness-failure) exit 23 ;;
      delegated-parent)
        IRIS_DRIVE_NATIVE_MOBILE_RESERVED=0
        IRIS_DRIVE_LAB_ALLOCATED_ANDROID=""
        IRIS_DRIVE_LAB_ALLOCATED_IOS=""
        python3() { : >"$ROOT/delegated-child-ran"; return 17; }
        redact_physical_identifiers() { cat; }
        reserve_physical_devices
        exit 99
        ;;
      owned) prepare_android_fresh ;;
      owned-keep)
        prepare_android_fresh
        IRIS_DRIVE_MOBILE_LINK_KEEP_ANDROID_APP=1
        ;;
    esac
  ' bash "$physical_functions" "$contract_tmp" "$mock_adb" "$scenario"; then
    actual_status=0
  else
    actual_status=$?
  fi
  [[ "$actual_status" -eq "$expected_status" ]] || {
    echo "physical cleanup changed $scenario exit status: $actual_status" >&2
    exit 1
  }
  [[ ! -d "$contract_tmp/$scenario-private" ]]
  expected_log="$contract_tmp/$scenario-expected.log"
  : >"$expected_log"
  if [[ "$scenario" == owned* ]]; then
    for action in 'am force-stop' 'pm clear' 'am start -W -n' 'am force-stop'; do
      target=to.example.irisdrive.uitest
      [[ "$action" != 'am start -W -n' ]] || target+=/.MainActivity
      printf '%s\n' "-s contract-android shell $action $target" >>"$expected_log"
    done
    if [[ "$scenario" == owned ]]; then
      printf '%s\n' '-s contract-android shell pm clear to.example.irisdrive.uitest' >>"$expected_log"
    fi
  fi
  diff -u "$expected_log" "$adb_log"
done
[[ -f "$contract_tmp/delegated-child-ran" ]]
grep -Fq '"reason": "isolated_device_opt_in_required"' \
  "$contract_tmp/results/mobile-linking-auto-skip-skipped.json"

cleanup_contract_tmp
trap - EXIT
echo "MOBILE_PHYSICAL_LINKING_E2E_CONTRACT_OK"
