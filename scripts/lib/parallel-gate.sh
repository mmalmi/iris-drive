#!/usr/bin/env bash

# Bash 3.2-compatible runner for independent verification lanes. Output stays
# grouped, and exit 75 survives when every failed lane reports unavailable
# infrastructure.

parallel_group_abort() {
  local status="${1:-$?}"
  local pid

  trap - EXIT HUP INT TERM
  for pid in "${PARALLEL_GROUP_PIDS[@]:-}"; do
    kill "$pid" >/dev/null 2>&1 || true
  done
  for pid in "${PARALLEL_GROUP_PIDS[@]:-}"; do
    wait "$pid" >/dev/null 2>&1 || true
  done
  if [[ -n "${PARALLEL_GROUP_TMPDIR:-}" ]]; then
    rm -rf "$PARALLEL_GROUP_TMPDIR"
  fi
  parallel_group_restore_traps
  exit "$status"
}

parallel_group_restore_traps() {
  trap - EXIT HUP INT TERM
  [[ -z "${PARALLEL_GROUP_SAVED_EXIT_TRAP:-}" ]] || eval "$PARALLEL_GROUP_SAVED_EXIT_TRAP"
  [[ -z "${PARALLEL_GROUP_SAVED_HUP_TRAP:-}" ]] || eval "$PARALLEL_GROUP_SAVED_HUP_TRAP"
  [[ -z "${PARALLEL_GROUP_SAVED_INT_TRAP:-}" ]] || eval "$PARALLEL_GROUP_SAVED_INT_TRAP"
  [[ -z "${PARALLEL_GROUP_SAVED_TERM_TRAP:-}" ]] || eval "$PARALLEL_GROUP_SAVED_TERM_TRAP"
}

parallel_group_begin() {
  PARALLEL_GROUP_SAVED_EXIT_TRAP="$(trap -p EXIT)"
  PARALLEL_GROUP_SAVED_HUP_TRAP="$(trap -p HUP)"
  PARALLEL_GROUP_SAVED_INT_TRAP="$(trap -p INT)"
  PARALLEL_GROUP_SAVED_TERM_TRAP="$(trap -p TERM)"
  PARALLEL_GROUP_PREFIX="$1"
  PARALLEL_GROUP_TMPDIR="$(mktemp -d -t iris-drive-parallel.XXXXXX)"
  PARALLEL_GROUP_LABELS=()
  PARALLEL_GROUP_PIDS=()
  PARALLEL_GROUP_LOGS=()
  trap 'parallel_group_abort $?' EXIT
  trap 'parallel_group_abort 129' HUP
  trap 'parallel_group_abort 130' INT
  trap 'parallel_group_abort 143' TERM
}

parallel_group_run_lane() {
  local logfile="$1"
  shift
  local child="" status=0

  trap 'if [[ -n "$child" ]]; then kill "$child" >/dev/null 2>&1 || true; wait "$child" >/dev/null 2>&1 || true; fi; exit 129' HUP
  trap 'if [[ -n "$child" ]]; then kill "$child" >/dev/null 2>&1 || true; wait "$child" >/dev/null 2>&1 || true; fi; exit 130' INT
  trap 'if [[ -n "$child" ]]; then kill "$child" >/dev/null 2>&1 || true; wait "$child" >/dev/null 2>&1 || true; fi; exit 143' TERM
  "$@" >"$logfile" 2>&1 &
  child="$!"
  wait "$child" || status=$?
  trap - HUP INT TERM
  return "$status"
}

parallel_group_start() {
  local label="$1"
  shift
  local logfile="$PARALLEL_GROUP_TMPDIR/${#PARALLEL_GROUP_LABELS[@]}.log"

  PARALLEL_GROUP_LABELS+=("$label")
  PARALLEL_GROUP_LOGS+=("$logfile")
  printf '[%s] %s\n' "$PARALLEL_GROUP_PREFIX" "$*" >&2
  parallel_group_run_lane "$logfile" "$@" &
  PARALLEL_GROUP_PIDS+=("$!")
}

parallel_group_wait() {
  local result=0
  local lane_status=0
  local index

  for index in "${!PARALLEL_GROUP_PIDS[@]}"; do
    if wait "${PARALLEL_GROUP_PIDS[$index]}"; then
      lane_status=0
    else
      lane_status=$?
    fi

    if [[ "$lane_status" -eq 0 ]]; then
      sed "s/^/[$PARALLEL_GROUP_PREFIX:${PARALLEL_GROUP_LABELS[$index]}] /" \
        "${PARALLEL_GROUP_LOGS[$index]}"
      continue
    fi

    sed "s/^/[$PARALLEL_GROUP_PREFIX:${PARALLEL_GROUP_LABELS[$index]}] /" \
      "${PARALLEL_GROUP_LOGS[$index]}" >&2
    printf '[%s] %s failed with status %s\n' \
      "$PARALLEL_GROUP_PREFIX" "${PARALLEL_GROUP_LABELS[$index]}" "$lane_status" >&2
    if [[ "$lane_status" -ne 75 || "$result" -eq 0 ]]; then
      result="$lane_status"
    fi
  done

  rm -rf "$PARALLEL_GROUP_TMPDIR"
  PARALLEL_GROUP_TMPDIR=""
  PARALLEL_GROUP_PIDS=()
  parallel_group_restore_traps
  return "$result"
}
