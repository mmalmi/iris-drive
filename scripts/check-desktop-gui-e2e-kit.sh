#!/usr/bin/env bash

set -Eeuo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"

require_file_contains() {
  local file="$1"
  local pattern="$2"
  if ! grep -F "$pattern" "$ROOT/$file" >/dev/null; then
    echo "missing '$pattern' in $file" >&2
    exit 1
  fi
}

require_file_absent() {
  local file="$1"
  local pattern="$2"
  if grep -F "$pattern" "$ROOT/$file" >/dev/null; then
    echo "unexpected '$pattern' in $file" >&2
    exit 1
  fi
}

require_file_contains scripts/desktop-gui-smoke.sh "xdotool search --onlyvisible --name '^Iris Drive$'"
require_file_contains scripts/desktop-gui-smoke.sh "Xvfb"
require_file_contains scripts/desktop-gui-smoke.sh "IRIS_DRIVE_DISABLE_TRAY=1"
require_file_contains scripts/desktop-gui-smoke.sh "IRIS_DRIVE_DEV_VM_LINUX_CONFIG_DIR="
require_file_contains scripts/desktop-gui-smoke.sh "IRIS_DRIVE_DEV_VM_LINUX_MOUNTPOINT="
require_file_contains scripts/desktop-gui-smoke.sh "IRIS_DRIVE_DEV_VM_WINDOWS_CONFIG_DIR"
require_file_contains scripts/desktop-gui-smoke.sh "authorized_app_key_count"
require_file_contains scripts/desktop-gui-smoke.sh "UIAutomationClient"
require_file_contains scripts/desktop-gui-smoke.sh "InvokePattern"
require_file_contains scripts/desktop-gui-smoke.sh "requires an unlocked interactive desktop session"
require_file_contains scripts/desktop-gui-smoke.sh "Test-VisibleWindowLaunch"
require_file_contains scripts/desktop-gui-smoke.sh "IrisDriveGuiSmokeInteractive"
require_file_contains scripts/desktop-gui-smoke.sh "Wait-ShellReady"
require_file_contains scripts/desktop-gui-smoke.sh "IRIS_DRIVE_WINDOWS_GUI_READY_TIMEOUT_SECS"
require_file_contains scripts/desktop-gui-smoke.sh 'AddSeconds($ShellReadyTimeoutSeconds)'
require_file_contains scripts/desktop-gui-smoke.sh "Windows GUI smoke requires a desktop session that exposes visible windows"
require_file_contains scripts/desktop-gui-smoke.sh "IRIS_DRIVE_DESKTOP_GUI_LAUNCH_LINK"
require_file_contains scripts/desktop-gui-smoke.sh "IRIS_DRIVE_DESKTOP_GUI_EXPECTED_STATE"
require_file_contains scripts/desktop-gui-smoke.sh "approval_queued"
require_file_contains scripts/desktop-gui-smoke.sh "python3 -c 'import pyatspi'"
require_file_contains scripts/desktop-gui-smoke.sh 'scripts/lib/linux-approve-device.py'
require_file_contains scripts/lib/linux-approve-device.py 'APPROVAL_DIALOG_TITLE = "Approve this device?"'
require_file_contains scripts/lib/linux-approve-device.py 'pyatspi.ROLE_ALERT'
require_file_contains scripts/lib/linux-approve-device.py 'pyatspi.ROLE_DIALOG'
require_file_contains scripts/lib/linux-approve-device.py 'find_approve_button(dialog)'
require_file_contains scripts/desktop-gui-smoke.sh 'Confirm-ApprovalDialog'
require_file_contains scripts/desktop-gui-smoke.sh 'if ($ExpectedState -eq "authorized") {'
require_file_contains scripts/desktop-gui-smoke.sh 'Invoke-Button $Dialog "Yes"'
require_file_contains scripts/dev-vm-update-run.sh "building Linux GTK app"
require_file_contains scripts/dev-vm-update-run.sh "skipping Windows app GUI launch"
require_file_contains scripts/dev-vm-smoke.sh "run_linux_ui_smoke"
require_file_contains scripts/dev-vm-smoke.sh "run_windows_ui_smoke"
require_file_contains scripts/dev-vm-smoke.sh "desktop-ui"
require_file_contains scripts/dev-vm-smoke.sh "linux-ui"
require_file_contains scripts/dev-vm-smoke.sh "windows-ui"
require_file_contains scripts/e2e-everything-3vms.sh "linux-ui, windows-ui, desktop-ui"
require_file_contains scripts/cross-vm-five-platform-e2e.sh "running Linux GTK GUI smoke"
require_file_contains scripts/cross-vm-five-platform-e2e.sh "running Windows WPF GUI smoke"
require_file_contains scripts/cross-vm-five-platform-e2e.sh 'IRIS_DRIVE_E2E_DESKTOP_GUI_LINKING="${IRIS_DRIVE_E2E_DESKTOP_GUI_LINKING:-1}"'
require_file_contains scripts/cross-vm-e2e.sh 'source "$ROOT/scripts/lib/cross-vm-device-link.sh"'
require_file_contains scripts/cross-vm-e2e.sh 'LINK_TIMEOUT_SECS="${IRIS_DRIVE_E2E_LINK_TIMEOUT_SECS:-15}"'
require_file_contains scripts/cross-vm-e2e.sh 'run_step "bidirectional desktop GUI linking" run_bidirectional_desktop_gui_linking'
require_file_contains scripts/cross-vm-e2e.sh 'desktop_gui_primary_request_url="$request_url"'
require_file_contains scripts/cross-vm-e2e.sh 'run_timed_desktop_gui_primary_approval "$desktop_gui_primary_request_url"'
require_file_contains scripts/lib/cross-vm-device-link.sh "pending_device_approval_receipt_count"
require_file_absent scripts/lib/cross-vm-device-link.sh "pending_device_approval_receipt_count // 0"
require_file_contains scripts/lib/cross-vm-device-link.sh '[[ -z "$DESKTOP_GUI_PRIMARY_LINK_STARTED_AT" ]] || return 0'
require_file_contains scripts/lib/cross-vm-device-link.sh "late success after"
require_file_contains scripts/lib/cross-vm-device-link.sh "Windows WPF join -> Linux GTK approval"
require_file_contains scripts/lib/cross-vm-device-link.sh "Linux GTK join -> Windows WPF approval"
require_file_contains scripts/lib/cross-vm-device-link.sh "run_bidirectional_desktop_gui_linking"
require_file_contains scripts/lib/cross-vm-device-link.sh "monotonic_milliseconds"
require_file_contains scripts/lib/cross-vm-device-link.sh ".fips_direct_online == true"
require_file_contains scripts/lib/cross-vm-device-link.sh "run_timed_desktop_gui_primary_approval"
require_file_contains scripts/lib/cross-vm-device-link.sh $'while ! grep -Fq '\''IRIS_DRIVE_DESKTOP_GUI_APPROVAL_STARTED=1'\'' "$output"; do'
require_file_contains scripts/lib/cross-vm-device-link.sh $'  mark_desktop_gui_primary_approval_submission\n  wait "$action_pid" || status=$?'
require_file_contains scripts/desktop-gui-smoke.sh "IRIS_DRIVE_DESKTOP_GUI_APPROVAL_SUBMITTED=1"
require_file_absent scripts/lib/cross-vm-device-link.sh 'date +%s'
require_file_contains scripts/macos-smoke.sh "IRIS_DRIVE_DEBUG_LOG_DIR"
require_file_contains windows/App.xaml.cs "using var writer = new StreamWriter(client, new UTF8Encoding(false));"
require_file_contains windows/MainWindow.xaml.cs "if (launchArguments.Length == 0)"
require_file_contains windows/MainWindow.xaml.cs "ShowFromTray();"
require_file_contains macos/Sources/IrisDriveMacApp.swift "controlPanelWindow"
require_file_contains macos/Sources/IrisDriveMacApp.swift "irisDriveDebugLog(\"Iris Drive menu bar item installed\")"
require_file_contains macos/Sources/IrisDriveSetupViews.swift "Copy Request Link"
require_file_contains macos/Sources/IrisDriveSetupViews.swift "awaitingApprovalBack"
require_file_absent macos/Sources/IrisDriveSetupViews.swift "Start over"
require_file_contains macos/Sources/IrisDriveSetupViews.swift "openRecoveryPhrase"
require_file_contains macos/Sources/IrisDriveSetupViews.swift "openSecretKey"
require_file_contains macos/Sources/IrisDriveMacApp.swift '"type": "start_join_request"'
require_file_contains macos/Sources/IrisDriveMacApp.swift "forceRestart: true"
require_file_contains macos/Sources/IrisDriveControlPanel.swift "Request link or device ID"
require_file_contains macos/Sources/IrisDriveControlPanel.swift "scanApprovalRequestQr"
require_file_contains macos/Sources/IrisDriveControlPanel.swift "Approve this device?"
require_file_contains macos/Sources/IrisDriveControlPanel.swift '.accessibilityIdentifier(tab == .peers ? "sidebarDevices"'
require_file_contains macos/Sources/IrisDriveControlPanel.swift 'accessibilityIdentifier("manualDeviceApprovalInput")'
require_file_contains macos/Sources/IrisDriveControlPanel.swift 'accessibilityIdentifier("deviceApprovalApprove")'
require_file_contains macos/Sources/IrisDriveControlPanel.swift 'accessibilityIdentifier("deviceApprovalCancel")'
require_file_contains macos/Sources/IrisDriveControlPanel.swift 'accessibilityIdentifier("welcomeSignIn")'
require_file_contains macos/Sources/IrisDriveControlPanel.swift 'accessibilityIdentifier("driveTitle")'
require_file_contains macos/Sources/IrisDriveControlPanel.swift 'accessibilityIdentifier("addDeviceToggle")'
require_file_contains macos/Sources/IrisDriveMacApp.swift '"app_key_approval"'
require_file_contains macos/Sources/IrisDriveMacApp.swift 'pendingDeviceApproval = IrisDriveDeviceApprovalRequest('
require_file_absent macos/Sources/IrisDriveMacApp.swift 'approveDevice(url.absoluteString, label: "")'
require_file_contains macos/Sources/IrisDriveMacApp.swift 'self.refreshExternalDaemonStatus(paths: paths)'
require_file_absent macos/Sources/IrisDriveMacApp.swift 'self.refreshExternalDaemonStatusFile(paths: paths)'
python3 - "$ROOT/macos/Sources/IrisDriveMacApp.swift" <<'PY'
import sys
source=open(sys.argv[1], encoding="utf-8").read()
restart=source.split("if restartSyncAfterSuccess {", 1)[1].split("completion?()", 1)[0]
external=restart.split("if self.externalDaemonMode {", 1)[1].split("} else if", 1)[0]
if "self.startDaemon(" not in external:
    raise SystemExit("external-daemon profile setup does not start its status watcher")
