#!/usr/bin/env bash

latest_revealed_drive_path() {
  grep -F "Iris Drive mounted drive folder revealed: " "$APP_STDERR" 2>/dev/null \
    | tail -n 1 \
    | sed 's/^.*Iris Drive mounted drive folder revealed: //'
}

wait_for_finder_target_path() {
  local expected_path="$1"
  local seconds="$2"
  [[ -n "$expected_path" ]] || return 1

  /usr/bin/osascript - "$expected_path" "$seconds" >/dev/null <<'APPLESCRIPT'
on run argv
  set expectedPath to item 1 of argv
  set expectedDirectoryPath to expectedPath & "/"
  set timeoutSeconds to item 2 of argv as integer
  set deadline to (current date) + timeoutSeconds
  repeat while (current date) is less than deadline
    tell application "Finder"
      repeat with windowIndex from 1 to (count of windows)
        try
          set targetFolder to get target of window windowIndex
          set targetPath to POSIX path of (targetFolder as alias)
          if targetPath is expectedPath or targetPath is expectedDirectoryPath then return
        end try
      end repeat
    end tell
    delay 0.2
  end repeat
  error "Timed out waiting for Finder to show " & expectedPath
end run
APPLESCRIPT
}

finder_shows_latest_revealed_drive() {
  local opened_drive_path
  opened_drive_path="$(latest_revealed_drive_path)"
  wait_for_finder_target_path "$opened_drive_path" 10
}
