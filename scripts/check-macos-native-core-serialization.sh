#!/usr/bin/env bash

set -Eeuo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
APP="$ROOT/macos/Sources/IrisDriveMacApp.swift"
SOURCES="$ROOT/macos/Sources"

require_contains() {
  local needle="$1"
  if ! grep -Fqr "$needle" "$SOURCES"; then
    echo "missing '$needle' in macos/Sources" >&2
    exit 1
  fi
}

require_absent() {
  local needle="$1"
  if grep -Fq "$needle" "$APP"; then
    echo "unexpected '$needle' in macos/Sources/IrisDriveMacApp.swift" >&2
    exit 1
  fi
}

require_contains 'let nativeCoreQueue = DispatchQueue(label: "to.iris.drive.macos.native-core"'
require_contains "var nativeStatusRefreshInFlight = false"
require_contains "var nativeStatusRefreshPending = false"
require_contains "func scheduleNativeStatusRefresh()"
require_contains "func finishNativeStatusRefresh()"
require_contains "nativeCoreQueue.async { [weak self] in"
require_contains "self.scheduleNativeStatusRefresh()"
require_absent "let state = try nativeStatePayload(from: desktopCore.refreshJson())"

echo "MACOS_NATIVE_CORE_SERIALIZATION_OK"
