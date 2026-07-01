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
export CARGO_TARGET_DIR="${CARGO_TARGET_DIR:-$HOME/.cache/cargo-target}"
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

RUN_CREATE_PROFILE_SMOKE=0
if truthy "${IRIS_DRIVE_MACOS_SMOKE_CREATE_PROFILE:-0}"; then
  RUN_CREATE_PROFILE_SMOKE=1
fi

RUN_USER_JOURNEY_SMOKE=0
if truthy "${IRIS_DRIVE_MACOS_SMOKE_USER_JOURNEY:-0}" ||
  truthy "${IRIS_DRIVE_MACOS_SMOKE_LINK_JOURNEY:-0}"; then
  RUN_USER_JOURNEY_SMOKE=1
fi

if [[ "$RUN_CREATE_PROFILE_SMOKE" == "1" && "$RUN_USER_JOURNEY_SMOKE" == "1" ]]; then
  echo "FAIL: create-profile and user-journey smoke modes are mutually exclusive." >&2
  exit 1
fi

if [[ "$RUN_CREATE_PROFILE_SMOKE" == "1" || "$RUN_USER_JOURNEY_SMOKE" == "1" ]]; then
  SMOKE_APP_DATA="$SMOKE_HOME/Library/Application Support/Iris Drive"
else
  SMOKE_APP_DATA="$ROOT/macos/.build/SmokeAppData"
fi
SMOKE_CONFIG_DIR="$SMOKE_APP_DATA/Config"
START_TIME="$(date '+%Y-%m-%d %H:%M:%S')"
APP_PATH=""
IDRIVE_CLI=""
APP_STDOUT="$SMOKE_DIR/app.stdout.log"
APP_STDERR="$SMOKE_DIR/app.stderr.log"
APP_DEBUG_LOG_DIR="$SMOKE_DIR/logs"
APP_DEBUG_LOG="$APP_DEBUG_LOG_DIR/macos-app-debug.log"
USER_JOURNEY_OPENED_DRIVE_FOLDER=0
OWNER_DAEMON_PID=""

run_ui_smoke() {
  truthy "${IRIS_DRIVE_MACOS_SMOKE_UI:-0}"
}

run_create_profile_smoke() {
  [[ "$RUN_CREATE_PROFILE_SMOKE" == "1" ]]
}

run_create_profile_direct_smoke() {
  truthy "${IRIS_DRIVE_MACOS_SMOKE_CREATE_PROFILE_DIRECT:-0}"
}

run_create_profile_gui_smoke() {
  run_create_profile_smoke && ! run_create_profile_direct_smoke
}

run_user_journey_smoke() {
  [[ "$RUN_USER_JOURNEY_SMOKE" == "1" ]]
}

require_drive_folder_open() {
  truthy "${IRIS_DRIVE_MACOS_SMOKE_REQUIRE_DRIVE_FOLDER:-0}"
}

CREATE_PROFILE_USERNAME="${IRIS_DRIVE_MACOS_SMOKE_CREATE_USERNAME:-}"
if run_create_profile_gui_smoke && [[ -z "$CREATE_PROFILE_USERNAME" ]]; then
  CREATE_PROFILE_USERNAME="Mac Smoke"
fi

json_get() {
  local path="$1"
  python3 -c '
import json
import sys

value = json.load(sys.stdin)
for part in sys.argv[1].split("."):
    if isinstance(value, dict) and part in value:
        value = value[part]
    else:
        sys.exit(1)
if isinstance(value, bool):
    print(str(value).lower())
elif value is not None:
    print(value)
' "$path"
}

json_array_len() {
  local path="$1"
  python3 -c '
import json
import sys

value = json.load(sys.stdin)
for part in sys.argv[1].split("."):
    if isinstance(value, dict) and part in value:
        value = value[part]
    else:
        sys.exit(1)
if not isinstance(value, list):
    sys.exit(1)
print(len(value))
' "$path"
}

json_list_has_path() {
  local expected="$1"
  python3 -c '
import json
import sys

listing = json.load(sys.stdin)
expected = sys.argv[1]
paths = {entry.get("path") for entry in listing.get("files", [])}
if expected not in paths:
    sys.exit(1)
' "$expected"
}

json_report_is_synced() {
  local expected_kind="$1"
  python3 -c '
import json
import sys

data = json.load(sys.stdin)
reports = data.get("reports", [])
if len(reports) != 1:
    sys.exit(1)
report = reports[0]
if report.get("kind") != sys.argv[1] or report.get("state") != "synced":
    sys.exit(1)
upload = report.get("upload", {})
if int(upload.get("total_hashes") or 0) <= 0:
    sys.exit(1)
' "$expected_kind"
}

