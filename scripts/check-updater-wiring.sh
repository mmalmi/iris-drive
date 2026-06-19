#!/usr/bin/env bash

set -Eeuo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"

require_contains() {
  local file="$1"
  local needle="$2"
  if ! grep -Fq -- "$needle" "$ROOT/$file"; then
    echo "missing '$needle' in $file" >&2
    exit 1
  fi
}

require_contains crates/iris-drive-core/src/updater.rs "fn preferred_app_asset_uses_current_platform_artifacts_only()"
require_contains crates/iris-drive-core/src/updater.rs "fn compares_semver_like_update_tags()"
require_contains crates/iris-drive-cli/src/updater.rs "fn product_update_config_includes_running_embedded_hashtree_url()"
require_contains crates/iris-drive-cli/src/daemon/tests_part1.rs "fn daemon_status_records_binary_version_for_gui_mismatch_detection()"
require_contains crates/iris-drive-cli/src/service.rs "fn macos_binary_version_query_reads_version_json()"

require_contains crates/iris-drive-cli/src/service.rs '"binary_path": service_binary'
require_contains crates/iris-drive-cli/src/service.rs '"binary_version": binary_version'
require_contains crates/iris-drive-cli/src/service.rs 'macos_service_executable_path_from_plist_contents(&plist)'

require_contains macos/Sources/IrisDriveStatus.swift "var daemonVersionMismatch: Bool"
require_contains macos/Sources/IrisDriveStatus.swift "var serviceVersionMismatch: Bool"
require_contains macos/Sources/IrisDriveStatus.swift "var runtimeVersionMismatch: Bool"
require_contains macos/Sources/IrisDriveStatus.swift "static func versionsDiffer"
require_contains macos/Sources/IrisDriveControlPanel.swift "if status.runtimeVersionMismatch"
require_contains macos/Sources/IrisDriveControlPanel.swift 'Button(status.serviceVersionMismatch ? "Update Service" : "Restart Sync")'
require_contains macos/Sources/IrisDriveControlPanel.swift "controller.updateDaemonService()"

require_contains macos/Sources/IrisDriveUpdater.swift "installingAppUpdate = true"
require_contains macos/Sources/IrisDriveUpdater.swift "NSApp.terminate(nil)"
require_contains macos/Sources/IrisDriveUpdater.swift 'while kill -0 "$old_pid"'
require_contains macos/Sources/IrisDriveUpdater.swift 'service start --json'
require_contains macos/Sources/IrisDriveMacApp.swift "if installingAppUpdate"
require_contains macos/Sources/IrisDriveMacApp.swift 'updateStatus("Installing update")'
require_contains macos/Sources/IrisDriveMacApp.swift "stopSync()"
echo "UPDATER_WIRING_OK"
