#!/usr/bin/env bash
# Update install steps are worded by the Rust core; the desktop shells render
# them and, after a failure, offer Try Again instead of re-enabling Install.

set -Eeuo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
source "$ROOT/scripts/lib/source-assertions.sh"

require_contains crates/iris-drive-core/src/updater/progress.rs "pub fn report_update_stage"
require_contains crates/iris-drive-core/src/updater.rs "download_reporter(directory, &selection.tag)"
require_contains crates/iris-drive-app-core/src/c_abi/update_progress.rs "iris_drive_update_progress_json"
require_contains crates/iris-drive-app-core/src/c_abi/update_progress.rs "iris_drive_update_report_stage_json"

require_contains macos/Sources/IrisDriveDesktopCore.swift '@_silgen_name("iris_drive_update_progress_json")'
require_contains macos/Sources/IrisDriveUpdater.swift 'reportUpdateStage("installing")'
require_contains macos/Sources/IrisDriveUpdater.swift 'reportUpdateStage("failed", reason: reason)'
require_contains macos/Sources/IrisDriveControlPanel.swift "IrisDriveUpdateProgressRow(progress: status.updateProgress)"
require_absent macos/Sources/IrisDriveUpdater.swift 'updateStatus = "Downloading'
require_absent macos/Sources/IrisDriveUpdater.swift 'updateStatus = "Installing'

require_contains windows/IrisDriveNativeCore.cs "iris_drive_update_report_stage_json"
require_contains windows/MainWindowUpdates.cs 'ReportUpdateStage("failed", error.Message)'
require_contains windows/MainWindowUpdates.cs 'UpdateBannerText.Text = shown ? updateProgress.Text'
require_absent windows/MainWindowUpdates.cs 'updateStatus = $"Downloading'
require_absent windows/MainWindowUpdates.cs 'updateStatus = $"Downloaded'

require_contains linux/src/updater.rs "report_update_stage(&config_dir, UpdateStage::Failed, reason)"
require_contains linux/src/main.rs "update_progress(&app_config_dir())"
require_absent linux/src/main.rs 'format!("Downloading {}"'
require_absent linux/src/main.rs '"Downloaded {}"'

echo "CLIENT_UPDATE_PROGRESS_OWNERSHIP_OK"
