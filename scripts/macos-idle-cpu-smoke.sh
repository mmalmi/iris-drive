#!/usr/bin/env bash
# Launch an isolated built app and sample its idle macOS process roles.
set -Eeuo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
source "$ROOT/scripts/lib/macos-smoke-profile.sh"
APP_PATH="${IRIS_DRIVE_MACOS_IDLE_APP_PATH:-$ROOT/macos/.build/Applications/Iris Drive.app}"
APP_BASE_DIR=""
ARTIFACT_DIR="${IRIS_DRIVE_MACOS_IDLE_ARTIFACT_DIR:-}"
LOG_DIR=""

cleanup() {
  local status=$?
  if [[ -n "$APP_BASE_DIR" ]]; then
    if [[ -n "$ARTIFACT_DIR" ]]; then
      mkdir -p "$ARTIFACT_DIR"
      cp "$LOG_DIR"/app.*.log "$ARTIFACT_DIR/" 2>/dev/null || true
    fi
    if [[ -x "$APP_PATH/Contents/MacOS/idrive" ]]; then
      "$APP_PATH/Contents/MacOS/idrive" --config-dir "$APP_BASE_DIR/Config" \
        service uninstall --json >/dev/null 2>&1 || true
    fi
    pkill -f "$APP_PATH/Contents/MacOS/idrive.*daemon" >/dev/null 2>&1 || true
    osascript -e 'tell application "Iris Drive" to quit' >/dev/null 2>&1 || true
    rm -rf "$APP_BASE_DIR"
  fi
  [[ -z "$LOG_DIR" ]] || rm -rf "$LOG_DIR"
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

PROFILE_PARENT="$(macos_smoke_profile_parent "$APP_PATH")"
mkdir -p "$PROFILE_PARENT"
APP_BASE_DIR="$(mktemp -d "$PROFILE_PARENT/iris-drive-macos-idle.XXXXXX")"
# LaunchServices opens these files before the app starts. Its launcher cannot
# open paths protected by the signed app group's Data Vault.
LOG_DIR="$(mktemp -d "${TMPDIR:-/tmp}/iris-drive-macos-idle-logs.XXXXXX")"
mkdir -p "$APP_BASE_DIR/Config"
"$IDRIVE" --config-dir "$APP_BASE_DIR/Config" init --force \
  --label "macOS idle CPU gate" >/dev/null

open -n --stdout "$LOG_DIR/app.stdout.log" --stderr "$LOG_DIR/app.stderr.log" \
  --env "IRIS_DRIVE_APP_BASE_DIR=$APP_BASE_DIR" \
  --env "IRIS_DRIVE_DISABLE_DAEMON_SERVICE=true" \
  --env "IRIS_DRIVE_DISABLE_LOGIN_AGENT_SYNC=true" \
  --env "IRIS_DRIVE_DISABLE_SINGLE_INSTANCE=true" "$APP_PATH"
echo "macOS app data: $APP_BASE_DIR"
echo "macOS app launched: $APP_PATH"

IRIS_DRIVE_IDLE_CPU_REQUIRED_ROLES="${IRIS_DRIVE_RELEASE_GATE_MACOS_IDLE_CPU_ROLES:-app,daemon}" \
  IRIS_DRIVE_IDLE_CPU_WARMUP_SECS="${IRIS_DRIVE_RELEASE_GATE_MACOS_IDLE_CPU_WARMUP_SECS:-60}" \
  IRIS_DRIVE_IDLE_CPU_COMMAND_MATCH="$APP_PATH" \
  "$ROOT/scripts/idle-cpu-gate.sh" --platform macos

echo "MACOS_IDLE_CPU_SMOKE_OK"
