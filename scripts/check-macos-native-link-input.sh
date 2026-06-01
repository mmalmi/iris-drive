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

require_contains crates/iris-drive-app-core/src/lib.rs "classify_link_input"
require_contains crates/iris-drive-cli/src/commands.rs "LinkInput"
require_contains macos/Sources/IrisDriveMacApp.swift '["link-input", "classify", trimmed]'
require_contains macos/Sources/DeviceLinkInput.swift "inputIsComplete"
require_contains macos/Sources/IrisDriveControlPanel.swift "setupOwnerLinkInputIsComplete"
require_contains macos/Sources/IrisDriveControlPanel.swift "approveDeviceKeyIsComplete"
require_contains macos/Sources/IrisDriveControlPanel.swift ".disabled(!setupOwnerLinkInputIsComplete)"
require_contains macos/Sources/IrisDriveControlPanel.swift ".disabled(!approveDeviceKeyIsComplete)"
require_absent macos/Sources/IrisDriveControlPanel.swift ".disabled(setupOwner.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty)"
require_absent macos/Sources/IrisDriveControlPanel.swift ".disabled(approveDeviceKey.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty)"

echo "MACOS_NATIVE_LINK_INPUT_OK"