PY
require_file_contains macos/Sources/IrisDriveControlPanel.swift '.onChange(of: status.pendingDeviceApproval?.id, initial: true)'
require_file_contains macos/Sources/IrisDriveControlPanel.swift '(.peers, true, request.requestURL)'
require_file_contains macos/Sources/IrisDriveControlPanel.swift 'confirmApproveDevice(request.requestURL, force: true)'
require_file_contains scripts/macos-smoke.sh 'source "$ROOT/scripts/lib/macos-device-link-smoke.sh"'
require_file_contains scripts/macos-smoke.sh 'source "$ROOT/scripts/lib/macos-smoke-diagnostics.sh"'
require_file_contains scripts/macos-smoke.sh 'source "$ROOT/scripts/lib/macos-app-launch-smoke.sh"'
require_file_contains scripts/macos-smoke.sh 'launch_macos_smoke_app_with_targeted_recovery'
require_file_contains scripts/macos-smoke.sh 'run_macos_owner_device_link_journey'
require_file_contains scripts/lib/macos-device-link-smoke.sh 'MACOS_OWNER_LINK_TIMEOUT_SECS:-15'
require_file_contains scripts/lib/macos-device-link-smoke.sh 'pending_device_approval_receipt_count") == 0'
require_file_contains scripts/lib/macos-device-link-smoke.sh 'macos-device-link-ax.swift" "$app_pid" Cancel'
require_file_contains scripts/lib/macos-device-link-smoke.sh 'macos-device-link-ax.swift" "$app_pid" Approve'
require_file_contains scripts/lib/macos-device-link-smoke.sh 'provider write'
require_file_contains scripts/lib/macos-device-link-smoke.sh 'provider read'
require_file_contains scripts/lib/macos-device-link-smoke.sh 'macos_owner_link_monotonic_milliseconds'
require_file_absent scripts/lib/macos-device-link-smoke.sh 'date +%s'
require_file_contains scripts/macos-device-link-ax.swift 'kAXPressAction'
require_file_contains scripts/macos-device-link-ax.swift 'ManualPrepare'
require_file_contains scripts/macos-device-link-ax.swift 'kAXIdentifierAttribute'
require_file_contains scripts/macos-device-link-ax.swift 'kAXHiddenAttribute'
require_file_contains scripts/macos-device-link-ax.swift 'AXUIElementCopyActionNames'
require_file_contains scripts/macos-device-link-ax.swift 'AXUIElementGetPid'
require_file_contains scripts/macos-device-link-ax.swift '.postToPid(pid)'
require_file_contains scripts/macos-device-link-ax.swift 'text(field, kAXValueAttribute) == value'
require_file_contains scripts/macos-device-link-ax.swift '"sidebarDevices"'
require_file_contains scripts/macos-device-link-ax.swift '"manualDeviceApprovalInput"'
require_file_contains scripts/macos-device-link-ax.swift '"deviceApprovalApprove"'
require_file_contains scripts/macos-device-link-ax.swift '"deviceApprovalCancel"'
require_file_absent scripts/macos-device-link-ax.swift 'dialogButton('
require_file_absent scripts/macos-device-link-ax.swift 'kAXWindowsAttribute'
require_file_absent scripts/macos-device-link-ax.swift '"AXDialog"'
require_file_absent scripts/macos-device-link-ax.swift '.post(tap: .cghidEventTap)'
require_file_contains scripts/macos-vm-smoke.sh 'macos-vm-git-sync.sh'
require_file_contains scripts/macos-smoke.sh 'SMOKE_STATE_DIR="$(mktemp -d -t iris-drive-macos-smoke-state)"'
require_file_contains scripts/macos-smoke.sh 'SMOKE_HOME="$SMOKE_STATE_DIR/home"'
require_file_contains scripts/macos-smoke.sh 'APP_DEBUG_LOG_DIR="$SMOKE_STATE_DIR/logs"'
require_file_contains scripts/macos-smoke.sh 'remove_smoke_path_best_effort "$SMOKE_STATE_DIR"'
require_file_contains scripts/macos-smoke.sh 'trap bootstrap_cleanup EXIT'
require_file_contains scripts/macos-smoke.sh 'assert_safe_smoke_root "$SMOKE_DIR"'
require_file_contains scripts/macos-smoke.sh 'assert_safe_smoke_root "$SMOKE_STATE_DIR"'
require_file_contains scripts/macos-smoke.sh 'assert_safe_app_data_root "$SMOKE_APP_DATA"'
require_file_contains scripts/macos-smoke.sh 'for artifact in "$SMOKE_DIR"/*; do'
require_file_contains scripts/macos-smoke.sh '>"$SMOKE_DIR/result.json"'
require_file_absent scripts/macos-smoke.sh 'SMOKE_HOME="$SMOKE_DIR/home"'
require_file_absent scripts/macos-smoke.sh 'echo "$status_json"'

