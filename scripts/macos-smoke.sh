#!/usr/bin/env bash

set -Eeuo pipefail

case "$(uname -s)" in
  Darwin) ;;
  *)
    echo "macOS smoke is Darwin-only; skipping on $(uname -s)"
    exit 0
    ;;
esac

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
APP_PROCESS_NAME="Iris Drive"
APP_BUNDLE_ID="to.iris.drive.macos"
SMOKE_DIR="$(mktemp -d -t iris-drive-macos-smoke)"
SMOKE_HOME="$SMOKE_DIR/home"
if [[ "${IRIS_DRIVE_MACOS_SMOKE_CREATE_PROFILE:-0}" =~ ^(1|true|TRUE|True|yes|YES|Yes|on|ON|On)$ ]]; then
  SMOKE_APP_DATA="$SMOKE_HOME/Library/Application Support/Iris Drive"
else
  SMOKE_APP_DATA="$ROOT/macos/.build/SmokeAppData"
fi
SMOKE_CONFIG_DIR="$SMOKE_APP_DATA/Config"
START_TIME="$(date '+%Y-%m-%d %H:%M:%S')"
APP_PATH=""
APP_STDOUT="$SMOKE_DIR/app.stdout.log"
APP_STDERR="$SMOKE_DIR/app.stderr.log"

run_ui_smoke() {
  case "${IRIS_DRIVE_MACOS_SMOKE_UI:-0}" in
    1|true|TRUE|True|yes|YES|Yes|on|ON|On) return 0 ;;
    *) return 1 ;;
  esac
}

run_create_profile_smoke() {
  case "${IRIS_DRIVE_MACOS_SMOKE_CREATE_PROFILE:-0}" in
    1|true|TRUE|True|yes|YES|Yes|on|ON|On) return 0 ;;
    *) return 1 ;;
  esac
}

terminate_app_process() {
  pkill -TERM -x "$APP_PROCESS_NAME" >/dev/null 2>&1 || true
  for _ in {1..40}; do
    if ! pgrep -x "$APP_PROCESS_NAME" >/dev/null 2>&1; then
      return 0
    fi
    sleep 0.1
  done
  pkill -x "$APP_PROCESS_NAME" >/dev/null 2>&1 || true
}

cleanup() {
  if [[ -n "$APP_PATH" ]]; then
    terminate_app_process
    pkill -f "$APP_PATH/Contents/MacOS/idrive daemon" >/dev/null 2>&1 || true
  fi
  if run_ui_smoke; then
    osascript "$SMOKE_DIR" >/dev/null 2>&1 <<'APPLESCRIPT' || true
on run argv
  set smokeRoot to item 1 of argv
  tell application "Finder"
    repeat with finderWindow in windows
      try
        set targetPath to POSIX path of (target of finderWindow as alias)
        if targetPath starts with smokeRoot then close finderWindow
      end try
    end repeat
  end tell
end run
APPLESCRIPT
  fi
  rm -rf "$SMOKE_APP_DATA"
  rm -rf "$SMOKE_DIR"
}
trap cleanup EXIT

show_recent_logs() {
  local end_time
  end_time="$(date '+%Y-%m-%d %H:%M:%S')"
  if [[ -s "$APP_STDOUT" || -s "$APP_STDERR" ]]; then
    echo "Captured app stdout:" >&2
    cat "$APP_STDOUT" >&2 2>/dev/null || true
    echo "Captured app stderr:" >&2
    cat "$APP_STDERR" >&2 2>/dev/null || true
  fi
  /usr/bin/log show \
    --start "$START_TIME" \
    --end "$end_time" \
    --style compact \
    --predicate "(eventMessage CONTAINS[c] \"Iris Drive\") OR (eventMessage CONTAINS[c] \"idrive\") OR (eventMessage CONTAINS[c] \"$APP_BUNDLE_ID\") OR (eventMessage CONTAINS[c] \"Launch failed\") OR (eventMessage CONTAINS[c] \"spawn failed\")" \
    2>/dev/null || true
}

wait_for_process() {
  local name="$1"
  local seconds="$2"

  for _ in $(seq 1 "$((seconds * 10))"); do
    if pgrep -x "$name" >/dev/null 2>&1; then
      return 0
    fi
    sleep 0.1
  done
  return 1
}

wait_for_daemon() {
  local seconds="$1"

  for _ in $(seq 1 "$((seconds * 10))"); do
    if pgrep -f "$APP_PATH/Contents/MacOS/idrive.*daemon" >/dev/null 2>&1; then
      return 0
    fi
    sleep 0.1
  done
  return 1
}

wait_for_log() {
  local pattern="$1"
  local seconds="$2"

  for _ in $(seq 1 "$((seconds * 10))"); do
    if grep -F "$pattern" "$APP_STDOUT" "$APP_STDERR" >/dev/null 2>&1; then
      return 0
    fi
    sleep 0.1
  done
  return 1
}

