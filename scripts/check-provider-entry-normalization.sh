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

require_contains "crates/iris-drive-core/src/provider.rs" "pub parent_path: String"
require_contains "crates/iris-drive-core/src/provider.rs" "pub display_name: String"
require_contains "crates/iris-drive-core/src/provider.rs" "pub fn normalize_provider_document_path"
require_contains "crates/iris-drive-core/src/provider.rs" "pub fn provider_path_is_child_document"
require_contains "crates/iris-drive-core/src/provider.rs" "change_key: format!"
require_contains "crates/iris-drive-app-core/src/native_provider.rs" "use iris_drive_core::provider::{"
require_contains "crates/iris-drive-app-core/src/native_provider.rs" "native_provider_normalize_path_json"
require_contains "crates/iris-drive-app-core/src/native_provider.rs" "native_provider_is_child_document_json"
require_contains "crates/iris-drive-app-core/src/c_abi.rs" "iris_drive_provider_normalize_path_json"
require_contains "crates/iris-drive-app-core/src/c_abi.rs" "iris_drive_provider_is_child_document_json"
require_contains "crates/iris-drive-app-core/src/c_abi.rs" "providerNormalizePathJson"
require_contains "crates/iris-drive-app-core/src/c_abi.rs" "providerIsChildDocumentJson"
require_contains "crates/iris-drive-cli/src/drive.rs" "use iris_drive_core::provider::{"
require_contains "crates/iris-drive-cli/src/commands.rs" 'name = "normalize-path"'
require_absent "crates/iris-drive-app-core/src/native_provider.rs" "struct ProviderListEntry"
require_absent "crates/iris-drive-cli/src/drive.rs" "struct ProviderListEntry"

require_contains "ios/FileProvider/FileProviderStorage.swift" 'case parentPath = "parent_path"'
require_contains "ios/FileProvider/FileProviderStorage.swift" 'case displayName = "display_name"'
require_contains "ios/FileProvider/FileProviderStorage.swift" "IrisDriveNativeProvider.normalizePath(path: relative)"
require_contains "ios/Sources/IrisDriveNativeCore.swift" "iris_drive_provider_normalize_path_json"
require_contains "macos/FileProvider/FileProviderItem.swift" 'case parentPath = "parent_path"'
require_contains "macos/FileProvider/FileProviderItem.swift" 'case displayName = "display_name"'
require_contains "macos/FileProvider/FileProviderItem.swift" 'provider", "normalize-path", path'
require_contains "android/app/src/main/java/to/iris/drive/app/provider/IrisDriveDocumentStore.kt" 'parentPath = entry.optString("parent_path")'
require_contains "android/app/src/main/java/to/iris/drive/app/provider/IrisDriveDocumentStore.kt" 'displayName = entry.optString("display_name")'
require_contains "android/app/src/main/java/to/iris/drive/app/core/NativeCore.kt" "external fun providerNormalizePathJson"
require_contains "android/app/src/main/java/to/iris/drive/app/core/NativeCore.kt" "external fun providerIsChildDocumentJson"
require_contains "android/app/src/main/java/to/iris/drive/app/provider/IrisDriveDocumentStore.kt" "NativeCore.normalizedProviderPath(path)"
require_contains "android/app/src/main/java/to/iris/drive/app/provider/IrisDriveDocumentStore.kt" "NativeCore.providerPathIsChildDocument(parentPath, childPath)"
require_contains "crates/iris-drive-app-core/src/state.rs" "pub provider_change_key: String"
require_contains "crates/iris-drive-app-core/src/state.rs" "pub provider_directory_paths: Vec<String>"
require_contains "ios/Sources/IrisDriveNativeCore.swift" 'case providerChangeKey = "provider_change_key"'
require_contains "ios/Sources/IrisDriveMobileModel.swift" "currentProviderSignalKey = state.ui.providerChangeKey"
require_contains "macos/Sources/IrisDriveMacApp.swift" "let key = summary.changeKey"

require_absent "ios/FileProvider/FileProviderStorage.swift" "parentPath(for:"
require_absent "ios/FileProvider/FileProviderStorage.swift" "fileName(for:"
require_absent "ios/FileProvider/FileProviderStorage.swift" "isSafeRelativePath"
require_absent "ios/Sources/IrisDriveMobileModel.swift" "var anchor: String?"
require_absent "ios/Sources/IrisDriveMobileModel.swift" "case anchor"
require_absent "ios/Sources/IrisDriveMobileModel.swift" "loadProviderSignalSummary"
require_absent "ios/Sources/IrisDriveMobileModel.swift" "ProviderState"
require_absent "ios/Sources/IrisDriveMobileModel.swift" "changeKey.isEmpty"
require_absent "macos/FileProvider/FileProviderItem.swift" "parentPath(for:"
require_absent "macos/FileProvider/FileProviderItem.swift" "fileName(for:"
require_absent "macos/FileProvider/FileProviderItem.swift" "isSafeRelativePath"
require_absent "macos/Sources/IrisDriveMacApp.swift" "let anchor: String?"
require_absent "macos/Sources/IrisDriveMacApp.swift" "case anchor"
require_absent "macos/Sources/IrisDriveMacApp.swift" "effectiveChangeKey"
require_absent "macos/Sources/IrisDriveMacApp.swift" "changeKey.isEmpty"
require_absent "android/app/src/main/java/to/iris/drive/app/provider/IrisDriveDocumentStore.kt" "parentOf("
require_absent "android/app/src/main/java/to/iris/drive/app/provider/IrisDriveDocumentStore.kt" "substringAfterLast('/')"
require_absent "android/app/src/main/java/to/iris/drive/app/provider/IrisDriveDocumentStore.kt" "path.split('/')"
require_absent "android/app/src/main/java/to/iris/drive/app/provider/IrisDriveDocumentStore.kt" "?: path"
require_absent "android/app/src/main/java/to/iris/drive/app/provider/IrisDriveDocumentStore.kt" 'startsWith("$parentPath/")'

echo "PROVIDER_ENTRY_NORMALIZATION_OK"
