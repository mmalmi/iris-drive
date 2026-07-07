#!/usr/bin/env bash

set -Eeuo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
SMOKE="$ROOT/scripts/macos-smoke.sh"

require_contains() {
  local needle="$1"
  if ! grep -F "$needle" "$SMOKE" >/dev/null; then
    echo "FAIL: macOS smoke must contain: $needle" >&2
    exit 1
  fi
}

require_absent() {
  local needle="$1"
  if grep -F "$needle" "$SMOKE" >/dev/null; then
    echo "FAIL: macOS smoke must not use broad process cleanup: $needle" >&2
    exit 1
  fi
}

require_contains "app_process_pids()"
require_contains "process_command_matches()"
require_contains 'path_fragment="$APP_PATH/Contents/MacOS/$APP_PROCESS_NAME"'
require_contains "assert_app_running()"
require_contains "assert_daemon_running()"
require_contains "IRIS_DRIVE_MACOS_SMOKE_SURVIVAL_SECONDS"
require_contains "uninstall_smoke_daemon_service()"
require_contains '"$IDRIVE_CLI" --config-dir "$SMOKE_CONFIG_DIR" service uninstall --json'
require_absent 'pkill -TERM -x "$APP_PROCESS_NAME"'
require_absent 'pkill -x "$APP_PROCESS_NAME"'

echo "MACOS_SMOKE_PROCESS_SCOPE_OK"
