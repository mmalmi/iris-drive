#!/bin/bash
set -euo pipefail
ROOT=$(cd "$(dirname "$0")/.." && pwd)
if [[ $(uname -s) != Darwin ]]; then
  echo "macOS profile isolation requires the macOS SDK"
  exit 1
fi
BUILD=$(mktemp -d "${TMPDIR:-/tmp}/iris-drive-macos-profile-tests.XXXXXX")
trap 'rm -rf "$BUILD"' EXIT
xcrun swiftc -swift-version 5 \
  "$ROOT/macos/Shared/IrisDriveRuntimeSupport.swift" \
  "$ROOT/macos/Shared/IrisDriveFileProviderProfile.swift" \
  "$ROOT/macos/FileProvider/FileProviderItem.swift" \
  "$ROOT/macos/FileProvider/FileProviderEnumerator.swift" \
  "$ROOT/macos/FileProvider/FileProviderExtension.swift" \
  "$ROOT/scripts/macos-profile-tests/main.swift" \
  -o "$BUILD/profile-tests"
"$BUILD/profile-tests"
