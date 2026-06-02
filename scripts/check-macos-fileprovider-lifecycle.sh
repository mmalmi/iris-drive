#!/usr/bin/env bash

set -Eeuo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
APP="$ROOT/macos/Sources/IrisDriveMacApp.swift"
LIFECYCLE="$ROOT/macos/Sources/IrisDriveMacFileProvider.swift"

require_contains() {
  local file="$1"
  local label="$2"
  local needle="$3"
  if ! grep -Fq "$needle" "$file"; then
    echo "missing '$needle' in $label" >&2
    exit 1
  fi
}

require_contains "$APP" "macos/Sources/IrisDriveMacApp.swift" "ensureFileProviderDomainAfterStatusIfNeeded"
require_contains "$APP" "macos/Sources/IrisDriveMacApp.swift" "removeFileProviderDomainRegistration("
require_contains "$LIFECYCLE" "macos/Sources/IrisDriveMacFileProvider.swift" "irisDriveFileProviderRegistrationIdentityKey"
require_contains "$LIFECYCLE" "macos/Sources/IrisDriveMacFileProvider.swift" "markFileProviderRegistrationCurrent"
require_contains "$LIFECYCLE" "macos/Sources/IrisDriveMacFileProvider.swift" "fileProviderRegistrationIdentityIsCurrent"
require_contains "$LIFECYCLE" "macos/Sources/IrisDriveMacFileProvider.swift" "shouldRepairFileProviderRegistration"
require_contains "$LIFECYCLE" "macos/Sources/IrisDriveMacFileProvider.swift" "repairFileProviderRegistration"
require_contains "$LIFECYCLE" "macos/Sources/IrisDriveMacFileProvider.swift" "Iris Drive repairing stale FileProvider domain registration"
require_contains "$LIFECYCLE" "macos/Sources/IrisDriveMacFileProvider.swift" "Iris Drive FileProvider domain removed without re-add"

echo "MACOS_FILEPROVIDER_LIFECYCLE_OK"