LAUNCH_LIFECYCLE="$ROOT/scripts/lib/macos-app-launch-smoke.sh"
require_file_contains scripts/lib/macos-app-launch-smoke.sh 'macos_launch_stall_is_retryable'
require_file_contains scripts/lib/macos-app-launch-smoke.sh 'linkd.autoShortcut'
require_file_contains scripts/lib/macos-app-launch-smoke.sh 'MACOS_SMOKE_APP_READY_TIMEOUT_SECS=10'
require_file_contains scripts/lib/macos-app-launch-smoke.sh 'terminate_app_process'
require_file_absent scripts/lib/macos-app-launch-smoke.sh 'rm -rf'

bash - "$LAUNCH_LIFECYCLE" <<'BASH'
set -Eeuo pipefail
source "$1"
app_is_running() { return 0; }
app_process_pids() { printf '123\n'; }
macos_app_has_product_log() { return 1; }
macos_appintents_linkd_stall_for_pid() { [[ "$1" == 123 ]]; }
macos_launch_stall_is_retryable
macos_app_has_product_log() { return 0; }
if macos_launch_stall_is_retryable; then
  echo "product startup output must forbid AppIntents relaunch" >&2
  exit 1
fi
macos_app_has_product_log() { return 1; }
app_is_running() { return 1; }
if macos_launch_stall_is_retryable; then
  echo "app crash must forbid AppIntents relaunch" >&2
  exit 1
