#!/usr/bin/env bash

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

daemon_process_pids() {
  local pid command
  local path_fragment="$APP_PATH/Contents/MacOS/idrive"
  local config_fragment="--config-dir $SMOKE_CONFIG_DIR"

  [[ -n "${APP_PATH:-}" && -n "${SMOKE_CONFIG_DIR:-}" ]] || return 0
  while IFS= read -r pid; do
    command="$(ps -p "$pid" -o command= 2>/dev/null || true)"
    if [[ "$command" == *"$path_fragment"* &&
      "$command" == *"$config_fragment"* &&
      "$command" == *" daemon"* ]]; then
      printf '%s\n' "$pid"
    fi
  done < <(pgrep -f "idrive.*daemon" 2>/dev/null || true)
}

terminate_smoke_daemon_processes() {
  local pid signal

  for signal in TERM KILL; do
    for pid in $(daemon_process_pids); do
      kill "-$signal" "$pid" >/dev/null 2>&1 || true
    done
    for _ in {1..40}; do
      [[ -z "$(daemon_process_pids)" ]] && return 0
      sleep 0.1
    done
  done
}
