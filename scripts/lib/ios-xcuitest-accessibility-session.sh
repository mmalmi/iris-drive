#!/usr/bin/env bash
# Exact XCUITest accessibility-session failure classification and recovery.

ios_xcuitest_accessibility_session_disabled_after() {
  local log="$1" byte_offset="$2"
  [[ -f "$log" && "$byte_offset" =~ ^[0-9]+$ ]] || return 1
  python3 - "$log" "$byte_offset" <<'PY'
import pathlib, sys
path=pathlib.Path(sys.argv[1])
offset=int(sys.argv[2])
with path.open("rb") as handle:
    handle.seek(offset)
    text=handle.read().decode("utf-8", errors="replace")
raise SystemExit(0 if "Error getting main window kAXErrorAPIDisabled" in text else 1)
PY
}

ios_xcuitest_automation_mode_timed_out_after() {
  local log="$1" byte_offset="$2"
  [[ -f "$log" && "$byte_offset" =~ ^[0-9]+$ ]] || return 1
  python3 - "$log" "$byte_offset" <<'PY'
import pathlib, sys
path=pathlib.Path(sys.argv[1])
offset=int(sys.argv[2])
with path.open("rb") as handle:
    handle.seek(offset)
    text=handle.read().decode("utf-8", errors="replace")
raise SystemExit(0 if "Timed out while enabling automation mode." in text else 1)
PY
}

restart_ios_simulator_accessibility_session() {
  local device_udid="$1"
  xcrun simctl shutdown "$device_udid" >/dev/null
  xcrun simctl boot "$device_udid" >/dev/null
  xcrun simctl bootstatus "$device_udid" -b >/dev/null
}