fi
BASH

python3 - "$LAUNCH_LIFECYCLE" <<'PY'
from pathlib import Path
import sys

source = Path(sys.argv[1]).read_text()
body = source.split("launch_macos_smoke_app_with_targeted_recovery() {", 1)[1]
if body.count("launch_macos_smoke_app_once") != 2:
    raise SystemExit("targeted launch recovery must allow exactly one relaunch")
if body.index("macos_launch_stall_is_retryable") > body.index("terminate_app_process"):
    raise SystemExit("launch recovery must prove the AppIntents stall before termination")
PY

python3 - "$ROOT/scripts/cross-vm-e2e.sh" <<'PY'
from pathlib import Path
import sys

source = Path(sys.argv[1]).read_text()
markers = [
    'desktop_gui_primary_request_url="$request_url"',
    'configure_fips_static_hints',
    'start_daemon "$label"',
    'run_timed_desktop_gui_primary_approval "$desktop_gui_primary_request_url"',
    'wait_for_all_linking_complete',
]
positions = [source.rindex(marker) for marker in markers]
if positions != sorted(positions):
    raise SystemExit(
        "desktop GUI link timing must start after peer setup/daemon startup and "
        "immediately before owner approval"
    )
PY
require_file_contains scripts/macos-vm-smoke.sh 'REMOTE_PRIVATE_LOG="artifacts/macos-smoke-runner.private.log"'
require_file_contains scripts/macos-vm-smoke.sh 'state_dir=\$(mktemp -d -t iris-drive-macos-vm-smoke-state)'
require_file_contains scripts/macos-vm-smoke.sh 'IRIS_DRIVE_MACOS_SMOKE_STATE_DIR=\"\$state_dir\"'
require_file_contains scripts/macos-vm-smoke.sh 'rm -rf \"\$state_dir\"'
require_file_contains scripts/macos-vm-smoke.sh 'smoke_exit=\$?'
require_file_contains scripts/macos-vm-smoke.sh 'if (( smoke_exit == 0 )); then rm -f '\''$REMOTE_PRIVATE_LOG'\''; fi'
require_file_contains scripts/macos-vm-smoke.sh 'private raw log retained on the VM'
require_file_contains scripts/macos-vm-smoke.sh 'private phase diagnostics retained on the VM'
require_file_contains scripts/macos-vm-smoke.sh '$REMOTE_ARTIFACT_DIR/result.json'
require_file_absent scripts/macos-vm-smoke.sh 'scp -qr'
require_file_absent scripts/macos-vm-smoke.sh 'xcode-build.log'
require_file_absent scripts/macos-vm-smoke.sh 'tail -200'
require_file_contains scripts/macos-vm-idle-cpu.sh 'macos-vm-git-sync.sh'
require_file_contains scripts/release-gate.sh 'IRIS_DRIVE_MACOS_SSH_HOST'
require_file_contains scripts/release-gate.sh './scripts/macos-vm-smoke.sh'
require_file_contains scripts/release-gate.sh './scripts/macos-vm-idle-cpu.sh'
require_file_absent macos/Sources/IrisDriveSetupViews.swift 'keyedValue("Device"'
require_file_absent macos/Sources/IrisDriveControlPanel.swift "Device invite link"
require_file_absent macos/Sources/IrisDriveControlPanel.swift "Copy invite link"
require_file_absent macos/Sources/IrisDriveControlPanel.swift "Reset invite"
require_file_contains linux/src/setup.rs "Copy Request Link"
require_file_contains linux/src/setup.rs "setup_back_button"
require_file_absent linux/src/setup.rs "Start over"
require_file_contains linux/src/setup.rs "open_recovery_phrase_setup"
require_file_contains linux/src/setup.rs "open_secret_key_setup"
require_file_contains linux/src/setup.rs "start_join_request()"
require_file_contains linux/src/ui.rs "Request link or device ID"
require_file_contains linux/src/actions.rs "Approve this device?"
require_file_absent linux/src/actions.rs 'approve_device_values(model, request, String::new());'
require_file_contains linux/src/main.rs "apply_app_key_approval_link"
require_file_contains linux/src/main.rs "LaunchInputDelivery::Queue"
require_file_contains linux/src/main.rs "launch_input_without_active_window_waits_for_ui"
require_file_contains linux/src/render.rs "Copy device ID"
require_file_absent linux/src/setup.rs 'field_title("Device"'
require_file_absent linux/src/ui.rs "Name (optional)"
require_file_absent linux/src/ui.rs "Copy invite link"
require_file_absent linux/src/ui.rs "Reset invite"
require_file_contains windows/MainWindow.xaml "Copy Request Link"
require_file_contains windows/MainWindow.xaml 'Click="AwaitingBack_Click"'
require_file_absent windows/MainWindow.xaml "Start over"
require_file_contains windows/MainWindow.xaml "AwaitingQrGrid"
require_file_contains windows/MainWindow.xaml "Restore from recovery phrase"
require_file_contains windows/MainWindow.xaml "Restore from secret key"
require_file_contains windows/IrisDriveService.cs '"start_join_request"'
require_file_contains windows/IrisDriveNativeCore.cs "QrMatrixForText"
require_file_contains windows/MainWindowDevices.cs "Request link or device ID"
require_file_contains windows/MainWindowDevices.cs "Approve this device?"
require_file_contains windows/MainWindow.xaml.cs '"app_key_approval"'
require_file_contains windows/MainWindow.xaml.cs 'await ConfirmAndApproveDeviceAsync(argument, NoticeText, static () => { });'
require_file_absent windows/MainWindow.xaml.cs 'await ApproveDeviceAsync(argument, "");'
require_file_contains windows/MainWindow.xaml.cs "CopyPeerDevice_Click"
require_file_absent windows/MainWindow.xaml "Device invite link"
require_file_absent windows/MainWindow.xaml 'Text="Device"'
require_file_absent windows/MainWindowDevices.cs "Name (optional)"
require_file_absent windows/MainWindow.xaml "Reset invite"
require_file_absent windows/MainWindowDevices.cs "ResetInvite_Click"
require_file_contains docs/PARITY.md "Linux GTK and Windows WPF GUI smokes"

