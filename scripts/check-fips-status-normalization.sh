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

require_missing_file() {
  local file="$1"
  if [[ -e "$ROOT/$file" ]]; then
    echo "unexpected file $file" >&2
    exit 1
  fi
}

require_contains crates/iris-drive-core/src/lib.rs "pub mod fips_status;"
require_contains crates/iris-drive-core/src/fips_status.rs "pub fn normalize_fips_status_value"
require_contains crates/iris-drive-core/src/fips_status.rs "pub fn fips_online_devices_from_status"
require_contains crates/iris-drive-core/src/fips_status.rs "pub fn online_device_ids"

require_contains crates/iris-drive-cli/src/status/network.rs "normalize_fips_status_value("
require_absent crates/iris-drive-cli/src/status/network.rs "fn fips_state_label"
require_absent crates/iris-drive-cli/src/status/network.rs "fn fips_peer_connection_label"
require_absent crates/iris-drive-cli/src/status/network.rs "fn normalized_fips_peer_statuses"

require_contains crates/iris-drive-app-core/src/ffi.rs "normalize_fips_status_value("
require_contains crates/iris-drive-app-core/src/ffi.rs "online_device_ids("
require_absent crates/iris-drive-app-core/src/ffi.rs "struct NativeFipsStatus"
require_absent crates/iris-drive-app-core/src/ffi.rs "fn native_fips_state_label"
require_absent crates/iris-drive-app-core/src/lib.rs "mod native_fips;"
require_missing_file crates/iris-drive-app-core/src/native_fips.rs

echo "FIPS_STATUS_NORMALIZATION_OK"