json_report_check_ran() {
  local expected_kind="$1"
  python3 -c '
import json
import sys

data = json.load(sys.stdin)
reports = data.get("reports", [])
if len(reports) != 1:
    sys.exit(1)
report = reports[0]
if report.get("kind") != sys.argv[1]:
    sys.exit(1)
if not report.get("root_cid"):
    sys.exit(1)
if report.get("state") not in {"verified", "pending"}:
    sys.exit(1)
' "$expected_kind"
}

terminate_app_process() {
  local pid

  for pid in $(app_process_pids); do
    kill -TERM "$pid" >/dev/null 2>&1 || true
  done
  for _ in {1..40}; do
    if [[ -z "$(app_process_pids)" ]]; then
      return 0
    fi
    sleep 0.1
  done
  for pid in $(app_process_pids); do
    kill "$pid" >/dev/null 2>&1 || true
  done
}

remove_smoke_path_best_effort() {
  local path="$1"
  [[ -e "$path" ]] || return 0
  for _ in {1..5}; do
    rm -rf "$path" >/dev/null 2>&1 || true
    [[ ! -e "$path" ]] && return 0
    sleep 0.2
  done
  echo "warning: smoke cleanup left $path" >&2
}

cleanup() {
  if [[ -n "$OWNER_DAEMON_PID" ]]; then
    kill "$OWNER_DAEMON_PID" >/dev/null 2>&1 || true
    wait "$OWNER_DAEMON_PID" >/dev/null 2>&1 || true
  fi
  if [[ -n "$APP_PATH" ]]; then
    terminate_app_process
    pkill -f "$APP_PATH/Contents/MacOS/idrive daemon" >/dev/null 2>&1 || true
  fi
  if run_ui_smoke || run_user_journey_smoke; then
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
  remove_smoke_path_best_effort "$SMOKE_APP_DATA"
  remove_smoke_path_best_effort "$SMOKE_DIR"
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
  if [[ -s "$APP_DEBUG_LOG" ]]; then
    echo "Captured app debug log:" >&2
    cat "$APP_DEBUG_LOG" >&2 2>/dev/null || true
  fi
  if [[ -s "$SMOKE_DIR/owner-daemon.stdout.log" || -s "$SMOKE_DIR/owner-daemon.stderr.log" ]]; then
    echo "Captured owner daemon stdout:" >&2
    cat "$SMOKE_DIR/owner-daemon.stdout.log" >&2 2>/dev/null || true
    echo "Captured owner daemon stderr:" >&2
    cat "$SMOKE_DIR/owner-daemon.stderr.log" >&2 2>/dev/null || true
  fi
  /usr/bin/log show \
    --start "$START_TIME" \
    --end "$end_time" \
    --style compact \
    --predicate "(eventMessage CONTAINS[c] \"Iris Drive\") OR (eventMessage CONTAINS[c] \"idrive\") OR (eventMessage CONTAINS[c] \"$APP_BUNDLE_ID\") OR (eventMessage CONTAINS[c] \"Launch failed\") OR (eventMessage CONTAINS[c] \"spawn failed\")" \
    2>/dev/null || true
}

process_command_matches() {
  local pid="$1"
  local path_fragment="$2"
  local command

  command="$(ps -p "$pid" -o command= 2>/dev/null || true)"
  [[ "$command" == *"$path_fragment"* ]]
}

app_process_pids() {
  local pid
  local path_fragment="$APP_PATH/Contents/MacOS/$APP_PROCESS_NAME"

  [[ -n "${APP_PATH:-}" ]] || return 0
  pgrep -x "$APP_PROCESS_NAME" 2>/dev/null | while IFS= read -r pid; do
    if process_command_matches "$pid" "$path_fragment"; then
      printf '%s\n' "$pid"
    fi
  done
}

app_is_running() {
  [[ -n "$(app_process_pids)" ]]
}

wait_for_app_process() {
  local seconds="$1"

  for _ in $(seq 1 "$((seconds * 10))"); do
    if app_is_running; then
      return 0
    fi
    sleep 0.1
  done
  return 1
}

assert_app_running() {
  local context="$1"

  if ! app_is_running; then
    echo "FAIL: Iris Drive exited $context." >&2
    show_recent_logs >&2
    exit 1
  fi
}