python3 - "$ROOT/scripts/lib/cross-vm-device-link.sh" <<'PY_POLL'
import subprocess
import sys

prefix = r'''
source "$1"
POLL_SECS=3
print_statuses() { :; }
started_at="$(monotonic_milliseconds)"
'''
cases = [
    ("ready-before-deadline", r'''
ready_at=$((started_at + 300))
ready() { (( $(monotonic_milliseconds) >= ready_at )); }
wait_until_before ready-before-deadline "$((started_at + 1000))" ready "$started_at" 0.1
''', 0, "ok: ready-before-deadline", 2),
    ("false-check-crosses-deadline", r'''
slow_false() { sleep 0.2; return 1; }
wait_until_before false-check-crosses-deadline "$((started_at + 100))" slow_false "$started_at"
''', 1, "timed out after 100ms", 1),
    ("late-success", r'''
late_success() { sleep 0.2; }
wait_until_before late-success "$((started_at + 100))" late_success "$started_at" 0.1
''', 1, "late success after", 1),
]
for name, body, status, marker, timeout in cases:
    result = subprocess.run(["bash", "-c", prefix + body, "bash", sys.argv[1]],
                            capture_output=True, text=True, timeout=timeout)
    output = result.stdout + result.stderr
    if result.returncode != status or marker not in output:
        raise SystemExit(f"{name} failed: status={result.returncode} {output}")
    print(f"deadline regression passed: {name}")
