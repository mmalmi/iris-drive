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
SMOKE_APP_DATA="$ROOT/macos/.build/SmokeAppData"
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

click_show_drive_folder() {
  osascript >/dev/null <<'APPLESCRIPT'
tell application "System Events"
  tell process "Iris Drive"
    click menu bar item 1 of menu bar 2
    delay 0.2
    click menu item "Open Drive Folder" of menu 1 of menu bar item 1 of menu bar 2
  end tell
end tell
APPLESCRIPT
}

APP_PATH="$(IRIS_DRIVE_MACOS_SIGNING=none "$ROOT/scripts/macos-dev-app.sh" build)"
if [[ -z "$APP_PATH" || ! -d "$APP_PATH" ]]; then
  echo "FAIL: macOS app was not built." >&2
  exit 1
fi

terminate_app_process
rm -rf "$SMOKE_APP_DATA"
mkdir -p "$SMOKE_APP_DATA/Config"
"$APP_PATH/Contents/MacOS/idrive" \
  --config-dir "$SMOKE_APP_DATA/Config" \
  init --force --label "macOS smoke" >/dev/null

open_args=(
  -j
  --env "IRIS_DRIVE_APP_BASE_DIR=$SMOKE_APP_DATA"
  --stdout "$APP_STDOUT"
  --stderr "$APP_STDERR"
)
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

if run_ui_smoke; then
  if ! click_show_drive_folder; then
    echo "FAIL: could not click the Show Drive Folder menu item." >&2
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
if run_ui_smoke; then
  echo "app launched, menu bar item installed, bundled daemon started, and Show Drive Folder opened the drive folder"
else
  echo "app launched hidden, menu bar item installed, control panel status loaded, and bundled daemon started"
fi
