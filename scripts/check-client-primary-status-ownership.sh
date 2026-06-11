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

require_contains android/app/src/main/java/to/iris/drive/app/IrisDriveMainContent.kt "val statusText = state.primaryStatusLabel"
require_absent android/app/src/main/java/to/iris/drive/app/IrisDriveMainContent.kt 'val statusText = if (state.sync.running) "Up to date" else "Paused"'
require_absent android/app/src/main/java/to/iris/drive/app/IrisDriveMainContent.kt 'state.sync.status.ifBlank { if (state.sync.running) "on" else "paused" }'

require_contains windows/MainWindow.xaml.cs 'var shellStatus = syncRunning ? status.PrimaryStatusLabel : "Daemon offline";'
require_contains windows/MainWindow.xaml.cs 'var pillStatus = syncRunning ? status.PrimaryStatusLabel : "Offline";'
require_contains windows/MainWindow.xaml.cs "SetupNotice.Text = notice ?? status.PrimaryStatusLabel;"
require_absent windows/MainWindow.xaml.cs 'DriveMessage.Text = syncRunning ? "Sync on" : "Paused";'
require_absent windows/MainWindow.xaml.cs 'StatusPill.Text = syncRunning ? "On" : "Paused";'
require_absent windows/MainWindow.xaml.cs 'status.FileCount > 0 ? status.FileCount : status.TopLevelEntries'
require_absent windows/IrisDriveModels.cs "public int RosterSize"
require_absent windows/IrisDriveModels.cs "public int PublishedAppKeyRoots"
require_absent windows/IrisDriveModels.cs "public int TopLevelEntries"
require_absent windows/IrisDriveModels.cs "public int LocalBlockCount"
require_absent windows/IrisDriveModels.cs "public long LocalBlockBytes"
require_absent windows/IrisDriveModels.cs "JsonSetupComplete"

require_contains linux/src/refresh.rs 'let shell_status_label = if sync_running {'
require_contains linux/src/refresh.rs '"Daemon offline"'
require_contains linux/src/refresh.rs 'let pill_status_label = if sync_running {'
require_contains linux/src/setup.rs '"Daemon offline"'

echo "CLIENT_PRIMARY_STATUS_OWNERSHIP_OK"
