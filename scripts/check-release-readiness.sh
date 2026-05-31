#!/usr/bin/env bash

set -Eeuo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"

require_file() {
  local path="$1"
  if [[ ! -f "$ROOT/$path" ]]; then
    echo "missing release readiness file: $path" >&2
    exit 1
  fi
}

require_executable() {
  local path="$1"
  require_file "$path"
  if [[ ! -x "$ROOT/$path" ]]; then
    echo "release readiness file is not executable: $path" >&2
    exit 1
  fi
}

require_contains() {
  local path="$1"
  local needle="$2"
  if ! grep -Fq -- "$needle" "$ROOT/$path"; then
    echo "missing '$needle' in $path" >&2
    exit 1
  fi
}

require_executable scripts/release-gate.sh
require_executable scripts/ios-build
require_file .env.release.example
require_file .env.zapstore.example
require_file zapstore.yaml

require_contains Justfile "release-gate *args:"
require_contains Justfile "node scripts/local-release.mjs --build"
require_contains Justfile "release-publish:"
require_contains Justfile "release-final:"
require_contains scripts/local-release.mjs "--build"
require_contains scripts/local-release.mjs "--skip-zapstore"
require_contains scripts/local-release.mjs "publishZapstore"
require_contains scripts/local-release.mjs "scripts', 'ios-build'"
require_contains scripts/local-release.mjs ".env.zapstore.local"
require_contains scripts/local-release.mjs "requireCompleteAppRelease"
require_contains scripts/ios-build "ios-testflight-public"
require_contains scripts/ios-build "testFlightInternalTestingOnly"
require_contains scripts/ios-build "iTMSTransporter"
require_contains scripts/local-release-lib.mjs "validateReleaseAssetSet"
require_contains scripts/local-release-lib.mjs "plannedReleaseAssetNames"
require_contains android/app/build.gradle.kts "ANDROID_KEYSTORE_PATH"
require_contains scripts/release-gate.sh "just structure"
require_contains scripts/release-gate.sh "cargo test --workspace"
require_contains scripts/release-gate.sh "just e2e-5devices"
require_contains zapstore.yaml "release_source: dist/zapstore-current-android-arm64.apk"
require_contains .env.release.example "IRIS_DRIVE_RELEASE_TREE=releases/iris-drive"
require_contains .env.release.example "IRIS_DRIVE_WINDOWS_INSTALLER_PATH="
require_contains .env.release.example "IRIS_DRIVE_IOS_PUBLIC_TESTFLIGHT=1"
require_contains .env.zapstore.example "SIGN_WITH="
require_contains .gitignore ".env.zapstore.local"

echo "RELEASE_READINESS_OK"
