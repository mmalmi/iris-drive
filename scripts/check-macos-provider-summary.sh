#!/usr/bin/env bash

set -Eeuo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
APP="$ROOT/macos/Sources/IrisDriveMacApp.swift"

require_contains() {
  local needle="$1"
  if ! grep -Fq "$needle" "$APP"; then
    echo "missing '$needle' in macos/Sources/IrisDriveMacApp.swift" >&2
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

require_contains 'case changeKey = "change_key"'
require_contains "providerSignalSummary"
require_absent "fileProviderSignalKey("
require_absent "externalFileProviderSignalKey("
require_absent 'lastBlockSync["fetched"]'
require_absent 'drive["device_root_count"]'
require_absent 'peer["fips_online"]'

echo "MACOS_PROVIDER_SUMMARY_OK"
