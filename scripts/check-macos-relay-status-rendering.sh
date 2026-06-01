#!/usr/bin/env bash

set -Eeuo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
PANEL="$ROOT/macos/Sources/IrisDriveControlPanel.swift"
MAC_APP="$ROOT/macos/Sources/IrisDriveMacApp.swift"
ANDROID_STATE="$ROOT/android/app/src/main/java/to/iris/drive/app/core/AppState.kt"
ANDROID_PANEL="$ROOT/android/app/src/main/java/to/iris/drive/app/IrisDriveMainContent.kt"

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
require_contains "relay.statusLabel"
require_contains "relay.health"
require_absent "status.relays.map"
require_absent 'IrisDriveRelayStatus(url: relay, status: "configured")'
require_absent "reduce(into: [String: IrisDriveRelayStatus]())"
require_absent "relayStatusLabel("
require_absent 'status == "configured" ? "saved" : status'

require_macos_app_contains() {
  local needle="$1"
  if ! grep -Fq "$needle" "$MAC_APP"; then
    echo "missing '$needle' in macos/Sources/IrisDriveMacApp.swift" >&2
    exit 1
  fi
}

require_macos_app_absent() {
  local needle="$1"
  if grep -Fq "$needle" "$MAC_APP"; then
    echo "unexpected '$needle' in macos/Sources/IrisDriveMacApp.swift" >&2
    exit 1
  fi
}

require_macos_app_contains "self.refreshStatus()"
require_macos_app_absent "applyRelaysData"
require_macos_app_absent 'JSONSerialization.jsonObject(with: data) as? [String]'

require_android_contains() {
  local file="$1"
  local needle="$2"
  if ! grep -Fq "$needle" "$file"; then
    echo "missing '$needle' in ${file#$ROOT/}" >&2
    exit 1
  fi
}

require_android_absent() {
  local file="$1"
  local needle="$2"
  if grep -Fq "$needle" "$file"; then
    echo "unexpected '$needle' in ${file#$ROOT/}" >&2
    exit 1
  fi
}

require_android_contains "$ANDROID_STATE" "val relayStatuses: List<RelayStatus>"
require_android_contains "$ANDROID_STATE" 'ui.optJSONArray("relay_statuses").toRelayStatuses()'
require_android_contains "$ANDROID_STATE" "statusLabel = item.optString(\"status_label\")"
require_android_contains "$ANDROID_STATE" "health = item.optString(\"health\")"
require_android_contains "$ANDROID_PANEL" "state.relayStatuses.forEach"
require_android_contains "$ANDROID_PANEL" "relay.statusLabel"
require_android_contains "$ANDROID_PANEL" "relay.health"
require_android_absent "$ANDROID_PANEL" "state.relays.forEach"

echo "MACOS_RELAY_STATUS_RENDERING_OK"
