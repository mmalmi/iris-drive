#!/usr/bin/env bash

set -Eeuo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
PARITY="$ROOT/docs/PARITY.md"

require_contains() {
  local pattern="$1"
  if ! grep -F "$pattern" "$PARITY" >/dev/null; then
    echo "missing '$pattern' in docs/PARITY.md" >&2
    exit 1
  fi
}

require_contains "| Capability | Linux GTK | macOS SwiftUI | Windows WPF | iOS SwiftUI | Android Compose |"
require_contains "| First-run create profile |"
require_contains "| Native OS file-provider surface |"
require_contains "| Link this app install flow |"
require_contains "| Add another app install |"
require_contains "| Open nhash file link |"
require_contains "Files app FileProvider domain + open action"
require_contains "App-group runtime/config/block paths"
require_contains "SAF DocumentsProvider + open action"
require_contains "DocumentsProvider read/write surface"
require_contains "iOS simulator smoke"
require_contains "GUI linking smokes"
require_contains "Android GUI"
require_contains "just e2e-5devices"

if grep -F "No; app shell only" "$PARITY" >/dev/null; then
  echo "docs/PARITY.md still contains app-shell-only mobile gaps" >&2
  exit 1
fi

echo "PLATFORM_PARITY_MATRIX_OK"
