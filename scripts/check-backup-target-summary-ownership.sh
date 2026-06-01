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

require_contains crates/iris-drive-cli/src/status/backups.rs '"title": backup_target_title(target, label)'
require_contains crates/iris-drive-cli/src/status/backups.rs '"state": backup_target_state(target)'
require_contains crates/iris-drive-cli/src/status/backups.rs '"detail": backup_target_detail(target)'

require_contains windows/IrisDriveModels.cs 'String(target, "title")'
require_contains windows/IrisDriveModels.cs 'String(target, "detail")'
require_contains windows/IrisDriveModels.cs 'String(target, "state")'
require_absent windows/IrisDriveModels.cs 'Object(target, "last_sync")'
require_absent windows/IrisDriveModels.cs 'Object(target, "last_check")'
require_absent windows/IrisDriveModels.cs '"download_bytes_per_second"'

require_contains linux/src/render.rs 'find_string(target, &["title"])'
require_contains linux/src/render.rs 'find_string(target, &["detail"])'
require_contains linux/src/render.rs 'find_string(target, &["state"])'
require_absent linux/src/render.rs 'target.get("last_sync")'
require_absent linux/src/render.rs 'target.get("last_check")'
require_absent linux/src/render.rs '"download_bytes_per_second"'

require_contains macos/Sources/IrisDriveStatus.swift 'json["title"] as? String'
require_contains macos/Sources/IrisDriveStatus.swift 'json["detail"] as? String'
require_absent macos/Sources/IrisDriveStatus.swift 'var title: String'
require_absent macos/Sources/IrisDriveStatus.swift 'var detail: String'

echo "BACKUP_TARGET_SUMMARY_OWNERSHIP_OK"
