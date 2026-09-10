#!/usr/bin/env bash
# Launch an isolated built app and sample its idle macOS process roles.
set -Eeuo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
APP_PATH="${IRIS_DRIVE_MACOS_IDLE_APP_PATH:-$ROOT/macos/.build/Applications/Iris Drive.app}"
APP_BASE_DIR=""

cleanup() {
  local status=$?
  if [[ -n "$APP_BASE_DIR" ]]; then
    if [[ -x "$APP_PATH/Contents/MacOS/idrive" ]]; then
      "$APP_PATH/Contents/MacOS/idrive" --config-dir "$APP_BASE_DIR/Config" \
        service uninstall --json >/dev/null 2>&1 || true
    fi
    pkill -f "$APP_PATH/Contents/MacOS/idrive.*daemon" >/dev/null 2>&1 || true
    osascript -e 'tell application "Iris Drive" to quit' >/dev/null 2>&1 || true
    rm -rf "$APP_BASE_DIR"
  fi
  return "$status"
}
trap cleanup EXIT

[[ "$(uname -s)" == Darwin ]] || {
  echo "macOS idle CPU smoke requires macOS" >&2
  exit 2
}
[[ -d "$APP_PATH" ]] || APP_PATH="$("$ROOT/scripts/macos-dev-app.sh" build)"
IDRIVE="$APP_PATH/Contents/MacOS/idrive"
[[ -x "$IDRIVE" ]] || {
  echo "macOS idle CPU smoke needs the idrive bundled in $APP_PATH" >&2
  exit 1
}

APP_BASE_DIR="$(mktemp -d "${TMPDIR:-/tmp}/iris-drive-macos-idle.XXXXXX")"
mkdir -p "$APP_BASE_DIR/Config"
"$IDRIVE" --config-dir "$APP_BASE_DIR/Config" init --force \
  --label "macOS idle CPU gate" >/dev/null

output="$(
  IRIS_DRIVE_MACOS_SIGNING="${IRIS_DRIVE_MACOS_SIGNING:-none}" \
    IRIS_DRIVE_APP_BASE_DIR="$APP_BASE_DIR" \
    "$ROOT/scripts/macos-dev-app.sh" run-existing "$APP_PATH"
)"
printf '%s\n' "$output"
launched_path="$(printf '%s\n' "$output" | sed -n 's/^macOS app launched: //p' | tail -n 1)"
[[ "$launched_path" == "$APP_PATH" && -d "$launched_path" ]] || {
  echo "macOS idle CPU smoke could not determine its launched app path" >&2
  exit 1
}

IRIS_DRIVE_IDLE_CPU_REQUIRED_ROLES="${IRIS_DRIVE_RELEASE_GATE_MACOS_IDLE_CPU_ROLES:-app,daemon}" \
  IRIS_DRIVE_IDLE_CPU_WARMUP_SECS="${IRIS_DRIVE_RELEASE_GATE_MACOS_IDLE_CPU_WARMUP_SECS:-60}" \
  IRIS_DRIVE_IDLE_CPU_COMMAND_MATCH="$APP_PATH" \
  "$ROOT/scripts/idle-cpu-gate.sh" --platform macos

echo "MACOS_IDLE_CPU_SMOKE_OK"
