#!/usr/bin/env bash

set -Eeuo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"

require_contains() {
  local path="$1"
  local pattern="$2"
  if ! grep -F "$pattern" "$ROOT/$path" >/dev/null; then
    echo "missing '$pattern' in $path" >&2
    exit 1
  fi
}

require_file() {
  local path="$1"
  if [[ ! -f "$ROOT/$path" ]]; then
    echo "missing required sharing parity file: $path" >&2
    exit 1
  fi
}

require_file macos/Sources/IrisDriveControlPanel.swift
require_file ios/Sources/IrisDriveRootView.swift
require_file android/app/src/main/java/to/iris/drive/app/IrisDriveAndroidApp.kt
require_file android/app/src/main/java/to/iris/drive/app/IrisDriveMainContent.kt
require_file android/app/src/main/res/drawable/ic_shares.xml
require_file linux/src/ui.rs
require_file windows/MainWindow.xaml
require_file windows/MainWindowShares.cs

require_contains macos/Sources/IrisDriveControlPanel.swift "case shares"
require_contains macos/Sources/IrisDriveControlPanel.swift "SectionTitle(\"Shares\")"
require_contains macos/Sources/IrisDriveControlPanel.swift "Create Shared Folder"
require_contains macos/Sources/IrisDriveControlPanel.swift "Join Shared Folder"
require_contains macos/Sources/IrisDriveControlPanel.swift "InviteShareMemberSheet"

require_contains ios/Sources/IrisDriveRootView.swift "case shares"
require_contains ios/Sources/IrisDriveRootView.swift "Label(\"Shares\""
require_contains ios/Sources/IrisDriveRootView.swift "Section(\"Create Shared Folder\")"
require_contains ios/Sources/IrisDriveRootView.swift "Section(\"Accept Invite\")"
require_contains ios/Sources/IrisDriveRootView.swift "InviteShareMemberSheet"

require_contains android/app/src/main/java/to/iris/drive/app/IrisDriveAndroidApp.kt "Shares(\"Shares\", \"tabShares\", R.drawable.ic_shares)"
require_contains android/app/src/main/java/to/iris/drive/app/IrisDriveAndroidApp.kt "MainTab.values().forEach"
require_contains android/app/src/main/java/to/iris/drive/app/IrisDriveMainContent.kt "MainTab.Shares -> SharesContent"
require_contains android/app/src/main/java/to/iris/drive/app/IrisDriveMainContent.kt "CardSection(title = \"Shares\""
require_contains android/app/src/main/java/to/iris/drive/app/IrisDriveMainContent.kt "Create Shared Folder"
require_contains android/app/src/main/java/to/iris/drive/app/IrisDriveMainContent.kt "InviteShareMemberDialog"
require_contains android/app/src/androidTest/java/to/iris/drive/app/IrisDriveAndroidGuiFlowTest.kt "compose.onNodeWithTag(\"tabShares\").assertIsDisplayed()"

require_contains linux/src/ui.rs 'stack.add_titled(&shares_page, Some("shares"), "Shares")'
require_contains linux/src/ui.rs '("shares", "emblem-shared-symbolic", "Shares")'
require_contains linux/src/ui.rs "Create shared folder"
require_contains linux/src/ui.rs "Accept share invite"
require_contains linux/src/actions.rs "show_invite_share_member_dialog"

require_contains windows/MainWindow.xaml "NavSharesButton"
require_contains windows/MainWindow.xaml "Tag=\"Shares\""
require_contains windows/MainWindow.xaml "Create shared folder"
require_contains windows/MainWindow.xaml "Accept invite"
require_contains windows/MainWindowShares.cs "ShowInviteShareMember_Click"

echo "SHARING_MENU_PARITY_OK"
