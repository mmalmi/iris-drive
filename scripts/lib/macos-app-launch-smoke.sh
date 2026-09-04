# shellcheck shell=bash
# shellcheck disable=SC2154

MACOS_SMOKE_APP_READY_TIMEOUT_SECS=10
MACOS_SMOKE_APP_LAUNCH_FAILURE=""
MACOS_SMOKE_APPINTENTS_LOG="$SMOKE_STATE_DIR/appintents-linkd-stall.log"

macos_app_has_product_log() {
  local log_file
  for log_file in "$APP_STDOUT" "$APP_STDERR" "$APP_DEBUG_LOG"; do
    [[ ! -s "$log_file" ]] || return 0
  done
  return 1
}

macos_appintents_linkd_stall_for_pid() {
  local pid="$1"
  /usr/bin/log show --last 2m --style compact \
    --predicate "processIdentifier == $pid AND eventMessage CONTAINS[c] \"linkd.autoShortcut\"" \
    >"$MACOS_SMOKE_APPINTENTS_LOG" 2>/dev/null || return 1
  grep -F '4097' "$MACOS_SMOKE_APPINTENTS_LOG" >/dev/null
}

macos_launch_stall_is_retryable() {
  local pid pids
  app_is_running || return 1
  ! macos_app_has_product_log || return 1
  pids="$(app_process_pids)"
  [[ "$(printf '%s\n' "$pids" | sed '/^$/d' | wc -l | tr -d ' ')" == 1 ]] || return 1
  pid="$(printf '%s\n' "$pids" | sed -n '1p')"
  [[ "$pid" =~ ^[0-9]+$ ]] || return 1
  macos_appintents_linkd_stall_for_pid "$pid"
}

launch_macos_smoke_app_once() {
  MACOS_SMOKE_APP_LAUNCH_FAILURE=""
  if ! open "${open_args[@]}" "$APP_PATH"; then
    MACOS_SMOKE_APP_LAUNCH_FAILURE="open command failed"
    return 1
  fi
  if ! wait_for_app_process "$MACOS_SMOKE_APP_READY_TIMEOUT_SECS"; then
    MACOS_SMOKE_APP_LAUNCH_FAILURE="app process did not launch"
    return 1
  fi
  if ! wait_for_log "Iris Drive menu bar item installed" \
    "$MACOS_SMOKE_APP_READY_TIMEOUT_SECS"; then
    MACOS_SMOKE_APP_LAUNCH_FAILURE="menu bar readiness log was not emitted"
    return 1
  fi
}

launch_macos_smoke_app_with_targeted_recovery() {
  if launch_macos_smoke_app_once; then
    return 0
  fi
  [[ "$MACOS_SMOKE_APP_LAUNCH_FAILURE" == "menu bar readiness log was not emitted" ]] \
    || return 1
  macos_launch_stall_is_retryable || return 1

  echo "WARN: relaunching once after a proven macOS AppIntents/linkd pre-start stall." >&2
  terminate_app_process
  if app_is_running; then
    MACOS_SMOKE_APP_LAUNCH_FAILURE="AppIntents-stalled process did not terminate"
    return 1
  fi
  if launch_macos_smoke_app_once; then
    return 0
  fi
  MACOS_SMOKE_APP_LAUNCH_FAILURE="targeted AppIntents relaunch failed: $MACOS_SMOKE_APP_LAUNCH_FAILURE"
  return 1
}
