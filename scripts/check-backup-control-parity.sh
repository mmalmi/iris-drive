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

require_contains crates/iris-drive-app-core/src/actions.rs "AddBackupTarget"
require_contains crates/iris-drive-app-core/src/actions.rs "RemoveBackupTarget"
require_contains crates/iris-drive-app-core/src/actions.rs "AddBlossomServer"
require_contains crates/iris-drive-app-core/src/actions.rs "RemoveBlossomServer"
require_contains crates/iris-drive-app-core/src/actions.rs "SyncBackups"
require_contains crates/iris-drive-app-core/src/actions.rs "CheckBackups"
require_contains crates/iris-drive-app-core/src/state.rs "pub id: String"
require_contains crates/iris-drive-app-core/src/state.rs "pub kind: String"
require_contains crates/iris-drive-app-core/src/state.rs "pub target: String"
require_contains crates/iris-drive-app-core/src/state.rs "pub configured_label: String"
require_contains crates/iris-drive-app-core/src/state.rs "pub enabled: bool"

require_contains ios/Sources/IrisDriveMobileBackups.swift "func addBackupTarget"
require_contains ios/Sources/IrisDriveMobileBackups.swift "func removeBackupTarget"
require_contains ios/Sources/IrisDriveMobileModel.swift "func addBlossomServer"
require_contains ios/Sources/IrisDriveMobileModel.swift "func removeBlossomServer"
require_contains ios/Sources/IrisDriveMobileBackups.swift "func syncBackups"
require_contains ios/Sources/IrisDriveMobileBackups.swift "func checkBackups"
require_contains ios/Sources/IrisDriveBackupViews.swift "Add Backup"
require_contains ios/Sources/IrisDriveBackupViews.swift "Sync Now"
require_contains ios/Sources/IrisDriveBackupViews.swift "Check All"
require_contains ios/Sources/IrisDriveBackupViews.swift "Destination URL, User ID, or folder path"
require_contains ios/Sources/IrisDriveBackupViews.swift "Remove backup"
require_absent ios/Sources/IrisDriveBackupViews.swift "Add Custom Target"
require_absent ios/Sources/IrisDriveBackupViews.swift "Add File Server"
require_absent ios/Sources/IrisDriveBackupViews.swift "Remove file server"
require_absent ios/Sources/IrisDriveBackupViews.swift "Add Blossom"
require_absent ios/Sources/IrisDriveBackupViews.swift "Remove Blossom"

require_contains android/app/src/main/java/to/iris/drive/app/core/AppState.kt "fun addBackupTarget"
require_contains android/app/src/main/java/to/iris/drive/app/core/AppState.kt "fun removeBackupTarget"
require_contains android/app/src/main/java/to/iris/drive/app/core/AppState.kt "fun addBlossomServer"
require_contains android/app/src/main/java/to/iris/drive/app/core/AppState.kt "fun removeBlossomServer"
require_contains android/app/src/main/java/to/iris/drive/app/core/AppState.kt "fun syncBackups"
require_contains android/app/src/main/java/to/iris/drive/app/core/AppState.kt "fun checkBackups"
require_contains android/app/src/main/java/to/iris/drive/app/IrisDriveAndroidApp.kt 'Backups("Backup"'
require_contains android/app/src/main/java/to/iris/drive/app/IrisDriveMainContent.kt "Add Backup"
require_contains android/app/src/main/java/to/iris/drive/app/IrisDriveMainContent.kt "Sync Now"
require_contains android/app/src/main/java/to/iris/drive/app/IrisDriveMainContent.kt "Check All"
require_contains android/app/src/main/java/to/iris/drive/app/IrisDriveMainContent.kt "Destination URL, User ID, or folder path"
require_contains android/app/src/main/java/to/iris/drive/app/IrisDriveMainContent.kt "Remove backup"
require_absent android/app/src/main/java/to/iris/drive/app/IrisDriveMainContent.kt "Add Custom Target"
require_absent android/app/src/main/java/to/iris/drive/app/IrisDriveMainContent.kt "File Servers"
require_absent android/app/src/main/java/to/iris/drive/app/IrisDriveMainContent.kt "Add File Server"
require_absent android/app/src/main/java/to/iris/drive/app/IrisDriveMainContent.kt "Remove target"
require_absent android/app/src/main/java/to/iris/drive/app/IrisDriveMainContent.kt "Remove file server"
require_absent android/app/src/main/java/to/iris/drive/app/IrisDriveMainContent.kt "Add Blossom"
require_absent android/app/src/main/java/to/iris/drive/app/IrisDriveMainContent.kt "Remove Blossom"

