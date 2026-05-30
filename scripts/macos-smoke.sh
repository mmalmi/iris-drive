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

truthy() {
  case "${1:-}" in
    1|true|TRUE|True|yes|YES|Yes|on|ON|On) return 0 ;;
    *) return 1 ;;
  esac
}

if truthy "${IRIS_DRIVE_MACOS_SMOKE_CREATE_PROFILE:-0}"; then
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
  truthy "${IRIS_DRIVE_MACOS_SMOKE_UI:-0}"
}

run_create_profile_smoke() {
  truthy "${IRIS_DRIVE_MACOS_SMOKE_CREATE_PROFILE:-0}"
}

run_create_profile_direct_smoke() {
  truthy "${IRIS_DRIVE_MACOS_SMOKE_CREATE_PROFILE_DIRECT:-0}"
}

run_create_profile_gui_smoke() {
  run_create_profile_smoke && ! run_create_profile_direct_smoke
}

CREATE_PROFILE_USERNAME="${IRIS_DRIVE_MACOS_SMOKE_CREATE_USERNAME:-}"
if run_create_profile_gui_smoke && [[ -z "$CREATE_PROFILE_USERNAME" ]]; then
  CREATE_PROFILE_USERNAME="Mac Smoke"
fi

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

let environment = ProcessInfo.processInfo.environment
var userInfo = [String: Any]()
if let username = environment["IRIS_DRIVE_MACOS_SMOKE_CREATE_USERNAME"],
   !username.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty {
    userInfo["username"] = username
}
if let profilePhotoPath = environment["IRIS_DRIVE_MACOS_SMOKE_CREATE_PROFILE_PHOTO"],
   !profilePhotoPath.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty {
    userInfo["profilePhotoPath"] = profilePhotoPath
}

DistributedNotificationCenter.default().postNotificationName(
    Notification.Name("to.iris.drive.e2eCreateProfile"),
    object: nil,
    userInfo: userInfo.isEmpty ? nil : userInfo,
    deliverImmediately: true
)
RunLoop.current.run(until: Date().addingTimeInterval(0.2))
SWIFT
}

request_show_control_panel() {
  /usr/bin/swift - >/dev/null <<'SWIFT'
import Foundation

DistributedNotificationCenter.default().postNotificationName(
    Notification.Name("to.iris.drive.showControlPanel"),
    object: nil,
    userInfo: nil,
    deliverImmediately: true
)
RunLoop.current.run(until: Date().addingTimeInterval(0.2))
SWIFT
}

drive_create_profile_gui() {
  local username="$1"

  /usr/bin/osascript - "$APP_PROCESS_NAME" "$username" >/dev/null <<'APPLESCRIPT'
on setupGroup(appName)
  tell application "System Events"
    tell process appName
      return group 1 of window 1
    end tell
  end tell
end setupGroup

on setupStaticTextExists(appName, expected)
  tell application "System Events"
    tell process appName
      try
        repeat with textItem in static texts of my setupGroup(appName)
          try
            if (value of textItem as text) is expected then return true
          end try
        end repeat
      end try
    end tell
  end tell
  return false
end setupStaticTextExists

on waitForSetupText(appName, expected, timeoutSeconds)
  set deadline to (current date) + timeoutSeconds
  repeat while (current date) is less than deadline
    if my setupStaticTextExists(appName, expected) then return
    delay 0.2
  end repeat
  error "Timed out waiting for setup text: " & expected
end waitForSetupText

on clickSetupButton(appName, buttonName, fallbackIndex)
  tell application "System Events"
    tell process appName
      set frontmost to true
      set controls to my setupGroup(appName)
      if exists button buttonName of controls then
        click button buttonName of controls
      else
        click button fallbackIndex of controls
      end if
    end tell
  end tell
end clickSetupButton

on fillUsername(appName, username)
  set previousClipboard to the clipboard
  set the clipboard to username
  tell application "System Events"
    tell process appName
      set frontmost to true
      try
        click text field 1 of my setupGroup(appName)
      end try
    end tell

    repeat with attempt from 1 to 10
      keystroke "a" using command down
      keystroke "v" using command down
      delay 0.2
      tell process appName
        try
          if (value of text field 1 of my setupGroup(appName) as text) is username then
            set the clipboard to previousClipboard
            return
          end if
        end try
      end tell
      key code 48
      delay 0.1
    end repeat
  end tell
  set the clipboard to previousClipboard
  error "Username field did not accept GUI input"
end fillUsername

on run argv
  set appName to item 1 of argv
  set username to item 2 of argv

  tell application "System Events"
    tell process appName
      set frontmost to true
    end tell
  end tell

  my waitForSetupText(appName, "Iris Drive", 10)
  my clickSetupButton(appName, "Create profile", 1)
  my waitForSetupText(appName, "Create profile", 5)

  if username is not "" then
    my fillUsername(appName, username)
  end if

  my clickSetupButton(appName, "Create profile", 2)
  if username is "" then return

  my waitForSetupText(appName, "Profile photo", 5)
  my clickSetupButton(appName, "Later", 3)
end run
APPLESCRIPT
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

APP_PATH="${IRIS_DRIVE_MACOS_SMOKE_APP_PATH:-}"
if [[ -z "$APP_PATH" ]]; then
  APP_PATH="$(IRIS_DRIVE_MACOS_SIGNING=none "$ROOT/scripts/macos-dev-app.sh" build)"
fi
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
  --stdout "$APP_STDOUT"
  --stderr "$APP_STDERR"
)
if ! run_create_profile_gui_smoke; then
  open_args=(-j "${open_args[@]}")
fi
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

  if run_create_profile_gui_smoke; then
    if ! request_show_control_panel || ! drive_create_profile_gui "$CREATE_PROFILE_USERNAME"; then
      echo "FAIL: could not complete the Create profile GUI journey." >&2
      show_recent_logs >&2
      exit 1
    fi
  else
    if ! request_create_profile; then
      echo "FAIL: could not request Create profile." >&2
      show_recent_logs >&2
      exit 1
    fi
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

  if [[ -n "$CREATE_PROFILE_USERNAME" ]] &&
    [[ "$status_json" != *"\"username\":\"$CREATE_PROFILE_USERNAME\""* ]]; then
    echo "FAIL: Create profile did not save the requested username." >&2
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
