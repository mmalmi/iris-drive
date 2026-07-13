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
require_file_contains scripts/desktop-gui-smoke.sh "Windows GUI smoke requires a desktop session that exposes visible windows"
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
require_file_contains scripts/macos-smoke.sh "IRIS_DRIVE_DEBUG_LOG_DIR"
require_file_contains windows/App.xaml.cs "using var writer = new StreamWriter(client, new UTF8Encoding(false));"
require_file_contains windows/MainWindow.xaml.cs "if (launchArguments.Length == 0)"
require_file_contains windows/MainWindow.xaml.cs "ShowFromTray();"
require_file_contains macos/Sources/IrisDriveMacApp.swift "controlPanelWindow"
require_file_contains macos/Sources/IrisDriveMacApp.swift "irisDriveDebugLog(\"Iris Drive menu bar item installed\")"
require_file_contains macos/Sources/IrisDriveSetupViews.swift "Copy Request Link"
require_file_contains macos/Sources/IrisDriveSetupViews.swift "Start over"
require_file_contains macos/Sources/IrisDriveSetupViews.swift "openRecoveryPhrase"
require_file_contains macos/Sources/IrisDriveSetupViews.swift "openSecretKey"
require_file_contains macos/Sources/IrisDriveMacApp.swift '"type": "start_join_request"'
require_file_contains macos/Sources/IrisDriveMacApp.swift "forceRestart: true"
require_file_contains macos/Sources/IrisDriveControlPanel.swift "Request link or device ID"
require_file_contains macos/Sources/IrisDriveControlPanel.swift "scanApprovalRequestQr"
require_file_contains macos/Sources/IrisDriveControlPanel.swift "Approve this device?"
require_file_contains macos/Sources/IrisDriveMacApp.swift '"app_key_approval"'
require_file_absent macos/Sources/IrisDriveSetupViews.swift 'keyedValue("Device"'
require_file_absent macos/Sources/IrisDriveControlPanel.swift "Device invite link"
require_file_absent macos/Sources/IrisDriveControlPanel.swift "Copy invite link"
require_file_absent macos/Sources/IrisDriveControlPanel.swift "Reset invite"
require_file_contains linux/src/setup.rs "Copy Request Link"
require_file_contains linux/src/setup.rs "Start over"
require_file_contains linux/src/setup.rs "open_recovery_phrase_setup"
require_file_contains linux/src/setup.rs "open_secret_key_setup"
require_file_contains linux/src/setup.rs "start_join_request()"
require_file_contains linux/src/ui.rs "Request link or device ID"
require_file_contains linux/src/actions.rs "Approve this device?"
require_file_contains linux/src/main.rs "apply_app_key_approval_link"
require_file_contains linux/src/render.rs "Copy device ID"
require_file_absent linux/src/setup.rs 'field_title("Device"'
require_file_absent linux/src/ui.rs "Name (optional)"
require_file_absent linux/src/ui.rs "Copy invite link"
require_file_absent linux/src/ui.rs "Reset invite"
require_file_contains windows/MainWindow.xaml "Copy Request Link"
require_file_contains windows/MainWindow.xaml "Start over"
require_file_contains windows/MainWindow.xaml "AwaitingQrGrid"
require_file_contains windows/MainWindow.xaml "Restore from recovery phrase"
require_file_contains windows/MainWindow.xaml "Restore from secret key"
require_file_contains windows/IrisDriveService.cs '"start_join_request"'
require_file_contains windows/IrisDriveNativeCore.cs "QrMatrixForText"
require_file_contains windows/MainWindowDevices.cs "Request link or device ID"
require_file_contains windows/MainWindowDevices.cs "Approve this device?"
require_file_contains windows/MainWindow.xaml.cs '"app_key_approval"'
require_file_contains windows/MainWindow.xaml.cs "CopyPeerDevice_Click"
require_file_absent windows/MainWindow.xaml "Device invite link"
require_file_absent windows/MainWindow.xaml 'Text="Device"'
require_file_absent windows/MainWindowDevices.cs "Name (optional)"
require_file_absent windows/MainWindow.xaml "Reset invite"
require_file_absent windows/MainWindowDevices.cs "ResetInvite_Click"
require_file_contains docs/PARITY.md "Linux GTK and Windows WPF GUI smokes"

echo "DESKTOP_GUI_E2E_KIT_OK"
