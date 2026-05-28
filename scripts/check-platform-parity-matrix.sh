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
require_contains "iOS simulator smoke"
require_contains "Android adb smoke"
require_contains "just e2e-5devices"

echo "PLATFORM_PARITY_MATRIX_OK"
