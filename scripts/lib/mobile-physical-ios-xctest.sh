#!/usr/bin/env bash
# Physical-device XCUITest marker and runner readiness helpers.

copy_ios_markers() {
  rm -f "$IOS_MARKERS"
  xcrun devicectl device copy from \
    --device "$IOS_DEVICE_SELECTED" \
    --domain-type appDataContainer \
    --domain-identifier "$IOS_RUNNER_BUNDLE_ID" \
    --source "Documents/$IOS_MARKER_NAME" \
    --destination "$IOS_MARKERS" >/dev/null 2>&1
}

wait_for_ios_marker() {
  local marker="$1"
  local seconds="$2"
  local deadline=$((SECONDS + seconds))
  while (( SECONDS < deadline )); do
    if copy_ios_markers \
      && grep -Fxq "IRIS_XCUITEST_RUN_ID=$RUN_ID" "$IOS_MARKERS" \
      && grep -Fq "$marker" "$IOS_MARKERS"; then
      return 0
    fi
    if [[ -n "$IOS_TEST_PID" ]] && ! kill -0 "$IOS_TEST_PID" >/dev/null 2>&1; then
      return 1
    fi
    sleep 0.25
  done
  return 1
}

wait_for_ios_runner_or_marker() {
  local marker="$1"
  local seconds="$2"
  local processes="$TMP/ios-processes.json"
  local deadline=$((SECONDS + seconds))
  while (( SECONDS < deadline )); do
    if copy_ios_markers \
      && grep -Fxq "IRIS_XCUITEST_RUN_ID=$RUN_ID" "$IOS_MARKERS" \
      && grep -Fq "$marker" "$IOS_MARKERS"; then
      return 0
    fi
    if xcrun devicectl device info processes \
      --device "$IOS_DEVICE_SELECTED" --json-output "$processes" >/dev/null 2>&1 \
      && python3 - "$processes" <<'PY'
import json
import sys

with open(sys.argv[1], "r", encoding="utf-8") as handle:
    result = json.load(handle).get("result") or {}
processes = result.get("runningProcesses") or result.get("processes") or []
running = any(
    str(process.get("executable") or "").endswith("/IrisDriveIOSUITests-Runner")
    for process in processes
)
raise SystemExit(0 if running else 1)
PY
    then
      return 0
    fi
    if [[ -n "$IOS_TEST_PID" ]] && ! kill -0 "$IOS_TEST_PID" >/dev/null 2>&1; then
      return 1
    fi
    sleep 0.5
  done
  return 1
}
