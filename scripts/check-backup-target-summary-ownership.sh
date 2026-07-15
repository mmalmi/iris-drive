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

require_contains crates/iris-drive-core/src/lib.rs "pub mod backup_summary;"
require_contains crates/iris-drive-core/src/backup_summary.rs "pub fn backup_target_summary"
require_contains crates/iris-drive-core/src/backup_summary.rs "pub fn backup_target_kind_label"
require_contains crates/iris-drive-core/src/backup_summary.rs "pub fn blossom_backup_target"
require_contains crates/iris-drive-cli/src/status/backups.rs "backup_target_summary(target)"
require_contains crates/iris-drive-app-core/src/ffi.rs "backup_ui_rows_for_config"
require_contains crates/iris-drive-app-core/src/ffi.rs "backup_target_summary(target)"
require_contains crates/iris-drive-app-core/src/ffi.rs "blossom_backup_target(server)"
require_absent crates/iris-drive-cli/src/status/backups.rs "fn backup_target_title"
require_absent crates/iris-drive-cli/src/status/backups.rs "fn backup_target_state"
require_absent crates/iris-drive-cli/src/status/backups.rs "fn backup_target_detail"
require_absent crates/iris-drive-cli/src/status/backups.rs "fn backup_target_format_bytes"

require_contains windows/IrisDriveModels.cs 'String(backup, "label")'
require_contains windows/IrisDriveModels.cs 'String(backup, "detail")'
require_contains windows/IrisDriveModels.cs 'String(backup, "state")'
require_absent windows/IrisDriveModels.cs 'String(target, "title")'
require_absent windows/IrisDriveModels.cs 'Object(target, "last_sync")'
require_absent windows/IrisDriveModels.cs 'Object(target, "last_check")'
require_absent windows/IrisDriveModels.cs '"download_bytes_per_second"'

require_contains linux/src/render.rs 'target.label'
require_contains linux/src/render.rs 'target.detail'
require_contains linux/src/render.rs 'target.state'
require_absent linux/src/render.rs 'target.get("last_sync")'
require_absent linux/src/render.rs 'target.get("last_check")'
require_absent linux/src/render.rs '"download_bytes_per_second"'

require_contains macos/Sources/IrisDriveStatus.swift 'json["title"] as? String'
require_contains macos/Sources/IrisDriveStatus.swift 'json["detail"] as? String'
require_absent macos/Sources/IrisDriveStatus.swift 'var title: String'
require_absent macos/Sources/IrisDriveStatus.swift 'var detail: String'

echo "BACKUP_TARGET_SUMMARY_OWNERSHIP_OK"
