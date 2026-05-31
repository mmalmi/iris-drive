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

require_contains "crates/iris-drive-app-core/src/native_provider.rs" "parent_path: String"
require_contains "crates/iris-drive-app-core/src/native_provider.rs" "display_name: String"
require_contains "crates/iris-drive-cli/src/drive.rs" "parent_path: String"
require_contains "crates/iris-drive-cli/src/drive.rs" "display_name: String"

require_contains "ios/FileProvider/FileProviderStorage.swift" 'case parentPath = "parent_path"'
require_contains "ios/FileProvider/FileProviderStorage.swift" 'case displayName = "display_name"'
require_contains "macos/FileProvider/FileProviderItem.swift" 'case parentPath = "parent_path"'
require_contains "macos/FileProvider/FileProviderItem.swift" 'case displayName = "display_name"'
require_contains "android/app/src/main/java/to/iris/drive/app/provider/IrisDriveDocumentStore.kt" 'parentPath = entry.optString("parent_path")'
require_contains "android/app/src/main/java/to/iris/drive/app/provider/IrisDriveDocumentStore.kt" 'displayName = entry.optString("display_name")'

require_absent "ios/FileProvider/FileProviderStorage.swift" "parentPath(for:"
require_absent "ios/FileProvider/FileProviderStorage.swift" "fileName(for:"
require_absent "macos/FileProvider/FileProviderItem.swift" "parentPath(for:"
require_absent "macos/FileProvider/FileProviderItem.swift" "fileName(for:"
require_absent "android/app/src/main/java/to/iris/drive/app/provider/IrisDriveDocumentStore.kt" "parentOf("
require_absent "android/app/src/main/java/to/iris/drive/app/provider/IrisDriveDocumentStore.kt" "substringAfterLast('/')"

echo "PROVIDER_ENTRY_NORMALIZATION_OK"