assert_daemon_running() {
  local context="$1"

  if ! pgrep -f "$APP_PATH/Contents/MacOS/idrive.*daemon" >/dev/null 2>&1; then
    echo "FAIL: bundled idrive daemon exited $context." >&2
    show_recent_logs >&2
    exit 1
  fi
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
  local log_file

  for _ in $(seq 1 "$((seconds * 10))"); do
    for log_file in "$APP_STDOUT" "$APP_STDERR" "$APP_DEBUG_LOG"; do
      if [[ -f "$log_file" ]] &&
        grep -F "$pattern" "$log_file" >/dev/null 2>&1; then
        return 0
      fi
    done
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

mkdir_p_or_fail() {
  local path="$1"
  local seconds="${2:-10}"

  python3 - "$path" "$seconds" <<'PY'
import subprocess
import sys

path = sys.argv[1]
seconds = float(sys.argv[2])
try:
    subprocess.run(["mkdir", "-p", path], check=True, timeout=seconds)
except subprocess.TimeoutExpired:
    raise SystemExit(f"FAIL: mkdir -p timed out after {seconds:g}s: {path}")
except subprocess.CalledProcessError as error:
    raise SystemExit(f"FAIL: mkdir -p failed with exit {error.returncode}: {path}")
PY
}

wait_for_linked_authorized() {
  local seconds="$1"
  local status_json authorization_state

  for _ in $(seq 1 "$((seconds * 5))"); do
    status_json="$("$IDRIVE_CLI" --config-dir "$SMOKE_CONFIG_DIR" status 2>/dev/null || true)"
    authorization_state="$(printf '%s' "$status_json" | json_get account.authorization_state 2>/dev/null || true)"
    if [[ "$authorization_state" == "authorized" ]]; then
      return 0
    fi
    sleep 0.2
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

drive_setup_gui() {
  local mode="$1"
  local field_value="$2"

  /usr/bin/osascript - "$APP_PROCESS_NAME" "$mode" "$field_value" >/dev/null <<'APPLESCRIPT'
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

on clickSetupButton(appName, buttonName, alternateIndex)
  tell application "System Events"
    tell process appName
      set frontmost to true
      set controls to my setupGroup(appName)
      if exists button buttonName of controls then
        click button buttonName of controls
      else
        click button alternateIndex of controls
      end if
    end tell
  end tell
end clickSetupButton

on fillFirstTextField(appName, fieldValue, description)
  set previousClipboard to the clipboard
  set the clipboard to fieldValue
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
          if (value of text field 1 of my setupGroup(appName) as text) is fieldValue then
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
  error description & " field did not accept GUI input"
end fillFirstTextField

on run argv
  set appName to item 1 of argv
  set mode to item 2 of argv
  set fieldValue to item 3 of argv

  tell application "System Events"
    tell process appName
      set frontmost to true
    end tell
  end tell

  my waitForSetupText(appName, "Iris Drive", 10)

  if mode is "create" then
    my clickSetupButton(appName, "Create profile", 1)
    my waitForSetupText(appName, "Create profile", 5)

    if fieldValue is not "" then
      my fillFirstTextField(appName, fieldValue, "Username")
    end if

    my clickSetupButton(appName, "Create profile", 2)
    if fieldValue is "" then return

    my waitForSetupText(appName, "Profile photo", 5)
    my clickSetupButton(appName, "Later", 3)
    return
  end if

  if mode is "link" then
    my clickSetupButton(appName, "Sign in", 2)
    my waitForSetupText(appName, "Sign in", 5)
    my clickSetupButton(appName, "Link this device", 3)
    my waitForSetupText(appName, "Link this device", 5)
    my fillFirstTextField(appName, fieldValue, "Owner")
    my clickSetupButton(appName, "Link device", 2)
    return
  end if

  error "Unknown setup mode: " & mode
end run
APPLESCRIPT
}

drive_create_profile_gui() {
  drive_setup_gui create "$1"
}

drive_link_device_gui() {
  drive_setup_gui link "$1"
}

wait_for_control_panel_text() {
  local expected="$1"
  local seconds="$2"

  /usr/bin/osascript - "$APP_PROCESS_NAME" "$expected" "$seconds" >/dev/null <<'APPLESCRIPT'
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

on run argv
  set appName to item 1 of argv
  set expected to item 2 of argv
  set timeoutSeconds to item 3 of argv as integer
  set deadline to (current date) + timeoutSeconds
  repeat while (current date) is less than deadline
    if my setupStaticTextExists(appName, expected) then return
    delay 0.2
  end repeat
  error "Timed out waiting for control panel text: " & expected
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

request_sidebar_open_button() {
  /usr/bin/swift - "$APP_BUNDLE_ID" "$APP_PROCESS_NAME" >/dev/null <<'SWIFT'
import AppKit
import ApplicationServices
import Foundation

let bundleIdentifier = CommandLine.arguments[1]
let processName = CommandLine.arguments[2]
let targetIdentifier = "sidebarOpenDrive"
let deadline = Date().addingTimeInterval(8)

func stringAttribute(_ element: AXUIElement, _ attribute: String) -> String? {
    var value: CFTypeRef?
    guard AXUIElementCopyAttributeValue(element, attribute as CFString, &value) == .success,
          let value
    else {
        return nil
    }
    return value as? String
}

func children(of element: AXUIElement) -> [AXUIElement] {
    var value: CFTypeRef?
    guard AXUIElementCopyAttributeValue(element, kAXChildrenAttribute as CFString, &value) == .success,
          let children = value as? [AXUIElement]
    else {
        return []
    }
    return children
}

func findElement(_ element: AXUIElement, depth: Int = 0) -> AXUIElement? {
    if stringAttribute(element, "AXIdentifier") == targetIdentifier {
        return element
    }
    guard depth < 20 else {
        return nil
    }
    for child in children(of: element) {
        if let match = findElement(child, depth: depth + 1) {
            return match
        }
    }
    return nil
}

func runningApp() -> NSRunningApplication? {
    if let app = NSRunningApplication
        .runningApplications(withBundleIdentifier: bundleIdentifier)
        .first(where: { !$0.isTerminated }) {
        return app
    }
    return NSWorkspace.shared.runningApplications.first {
        !$0.isTerminated && $0.localizedName == processName
    }
}

var lastError = "sidebar Open button not found"
while Date() < deadline {
    guard let app = runningApp() else {
        lastError = "Iris Drive process not found"
        RunLoop.current.run(until: Date().addingTimeInterval(0.1))
        continue
    }
    app.activate(options: [.activateAllWindows])
    let axApp = AXUIElementCreateApplication(app.processIdentifier)
    if let button = findElement(axApp) {
        let result = AXUIElementPerformAction(button, kAXPressAction as CFString)
        if result == .success {
            exit(0)
        }
        lastError = "sidebar Open button AXPress failed: \(result.rawValue)"
    }
    RunLoop.current.run(until: Date().addingTimeInterval(0.1))
}

fputs("\(lastError)\n", stderr)
exit(1)
SWIFT
}

assert_single_sync_action_button() {
  /usr/bin/swift - "$APP_BUNDLE_ID" "$APP_PROCESS_NAME" >/dev/null <<'SWIFT'
import AppKit
import ApplicationServices
import Foundation

let bundleIdentifier = CommandLine.arguments[1]
let processName = CommandLine.arguments[2]
let deadline = Date().addingTimeInterval(8)

func stringAttribute(_ element: AXUIElement, _ attribute: String) -> String? {
    var value: CFTypeRef?
    guard AXUIElementCopyAttributeValue(element, attribute as CFString, &value) == .success,
          let value
    else {
        return nil
    }
    return value as? String
}

func children(of element: AXUIElement) -> [AXUIElement] {
    var value: CFTypeRef?
    guard AXUIElementCopyAttributeValue(element, kAXChildrenAttribute as CFString, &value) == .success,
          let children = value as? [AXUIElement]
    else {
        return []
    }
    return children
}

func findButtonDescription(_ element: AXUIElement, description: String, depth: Int = 0) -> AXUIElement? {
    if stringAttribute(element, kAXRoleAttribute) == kAXButtonRole as String,
       stringAttribute(element, kAXDescriptionAttribute) == description {
        return element
    }
    guard depth < 20 else {
        return nil
    }
    for child in children(of: element) {
        if let match = findButtonDescription(child, description: description, depth: depth + 1) {
            return match
        }
    }
    return nil
}

func countButtonDescriptions(_ element: AXUIElement, descriptions: Set<String>, depth: Int = 0) -> [String: Int] {
    var counts: [String: Int] = [:]
    if stringAttribute(element, kAXRoleAttribute) == kAXButtonRole as String,
       let description = stringAttribute(element, kAXDescriptionAttribute),
       descriptions.contains(description) {
        counts[description, default: 0] += 1
    }
    guard depth < 20 else {
        return counts
    }
    for child in children(of: element) {
        let childCounts = countButtonDescriptions(child, descriptions: descriptions, depth: depth + 1)
        for (key, value) in childCounts {
            counts[key, default: 0] += value
        }
    }
    return counts
}

func runningApp() -> NSRunningApplication? {
    if let app = NSRunningApplication
        .runningApplications(withBundleIdentifier: bundleIdentifier)
        .first(where: { !$0.isTerminated }) {
        return app
    }
    return NSWorkspace.shared.runningApplications.first {
        !$0.isTerminated && $0.localizedName == processName
    }
}

var lastError = "sync action buttons were not visible"
while Date() < deadline {
    guard let app = runningApp() else {
        lastError = "Iris Drive process not found"
        RunLoop.current.run(until: Date().addingTimeInterval(0.1))
        continue
    }
    app.activate(options: [.activateAllWindows])
    let axApp = AXUIElementCreateApplication(app.processIdentifier)
    if let settingsButton = findButtonDescription(axApp, description: "Settings") {
        _ = AXUIElementPerformAction(settingsButton, kAXPressAction as CFString)
    }
    RunLoop.current.run(until: Date().addingTimeInterval(0.1))
    let counts = countButtonDescriptions(
        axApp,
        descriptions: ["Pause sync", "Resume sync"]
    )
    let pauseCount = counts["Pause sync", default: 0]
    let resumeCount = counts["Resume sync", default: 0]
    if pauseCount + resumeCount == 1 {
        exit(0)
    }
    if pauseCount > 0 && resumeCount > 0 {
        lastError = "Pause sync and Resume sync are both visible"
        break
    }
    RunLoop.current.run(until: Date().addingTimeInterval(0.1))
}

fputs("\(lastError)\n", stderr)
exit(1)
SWIFT
}

app_group_identifier() {
  local app_path="$1"

  codesign -d --entitlements :- "$app_path" 2>/dev/null \
    | python3 -c '
import plistlib
import sys

try:
    entitlements = plistlib.loads(sys.stdin.buffer.read())
except Exception:
    sys.exit(0)
groups = entitlements.get("com.apple.security.application-groups") or []
if groups:
    print(groups[0])
'
}

configure_smoke_app_data() {
  local app_group
  local smoke_name

  if [[ -n "${IRIS_DRIVE_MACOS_SMOKE_APP_DATA:-}" ]]; then
    SMOKE_APP_DATA="$IRIS_DRIVE_MACOS_SMOKE_APP_DATA"
    SMOKE_CONFIG_DIR="$SMOKE_APP_DATA/Config"
    return
  fi

  app_group="$(app_group_identifier "$APP_PATH")"
  if [[ -n "$app_group" ]]; then
    smoke_name="$(basename "$SMOKE_DIR")"
    SMOKE_APP_DATA="$HOME/Library/Group Containers/$app_group/Iris Drive Smoke/$smoke_name"
    SMOKE_CONFIG_DIR="$SMOKE_APP_DATA/Config"
  fi
}

resolve_idrive_cli() {
  if [[ -n "${IRIS_DRIVE_MACOS_SMOKE_IDRIVE:-}" ]]; then
    printf '%s\n' "$IRIS_DRIVE_MACOS_SMOKE_IDRIVE"
    return 0
  fi

  local target_dir
  target_dir="$(cargo metadata --no-deps --format-version 1 \
    | python3 -c 'import json, sys; print(json.load(sys.stdin)["target_directory"])')"
  if [[ -x "$target_dir/debug/idrive" ]]; then
    printf '%s\n' "$target_dir/debug/idrive"
    return 0
  fi

  printf '%s\n' "$APP_PATH/Contents/MacOS/idrive"
}

run_user_journey() {
  local idrive="$IDRIVE_CLI"
  local owner_config_dir="$SMOKE_DIR/owner/Config"
  local source_dir="$SMOKE_DIR/source"
  local backup_dir="$SMOKE_DIR/filesystem-backup"
  local owner_json admin_app_key_npub linked_json linked_app_key_npub authorization_state
  local approve_json roster_size roster_json roster_devices import_json list_json
  local sync_json check_json

  mkdir_p_or_fail "$owner_config_dir"
  owner_json="$("$idrive" --config-dir "$owner_config_dir" init --force --label "macOS owner")" || {
    echo "FAIL: could not initialize owner profile for link journey." >&2
    return 1
  }
  admin_app_key_npub="$(printf '%s' "$owner_json" | json_get current_app_key_npub)" || {
    echo "FAIL: owner init did not return current_app_key_npub." >&2
    echo "$owner_json" >&2
    return 1
  }
  IRIS_DRIVE_FIPS_ENABLE_BOOTSTRAP=false \
    IRIS_DRIVE_FIPS_ENABLE_WEBRTC=false \
    IRIS_DRIVE_FIPS_ENABLE_UDP=false \
    "$idrive" --config-dir "$owner_config_dir" daemon --watch-interval 0 --no-gateway \
    >"$SMOKE_DIR/owner-daemon.stdout.log" 2>"$SMOKE_DIR/owner-daemon.stderr.log" &
  OWNER_DAEMON_PID="$!"
  sleep 1
  if ! kill -0 "$OWNER_DAEMON_PID" >/dev/null 2>&1; then
    echo "FAIL: owner daemon did not start for link journey." >&2
    return 1
  fi

  if ! request_show_control_panel || ! drive_link_device_gui "$admin_app_key_npub"; then
    echo "FAIL: could not complete the Link this device GUI journey." >&2
    return 1
  fi
  if ! wait_for_control_panel_text "Waiting for approval" 10; then
    echo "FAIL: Link this device completed the GUI login before admin approval." >&2
    return 1
  fi

  if ! wait_for_file "$SMOKE_CONFIG_DIR/key" 20; then
    echo "FAIL: Link this device did not initialize local device key material." >&2
    return 1
  fi

  linked_json="$("$idrive" --config-dir "$SMOKE_CONFIG_DIR" whoami)" || {
    echo "FAIL: linked device profile is not readable." >&2
    return 1
  }
  linked_app_key_npub="$(printf '%s' "$linked_json" | json_get current_app_key_npub)" || {
    echo "FAIL: linked profile did not return current_app_key_npub." >&2
    echo "$linked_json" >&2
    return 1
  }
  authorization_state="$(printf '%s' "$linked_json" | json_get authorization_state)" || {
    echo "FAIL: linked profile did not return authorization_state." >&2
    echo "$linked_json" >&2
    return 1
  }
  if [[ "$authorization_state" != "awaiting_approval" ]]; then
    echo "FAIL: linked device should wait for approval, got $authorization_state." >&2
    echo "$linked_json" >&2
    return 1
  fi

  approve_json="$("$idrive" --config-dir "$owner_config_dir" approve "$linked_app_key_npub" --label "Mac GUI linked")" || {
    echo "FAIL: owner could not approve linked GUI device." >&2
    return 1
  }
  roster_size="$(printf '%s' "$approve_json" | json_get roster_size)" || {
    echo "FAIL: approve did not return roster_size." >&2
    echo "$approve_json" >&2
    return 1
  }
  if [[ "$roster_size" != "2" ]]; then
    echo "FAIL: owner roster size after approve was $roster_size, expected 2." >&2
    echo "$approve_json" >&2
    return 1
  fi

  roster_json="$("$idrive" --config-dir "$owner_config_dir" roster)" || {
    echo "FAIL: owner roster could not be listed after approval." >&2
    return 1
  }
  roster_devices="$(printf '%s' "$roster_json" | json_array_len app_keys.devices)" || {
    echo "FAIL: owner roster did not include app_keys.devices." >&2
    echo "$roster_json" >&2
    return 1
  }
  if [[ "$roster_devices" != "2" ]]; then
    echo "FAIL: owner roster listed $roster_devices devices, expected 2." >&2
    echo "$roster_json" >&2
    return 1
  fi
  if ! wait_for_linked_authorized 40; then
    echo "FAIL: linked GUI device did not become authorized after owner approval." >&2
    "$idrive" --config-dir "$SMOKE_CONFIG_DIR" status >&2 || true
    return 1
  fi

  if ! wait_for_daemon 10; then
    echo "FAIL: bundled idrive daemon did not start after Link this device." >&2
    return 1
  fi

  if ! request_sidebar_open_button; then
    echo "FAIL: could not click sidebar Open button during user journey." >&2
    return 1
  fi
  if wait_for_log "Iris Drive mounted drive folder opened" 10 ||
    wait_for_log "Iris Drive mounted drive folder revealed" 1; then
    USER_JOURNEY_OPENED_DRIVE_FOLDER=1
    if require_drive_folder_open &&
      ! wait_for_log "Iris Drive FileProvider domain state userEnabled=true" 10; then
      echo "FAIL: FileProvider domain did not report userEnabled=true." >&2
      return 1
    fi
    if wait_for_log "Iris Drive FileProvider domain state userEnabled=false" 1; then
      echo "FAIL: FileProvider domain is disabled in macOS." >&2
      return 1
    fi
  elif wait_for_log "Iris Drive FileProvider open failed: disabled for this signing mode" 1 &&
    ! require_drive_folder_open; then
    echo "WARN: Show Drive Folder requested, but this app is not FileProvider-capable in its signing mode." >&2
  elif wait_for_log "Iris Drive FileProvider domain state userEnabled=false" 1; then
    echo "FAIL: FileProvider domain is disabled in macOS." >&2
    return 1
  else
    echo "FAIL: Show Drive Folder did not open the drive folder during user journey." >&2
    return 1
  fi

  mkdir_p_or_fail "$source_dir/docs"
  printf 'macOS GUI journey file\n' >"$source_dir/docs/mac-gui-note.txt"
  import_json="$("$idrive" --config-dir "$owner_config_dir" import "$source_dir")" || {
    echo "FAIL: owner could not import journey files." >&2
    return 1
  }
  if [[ "$(printf '%s' "$import_json" | json_get file_count)" != "1" ]]; then
    echo "FAIL: file import did not report one imported file." >&2
    echo "$import_json" >&2
    return 1
  fi

  list_json="$("$idrive" --config-dir "$owner_config_dir" list)" || {
    echo "FAIL: owner could not list imported files." >&2
    return 1
  }
  if ! printf '%s' "$list_json" | json_list_has_path "docs/mac-gui-note.txt"; then
    echo "FAIL: imported journey file was not visible in the drive listing." >&2
    echo "$list_json" >&2
    return 1
  fi

  mkdir_p_or_fail "$backup_dir"
  "$idrive" --config-dir "$owner_config_dir" \
    backups add "fs:$backup_dir" --label "macOS smoke replica" >/dev/null || {
    echo "FAIL: could not add journey backup target." >&2
    return 1
  }
  sync_json="$("$idrive" --config-dir "$owner_config_dir" backups sync)" || {
    echo "FAIL: backup sync failed during user journey." >&2
    return 1
  }
  if ! printf '%s' "$sync_json" | json_report_is_synced filesystem; then
    echo "FAIL: backup sync did not report a synced filesystem target." >&2
    echo "$sync_json" >&2
    return 1
  fi

  check_json="$("$idrive" --config-dir "$owner_config_dir" backups check --sample-size 4)" || {
    echo "FAIL: backup check failed during user journey." >&2
    return 1
  }
  if ! printf '%s' "$check_json" | json_report_check_ran filesystem; then
    echo "FAIL: backup check did not inspect the filesystem target." >&2
    echo "$check_json" >&2
    return 1
  fi
}

APP_PATH="${IRIS_DRIVE_MACOS_SMOKE_APP_PATH:-}"
if [[ -z "$APP_PATH" ]]; then
  APP_PATH="$("$ROOT/scripts/macos-dev-app.sh" build)"
fi
if [[ -z "$APP_PATH" || ! -d "$APP_PATH" ]]; then
  echo "FAIL: macOS app was not built." >&2
  exit 1
fi
configure_smoke_app_data
IDRIVE_CLI="$(resolve_idrive_cli)"
if [[ -z "$IDRIVE_CLI" || ! -x "$IDRIVE_CLI" ]]; then
  echo "FAIL: idrive CLI was not built." >&2
  exit 1
fi

terminate_app_process
rm -rf "$SMOKE_APP_DATA"
if run_create_profile_smoke || run_user_journey_smoke; then
  mkdir_p_or_fail "$SMOKE_HOME"
else
  mkdir_p_or_fail "$SMOKE_CONFIG_DIR"
  "$IDRIVE_CLI" \
    --config-dir "$SMOKE_CONFIG_DIR" \
    init --force --label "macOS smoke" >/dev/null
fi

open_args=(
  --stdout "$APP_STDOUT"
  --stderr "$APP_STDERR"
  --env "IRIS_DRIVE_DEBUG_LOG_DIR=$APP_DEBUG_LOG_DIR"
  --env "IRIS_DRIVE_DISABLE_LOGIN_AGENT_SYNC=true"
)
if ! run_create_profile_gui_smoke && ! run_user_journey_smoke && ! run_ui_smoke; then
  open_args=(-j "${open_args[@]}")
fi
if run_create_profile_smoke || run_user_journey_smoke; then
  open_args+=(
    --env "CFFIXED_USER_HOME=$SMOKE_HOME"
    --env "HOME=$SMOKE_HOME"
    --env "IRIS_DRIVE_APP_BASE_DIR=$SMOKE_APP_DATA"
    --env "IRIS_DRIVE_ENABLE_E2E_NOTIFICATIONS=1"
  )
  if require_drive_folder_open; then
    open_args+=(--env "IRIS_DRIVE_FILEPROVIDER_RESET_ON_START=true")
  fi
else
  open_args+=(--env "IRIS_DRIVE_APP_BASE_DIR=$SMOKE_APP_DATA")
fi
open "${open_args[@]}" "$APP_PATH"

if ! wait_for_app_process 10; then
  echo "FAIL: Iris Drive did not launch." >&2
  show_recent_logs >&2
  exit 1
fi
assert_app_running "immediately after launch"

if ! wait_for_log "Iris Drive menu bar item installed" 10; then
  echo "FAIL: Iris Drive menu bar item was not installed." >&2
  show_recent_logs >&2
  exit 1
fi
assert_app_running "after installing the menu bar item"

if run_create_profile_smoke || run_user_journey_smoke; then
  if ! wait_for_log "Iris Drive local profile not found" 10; then
    echo "FAIL: Iris Drive did not enter first-run setup." >&2
    show_recent_logs >&2
    exit 1
  fi
  assert_app_running "after entering first-run setup"

  if run_user_journey_smoke; then
    if ! run_user_journey; then
      show_recent_logs >&2
      exit 1
    fi
    assert_app_running "after the link-device journey"
  elif run_create_profile_gui_smoke; then
    if ! request_show_control_panel || ! drive_create_profile_gui "$CREATE_PROFILE_USERNAME"; then
      echo "FAIL: could not complete the Create profile GUI journey." >&2
      show_recent_logs >&2
      exit 1
    fi
    assert_app_running "after the Create profile GUI journey"
  else
    if ! request_create_profile; then
      echo "FAIL: could not request Create profile." >&2
      show_recent_logs >&2
      exit 1
    fi
    assert_app_running "after the direct Create profile request"
  fi

  if run_create_profile_smoke && ! wait_for_file "$SMOKE_CONFIG_DIR/key" 20; then
    echo "FAIL: Create profile did not initialize a local profile." >&2
    show_recent_logs >&2
    exit 1
  fi

  if run_create_profile_smoke; then
    status_json="$("$IDRIVE_CLI" --config-dir "$SMOKE_CONFIG_DIR" status)"
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
    assert_app_running "after Create profile daemon startup"
    assert_daemon_running "after Create profile daemon startup"
  fi
else
  if ! wait_for_log "Iris Drive control panel updated" 10; then
    echo "FAIL: Iris Drive did not load control panel status." >&2
    show_recent_logs >&2
    exit 1
  fi
  assert_app_running "after loading control panel status"

  if ! wait_for_daemon 10; then
    echo "FAIL: bundled idrive daemon did not start." >&2
    show_recent_logs >&2
    exit 1
  fi
  assert_app_running "after bundled daemon startup"
  assert_daemon_running "after bundled daemon startup"
fi

if run_ui_smoke && ! run_user_journey_smoke; then
  open "$APP_PATH"
  if ! request_show_control_panel || ! assert_single_sync_action_button || ! request_sidebar_open_button; then
    echo "FAIL: could not verify desktop sync controls or click sidebar Open button." >&2
    show_recent_logs >&2
    exit 1
  fi
  assert_app_running "after desktop UI actions"
  assert_daemon_running "after desktop UI actions"

  if ! wait_for_log "Iris Drive mounted drive folder opened" 10 &&
    ! wait_for_log "Iris Drive mounted drive folder revealed" 1; then
    echo "FAIL: Show Drive Folder did not open the drive folder." >&2
    show_recent_logs >&2
    exit 1
  fi
  assert_app_running "after Show Drive Folder"
  assert_daemon_running "after Show Drive Folder"
fi

sleep "${IRIS_DRIVE_MACOS_SMOKE_SURVIVAL_SECONDS:-5}"
assert_app_running "during post-smoke survival check"
if ! run_create_profile_smoke || [[ -f "$SMOKE_CONFIG_DIR/key" ]]; then
  assert_daemon_running "during post-smoke survival check"
fi

echo "MACOS_SMOKE_OK"
if run_create_profile_smoke; then
  echo "app launched into first-run setup, Create profile initialized a local profile, and bundled daemon started"
elif run_user_journey_smoke; then
  if [[ "$USER_JOURNEY_OPENED_DRIVE_FOLDER" == "1" ]]; then
    echo "app linked a GUI device, opened the drive folder, imported a file, and synced a backup replica"
  else
    echo "app linked a GUI device, handled Show Drive Folder as unavailable in this signing mode, imported a file, and synced a backup replica"
  fi
elif run_ui_smoke; then
  echo "app launched, menu bar item installed, bundled daemon started, and Show Drive Folder opened the drive folder"
else
  echo "app launched hidden, menu bar item installed, control panel status loaded, and bundled daemon started"
fi
