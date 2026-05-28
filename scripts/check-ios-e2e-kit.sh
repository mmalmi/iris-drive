#!/usr/bin/env bash

set -Eeuo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"

require_file() {
  local path="$1"
  if [[ ! -f "$ROOT/$path" ]]; then
    echo "missing required iOS e2e kit file: $path" >&2
    exit 1
  fi
}

require_contains() {
  local path="$1"
  local pattern="$2"
  if ! grep -F "$pattern" "$ROOT/$path" >/dev/null; then
    echo "missing '$pattern' in $path" >&2
    exit 1
  fi
}

require_file ios/project.yml
require_file ios/Sources/IrisDriveIOSApp.swift
require_file ios/Sources/IrisDriveMobileModel.swift
require_file ios/Sources/IrisDriveNativeCore.swift
require_file ios/FileProvider/FileProviderExtension.swift
require_file scripts/ios-simulator-smoke.sh
require_file scripts/cross-vm-four-platform-e2e.sh

require_contains ios/project.yml "IrisDriveIOS"
require_contains ios/project.yml "IrisDriveFileProvider"
require_contains ios/Info.plist "CFBundleURLSchemes"
require_contains ios/Info.plist "iris-drive"
require_contains ios/Sources/IrisDriveIOSApp.swift "ensureFileProviderDomain"
require_contains ios/Sources/IrisDriveMobileModel.swift "NSFileProviderManager.add"
require_contains ios/Sources/IrisDriveMobileModel.swift "copyLinkRequest"
require_contains ios/Sources/IrisDriveMobileModel.swift "openDriveFolder"
require_contains ios/Sources/IrisDriveMobileModel.swift "addRelay"
require_contains ios/Sources/IrisDriveMobileModel.swift "device-link"
require_contains ios/Sources/IrisDriveNativeCore.swift "iris_drive_app_dispatch_json"
require_contains ios/Sources/IrisDriveRootView.swift "Copy link request"
require_contains scripts/ios-simulator-smoke.sh "xcrun simctl"
require_contains scripts/ios-simulator-smoke.sh "SIMCTL_CHILD_IRIS_DRIVE_DEBUG_ACTION"
require_contains scripts/cross-vm-four-platform-e2e.sh "IRIS_DRIVE_E2E_IOS_HOST"
require_contains Justfile "ios-build"
require_contains Justfile "ios-smoke"
require_contains Justfile "e2e-4devices"

echo "IOS_E2E_KIT_OK"
