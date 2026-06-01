#!/usr/bin/env bash

set -Eeuo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"

require_contains() {
  local path="$1"
  local pattern="$2"
  if ! grep -F "$pattern" "$ROOT/$path" >/dev/null; then
    echo "missing '$pattern' in $path" >&2
    exit 1
  fi
}

require_absent() {
  local path="$1"
  local pattern="$2"
  if grep -F "$pattern" "$ROOT/$path" >/dev/null; then
    echo "unexpected '$pattern' in $path" >&2
    exit 1
  fi
}

require_contains crates/iris-drive-core/src/lib.rs "pub mod relay_config;"
require_contains crates/iris-drive-core/src/relay_config.rs "pub fn normalize_relay_url"
require_contains crates/iris-drive-core/src/relay_config.rs "pub fn dedupe_relay_urls"
require_contains crates/iris-drive-app-core/src/ffi.rs "iris_drive_core::relay_config::{dedupe_relay_urls, normalize_relay_url}"
require_contains crates/iris-drive-cli/src/drive/commands.rs "iris_drive_core::relay_config::{dedupe_relay_urls, normalize_relay_url}"
require_contains crates/iris-drive-cli/src/daemon.rs "iris_drive_core::relay_config::normalize_relay_url"
require_contains crates/iris-drive-cli/src/status.rs "iris_drive_core::relay_config::normalize_relay_url"
require_absent crates/iris-drive-cli/src/drive/commands.rs "fn normalize_relay_url"
require_absent crates/iris-drive-cli/src/drive/commands.rs "fn dedupe_relays"
require_absent ios/Sources/IrisDriveMobileModel.swift "if !relays.contains(candidate)"

echo "RELAY_CONFIG_NORMALIZATION_OK"
