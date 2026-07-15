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

require_contains windows/MainWindow.xaml.cs "private Forms.ToolStripMenuItem? syncTrayMenuItem;"
require_contains windows/MainWindow.xaml.cs "UpdateTraySyncMenuItem(syncRunning);"
require_contains windows/MainWindow.xaml.cs "syncTrayMenuItem = new Forms.ToolStripMenuItem"
require_absent windows/MainWindow.xaml.cs 'menu.Items.Add("Resume Sync"'
require_absent windows/MainWindow.xaml.cs 'menu.Items.Add("Pause Sync"'
require_absent windows/MainWindow.xaml.cs 'menu.Items.Add("Log out"'

require_contains linux/src/main.rs "tray_sync_running: Arc<AtomicBool>,"
require_contains linux/src/tray.rs "tray_sync_menu_item("
require_absent linux/src/tray.rs "TrayCommand::Logout"
require_absent linux/src/tray.rs '"Log Out"'

require_contains macos/Sources/IrisDriveMacApp.swift "private var syncMenuItem: NSMenuItem?"
require_contains macos/Sources/IrisDriveMacApp.swift "updateSyncMenuItem(running: running)"
require_absent macos/Sources/IrisDriveMacApp.swift "startSyncMenuItem"
require_absent macos/Sources/IrisDriveMacApp.swift "stopSyncMenuItem"
require_absent macos/Sources/IrisDriveMacApp.swift "Copy drive.iris.to Link"
require_absent macos/Sources/IrisDriveMacApp.swift "View on drive.iris.to"
require_absent macos/Sources/IrisDriveMacApp.swift "Show Config Folder"
require_absent macos/Sources/IrisDriveMacApp.swift 'title: "Log Out"'

require_contains windows/README.md "Windows tray icon with show/open/sync/quit actions"

echo "TRAY_MENU_STATE_OK"