require_contains linux/src/actions.rs "NativeAppAction::AddBackupTarget"
require_contains linux/src/actions.rs "NativeAppAction::RemoveBackupTarget"
require_contains linux/src/actions.rs "NativeAppAction::SyncBackups"
require_contains linux/src/actions/backup_check.rs "NativeAppAction::CheckBackups"
require_absent linux/src/actions.rs 'run_idrive(["backups", "sync"])'
require_absent linux/src/actions.rs 'run_idrive(["backups", "check"])'
require_absent linux/src/actions/backup_check.rs 'run_idrive(["backups", "check"])'
require_contains linux/src/ui.rs "Add Backup"
require_contains linux/src/ui.rs "Destination URL, User ID, or folder path"
require_contains linux/src/render.rs "Remove backup"
require_contains linux/src/render.rs "No backups configured"
require_absent linux/src/ui.rs "Add custom target"
require_absent linux/src/ui.rs "Add File Server"
require_absent linux/src/render.rs "Remove target"
require_absent linux/src/ui.rs 'endpoint_group("Blossom"'
require_absent linux/src/render.rs "No Blossom servers"

require_contains macos/Sources/IrisDriveBackupActions.swift "addBackupTarget"
require_contains macos/Sources/IrisDriveBackupActions.swift "removeBackupTarget"
require_contains macos/Sources/IrisDriveBackupActions.swift "syncBackups"
require_contains macos/Sources/IrisDriveBackupActions.swift "checkBackups"
require_absent macos/Sources/IrisDriveBackupActions.swift 'arguments: ["backups", "sync"]'
require_absent macos/Sources/IrisDriveBackupActions.swift 'arguments: ["backups", "check"]'
require_contains macos/Sources/IrisDriveControlPanel.swift "Add Backup"
require_contains macos/Sources/IrisDriveControlPanel.swift "Choose Folder"
require_contains macos/Sources/IrisDriveControlPanel.swift "https://backup.example"
require_absent macos/Sources/IrisDriveControlPanel.swift "Destination URL, User ID, or folder path"
require_absent macos/Sources/IrisDriveControlPanel.swift "backupTargetLabelInput"
require_contains macos/Sources/BackupTargetRow.swift "Remove backup"
require_absent macos/Sources/IrisDriveControlPanel.swift "Add Custom Target"
require_absent macos/Sources/IrisDriveControlPanel.swift "Add File Server"
require_absent macos/Sources/BackupTargetRow.swift "Remove file server"
require_absent macos/Sources/BackupTargetRow.swift "Remove target"
require_absent macos/Sources/IrisDriveControlPanel.swift 'title: "Blossom"'

require_contains windows/IrisDriveService.cs "AddBackupTarget"
require_contains windows/IrisDriveService.cs "RemoveBackupTarget"
require_contains windows/IrisDriveService.cs "SyncBackups"
require_contains windows/IrisDriveService.cs "CheckBackups"
require_absent windows/IrisDriveService.cs 'RunAsync("backups", "sync")'
require_absent windows/IrisDriveService.cs 'RunAsync("backups", "check")'
require_contains windows/MainWindow.xaml "Add backup"
require_contains windows/MainWindow.xaml "Destination URL, User ID, or folder path"
require_contains windows/MainWindowBackups.cs "Remove backup"
require_contains windows/MainWindowBackups.cs "No backups configured"
require_absent windows/MainWindow.xaml "Add custom target"
require_absent windows/MainWindowBackups.cs "Remove file server"
require_absent windows/MainWindowBackups.cs "Remove target"
require_absent windows/MainWindow.xaml "Text=\"Blossom\""
require_absent windows/MainWindowBackups.cs "No Blossom servers"

echo "BACKUP_CONTROL_PARITY_OK"
