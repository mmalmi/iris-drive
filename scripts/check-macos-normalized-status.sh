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

require_contains macos/Sources/IrisDriveStatus.swift 'json["roster_online_device_count"]'
require_contains macos/Sources/IrisDriveStatus.swift 'json["roster_direct_device_count"]'
require_contains macos/Sources/IrisDriveStatus.swift 'json["online_device_count"]'
require_contains macos/Sources/IrisDriveStatus.swift 'json["direct_device_count"]'
require_contains macos/Sources/IrisDriveStatus.swift 'json["mesh_device_count"]'
require_contains macos/Sources/IrisDriveStatus.swift "setupState == \"authorized\""
require_contains macos/Sources/IrisDriveMacApp.swift 'json["summary"]'
require_contains macos/Sources/IrisDriveMacApp.swift "applyStatusSummary"

require_absent macos/Sources/IrisDriveStatus.swift "roster_online_peer_count"
require_absent macos/Sources/IrisDriveStatus.swift "roster_connected_peer_count"
require_absent macos/Sources/IrisDriveStatus.swift "online_peer_count"
require_absent macos/Sources/IrisDriveStatus.swift "direct_peer_count"
require_absent macos/Sources/IrisDriveStatus.swift "connected_peer_count"
require_absent macos/Sources/IrisDriveStatus.swift "mesh_peer_count"
require_absent macos/Sources/IrisDriveStatus.swift 'authorizationState == "authorized"'
require_absent macos/Sources/IrisDriveStatus.swift 'authorizationState == "awaiting_approval"'
require_absent macos/Sources/IrisDriveStatus.swift 'authorizationState == "revoked"'
require_absent macos/Sources/IrisDriveMacApp.swift 'status.fileCount = Self.intValue(hashtree["file_count"])'
require_absent macos/Sources/IrisDriveMacApp.swift 'network["authorized_device_count"]'

echo "MACOS_NORMALIZED_STATUS_OK"
