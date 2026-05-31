#!/usr/bin/env bash

set -Eeuo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
PANEL="$ROOT/macos/Sources/IrisDriveControlPanel.swift"

require_contains() {
  local needle="$1"
  if ! grep -Fq "$needle" "$PANEL"; then
    echo "missing '$needle' in macos/Sources/IrisDriveControlPanel.swift" >&2
    exit 1
  fi
}

require_absent() {
  local needle="$1"
  if grep -Fq "$needle" "$PANEL"; then
    echo "unexpected '$needle' in macos/Sources/IrisDriveControlPanel.swift" >&2
    exit 1
  fi
}

require_contains "private var relayRows: [IrisDriveRelayStatus]"
require_contains "status.relayStatuses"
require_absent "status.relays.map"
require_absent 'IrisDriveRelayStatus(url: relay, status: "configured")'
require_absent "reduce(into: [String: IrisDriveRelayStatus]())"

echo "MACOS_RELAY_STATUS_RENDERING_OK"
