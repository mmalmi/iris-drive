#!/usr/bin/env bash

set -Eeuo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"

require_file() {
  local path="$1"
  if [[ ! -f "$ROOT/$path" ]]; then
    echo "missing required Android e2e kit file: $path" >&2
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

require_file android/settings.gradle.kts
require_file android/app/build.gradle.kts
require_file android/app/src/main/AndroidManifest.xml
require_file android/app/src/main/java/to/iris/drive/app/MainActivity.kt
require_file android/app/src/main/java/to/iris/drive/app/IrisDriveAndroidApp.kt
require_file android/app/src/main/java/to/iris/drive/app/provider/IrisDriveDocumentsProvider.kt
require_file scripts/mobile-android-smoke.sh
require_file scripts/cross-vm-five-platform-e2e.sh
require_file tools/run-android

require_contains android/app/build.gradle.kts "iris-drive-app-core"
require_contains android/app/src/main/AndroidManifest.xml "android.content.action.DOCUMENTS_PROVIDER"
require_contains android/app/src/main/AndroidManifest.xml "iris-drive"
require_contains android/app/src/main/java/to/iris/drive/app/core/AppState.kt "create_profile"
require_contains android/app/src/main/java/to/iris/drive/app/core/AppState.kt "approve_device"
require_contains android/app/src/main/java/to/iris/drive/app/IrisDriveAndroidApp.kt "Approve Device"
require_contains android/app/src/main/java/to/iris/drive/app/IrisDriveAndroidApp.kt "Copy owner key"
require_contains android/app/src/main/java/to/iris/drive/app/IrisDriveAndroidApp.kt "Open snapshot link"
require_contains android/app/src/main/java/to/iris/drive/app/IrisDriveAndroidApp.kt "Reset relay"
require_contains scripts/mobile-android-smoke.sh "PROVIDER_AUTHORITY"
require_contains scripts/mobile-android-smoke.sh "create-profile"
require_contains scripts/cross-vm-five-platform-e2e.sh "IRIS_DRIVE_E2E_ANDROID_HOST"
require_contains Justfile "android-build"
require_contains Justfile "android-smoke"
require_contains Justfile "e2e-5devices"

echo "ANDROID_E2E_KIT_OK"
