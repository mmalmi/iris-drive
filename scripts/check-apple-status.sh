#!/bin/bash
set -euo pipefail
root=$(cd "$(dirname "$0")/.." && pwd)
build=$(mktemp -d)
trap 'rm -rf "$build"' EXIT
xcrun swiftc -swift-version 5 "$root/ios/Sources/IrisDriveDeviceModel.swift" \
    "$root/macos/Sources/IrisDriveUpdateCheck.swift" "$root/scripts/apple-status-tests/main.swift" \
    -o "$build/status-tests"
"$build/status-tests"