wait_for_file() {
  local path="$1"
  local seconds="$2"

  for _ in $(seq 1 "$((seconds * 10))"); do
    if [[ -f "$path" ]]; then
      return 0
    fi
    sleep 0.1
  done
  return 1
}

request_create_profile() {
  /usr/bin/swift - >/dev/null <<'SWIFT'
import Foundation

DistributedNotificationCenter.default().postNotificationName(
    Notification.Name("to.iris.drive.e2eCreateProfile"),
    object: nil,
    userInfo: nil,
    deliverImmediately: true
)
RunLoop.current.run(until: Date().addingTimeInterval(0.2))
SWIFT
}

request_show_drive_folder() {
  /usr/bin/swift - >/dev/null <<'SWIFT'
import Foundation

DistributedNotificationCenter.default().postNotificationName(
    Notification.Name("to.iris.drive.showDriveFolder"),
    object: nil,
    userInfo: nil,
    deliverImmediately: true
)
RunLoop.current.run(until: Date().addingTimeInterval(0.2))
SWIFT
}

APP_PATH="$(IRIS_DRIVE_MACOS_SIGNING=none "$ROOT/scripts/macos-dev-app.sh" build)"
if [[ -z "$APP_PATH" || ! -d "$APP_PATH" ]]; then
  echo "FAIL: macOS app was not built." >&2
  exit 1
fi

terminate_app_process
rm -rf "$SMOKE_APP_DATA"
if run_create_profile_smoke; then
  mkdir -p "$SMOKE_HOME"
else
  mkdir -p "$SMOKE_CONFIG_DIR"
  "$APP_PATH/Contents/MacOS/idrive" \
    --config-dir "$SMOKE_CONFIG_DIR" \
    init --force --label "macOS smoke" >/dev/null
fi

open_args=(
  -j
  --stdout "$APP_STDOUT"
  --stderr "$APP_STDERR"
)
if run_create_profile_smoke; then
  open_args+=(
    --env "CFFIXED_USER_HOME=$SMOKE_HOME"
    --env "HOME=$SMOKE_HOME"
    --env "IRIS_DRIVE_ENABLE_E2E_NOTIFICATIONS=1"
  )
else
  open_args+=(--env "IRIS_DRIVE_APP_BASE_DIR=$SMOKE_APP_DATA")
fi
open "${open_args[@]}" "$APP_PATH"

if ! wait_for_process "$APP_PROCESS_NAME" 10; then
  echo "FAIL: Iris Drive did not launch." >&2
  show_recent_logs >&2
  exit 1
fi

if ! wait_for_log "Iris Drive menu bar item installed" 10; then
  echo "FAIL: Iris Drive menu bar item was not installed." >&2
  show_recent_logs >&2
  exit 1
fi

if run_create_profile_smoke; then
  if ! wait_for_log "Iris Drive local profile not found" 10; then
    echo "FAIL: Iris Drive did not enter first-run setup." >&2
    show_recent_logs >&2
    exit 1
  fi

  if ! request_create_profile; then
    echo "FAIL: could not request Create profile." >&2
    show_recent_logs >&2
    exit 1
  fi

  if ! wait_for_file "$SMOKE_CONFIG_DIR/key" 20; then
    echo "FAIL: Create profile did not initialize a local profile." >&2
    show_recent_logs >&2
    exit 1
  fi

  status_json="$("$APP_PATH/Contents/MacOS/idrive" --config-dir "$SMOKE_CONFIG_DIR" status)"
  if [[ "$status_json" != *'"initialized":true'* ]]; then
    echo "FAIL: Create profile initialized key material but status is not initialized." >&2
    echo "$status_json" >&2
    show_recent_logs >&2
    exit 1
  fi

  if ! wait_for_daemon 10; then
    echo "FAIL: bundled idrive daemon did not start after Create profile." >&2
    show_recent_logs >&2
    exit 1
  fi
else
  if ! wait_for_log "Iris Drive control panel updated" 10; then
    echo "FAIL: Iris Drive did not load control panel status." >&2
    show_recent_logs >&2
    exit 1
  fi

  if ! wait_for_daemon 10; then
    echo "FAIL: bundled idrive daemon did not start." >&2
    show_recent_logs >&2
    exit 1
  fi
fi

if run_ui_smoke; then
  if ! request_show_drive_folder; then
    echo "FAIL: could not request Show Drive Folder." >&2
    show_recent_logs >&2
    exit 1
  fi

  if ! wait_for_log "Iris Drive mounted drive folder opened" 10 &&
    ! wait_for_log "Iris Drive mounted drive folder revealed" 1; then
    echo "FAIL: Show Drive Folder did not open the drive folder." >&2
    show_recent_logs >&2
    exit 1
  fi
fi

echo "MACOS_SMOKE_OK"
if run_create_profile_smoke; then
  echo "app launched into first-run setup, Create profile initialized a local profile, and bundled daemon started"
elif run_ui_smoke; then
  echo "app launched, menu bar item installed, bundled daemon started, and Show Drive Folder opened the drive folder"
else
  echo "app launched hidden, menu bar item installed, control panel status loaded, and bundled daemon started"
fi