PY_POLL

python3 - "$ROOT/scripts/lib/cross-vm-device-link.sh" <<'PY'
import copy
import json
from pathlib import Path
import shlex
import subprocess
import sys
import tempfile

with tempfile.TemporaryDirectory() as directory:
    root = Path(directory)
    labels = ["owner", "windows", "macos", "ios", "android"]
    authorized = {"profile": {"authorization_state": "authorized", "pending_device_approval_receipt_count": 0}}
    direct = lambda peer: {"app_key_npub": peer, "fips_online": True, "fips_direct_online": True}
    baseline = {label: copy.deepcopy(authorized) for label in labels}
    baseline["windows"]["peers"] = [direct("aux-key")]
    baseline["aux"] = {**copy.deepcopy(authorized), "network": {"fips": {"running": True, "fresh": True}},
                       "peers": [direct("windows-key")]}
    cases = [("ready", None, None), ("missing-windows", None, None), ("failed-windows-query", None, None)]
    for label, path, value in [
        ("owner", ("profile", "authorization_state"), "awaiting_approval"),
        ("android", ("profile", "pending_device_approval_receipt_count"), 1),
        ("aux", ("profile", "authorization_state"), "awaiting_approval"),
        ("aux", ("profile", "pending_device_approval_receipt_count"), 1),
        ("aux", ("network", "fips", "running"), False),
        ("aux", ("network", "fips", "fresh"), False),
        ("aux", ("peers", 0, "fips_online"), False),
        ("aux", ("peers", 0, "fips_direct_online"), False),
        ("aux", ("peers", 0, "app_key_npub"), "wrong"),
        ("windows", ("peers", 0, "fips_online"), False),
        ("windows", ("peers", 0, "fips_direct_online"), False),
        ("windows", ("peers", 0, "app_key_npub"), "wrong"),
    ]:
        cases.append((f"{label}-{path[-1]}", (label, path), value))
    for name, mutation, value in cases:
        snapshots = copy.deepcopy(baseline)
        if mutation:
            label, path = mutation
            item = snapshots[label]
            for key in path[:-1]:
                item = item[key]
            item[path[-1]] = value
        for label, snapshot in snapshots.items():
            (root / label).write_text(json.dumps(snapshot))
        (root / "calls").write_text("")
        selected = [label for label in labels if name != "missing-windows" or label != "windows"]
        script = f'''source {shlex.quote(sys.argv[1])}
cd {shlex.quote(directory)}
LABELS=({' '.join(selected)})
windows_label=windows
DESKTOP_GUI_AUX_NPUB=aux-key
host_value() {{ echo windows-key; }}
desktop_gui_aux_idrive() {{ cat aux; }}
idrive_cmd() {{ echo "$1" >>calls; cat "$1"; [[ "$1" != windows || {shlex.quote(name)} != failed-windows-query ]]; }}
desktop_gui_reverse_link_complete
'''
        result = subprocess.run(["bash", "-c", script], capture_output=True, text=True, timeout=5)
        expected = 0 if name == "ready" else 1
        assert result.returncode == expected, (name, result.returncode, result.stderr)
        calls = (root / "calls").read_text().splitlines()
        assert calls.count("windows") <= 1, (name, "duplicate Windows status query", calls)
        if name == "ready":
            assert calls == labels, ("missing readiness check", calls)
print(f"REVERSE_APPROVAL_STATUS_CASES_OK count={len(cases)}")
PY

echo "DESKTOP_GUI_E2E_KIT_OK"
