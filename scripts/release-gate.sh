#!/usr/bin/env bash

set -Eeuo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$ROOT"

usage() {
  cat <<'USAGE'
Usage: scripts/release-gate.sh [--full]

Runs the local release confidence gate. The default gate is deterministic and
host-local. --full additionally runs the five-platform lab e2e, which requires
the configured Linux, Windows, macOS, iOS, and Android hosts/devices.

Environment:
  IRIS_DRIVE_RELEASE_GATE_FULL=1       Same as --full.
  IRIS_DRIVE_RELEASE_GATE_ANDROID=0    Skip local Android build/smoke.
  IRIS_DRIVE_RELEASE_GATE_IOS=0        Skip local iOS build/smoke.
  IRIS_DRIVE_RELEASE_GATE_MACOS=0      Skip local macOS build/smoke.
  IRIS_DRIVE_RELEASE_GATE_IDLE_CPU=0   Skip idle CPU sampling gates.
  IRIS_DRIVE_RELEASE_GATE_ANDROID_IDLE_CPU_WARMUP_SECS=90
                                        Override Android idle CPU warmup.
  IRIS_DRIVE_RELEASE_GATE_MACOS_IDLE_CPU_WARMUP_SECS=60
                                        Override macOS idle CPU warmup.
USAGE
}

bool_true() {
  case "${1:-}" in
    1 | true | TRUE | True | yes | YES | Yes | on | ON | On) return 0 ;;
    *) return 1 ;;
  esac
}

idle_cpu_gate_enabled() {
  [[ "${IRIS_DRIVE_RELEASE_GATE_IDLE_CPU:-1}" != "0" ]]
}

run() {
  printf '[release-gate] %s\n' "$*" >&2
  "$@"
}

run_parallel_checks() {
  local tmpdir
  tmpdir="$(mktemp -d -t iris-drive-release-gate.XXXXXX)"
  local labels=()
  local pids=()
  local logs=()

  start_check() {
    local label="$1"
    shift
    local logfile="$tmpdir/$label.log"
    labels+=("$label")
    logs+=("$logfile")
    printf '[release-gate] %s\n' "$*" >&2
    ("$@" >"$logfile" 2>&1) &
    pids+=("$!")
  }

  start_check local-release-tests node --test scripts/local-release*.test.mjs
  start_check fmt cargo fmt --check
  start_check structure just structure
  start_check workspace-tests cargo test --workspace --exclude idrive

  local failed=0
  local index
  for index in "${!pids[@]}"; do
    if wait "${pids[$index]}"; then
      sed "s/^/[release-gate:${labels[$index]}] /" "${logs[$index]}"
    else
      failed=1
      sed "s/^/[release-gate:${labels[$index]}] /" "${logs[$index]}" >&2
      printf '[release-gate] %s failed\n' "${labels[$index]}" >&2
    fi
  done
  rm -rf "$tmpdir"
  return "$failed"
}

run_rust_tests() {
  run cargo build -p idrive --bin idrive
  run cargo test -p idrive --bin idrive --test cli_e2e --test link_input_e2e -- --test-threads=1
  run cargo test -p idrive --test daemon_sync_matrix -- --test-threads=1
}

run_macos_idle_cpu_gate() {
  local app_base_dir
  local config_dir
  local idrive
  local output
  local app_path
  local status=0

  app_base_dir="$(mktemp -d -t iris-drive-macos-idle.XXXXXX)"
  config_dir="$app_base_dir/Config"
  idrive="${CARGO_TARGET_DIR:-$HOME/.cache/cargo-target}/debug/idrive"
  if [[ ! -x "$idrive" ]]; then
    idrive="$ROOT/target/debug/idrive"
  fi
  if [[ ! -x "$idrive" ]]; then
    echo "macOS idle CPU gate needs a debug idrive binary; run cargo build -p idrive --bin idrive first." >&2
    rm -rf "$app_base_dir"
    return 1
  fi
  mkdir -p "$config_dir"
  "$idrive" --config-dir "$config_dir" init --force --label "macOS idle CPU gate" >/dev/null

  output="$(
    IRIS_DRIVE_MACOS_SIGNING="${IRIS_DRIVE_MACOS_SIGNING:-none}" \
      IRIS_DRIVE_APP_BASE_DIR="$app_base_dir" \
      ./scripts/macos-dev-app.sh run
  )"
  printf '%s\n' "$output"
  app_path="$(printf '%s\n' "$output" | sed -n 's/^macOS app launched: //p' | tail -n 1)"
  if [[ -z "$app_path" || ! -d "$app_path" ]]; then
    echo "macOS idle CPU gate could not determine launched app path." >&2
    rm -rf "$app_base_dir"
    return 1
  fi

  IRIS_DRIVE_IDLE_CPU_REQUIRED_ROLES="${IRIS_DRIVE_RELEASE_GATE_MACOS_IDLE_CPU_ROLES:-app,daemon}" \
    IRIS_DRIVE_IDLE_CPU_WARMUP_SECS="${IRIS_DRIVE_RELEASE_GATE_MACOS_IDLE_CPU_WARMUP_SECS:-60}" \
    IRIS_DRIVE_IDLE_CPU_COMMAND_MATCH="$app_path" \
    ./scripts/idle-cpu-gate.sh --platform macos || status=$?

  "$app_path/Contents/MacOS/idrive" \
    --config-dir "$config_dir" \
    service uninstall --json >/dev/null 2>&1 || true
  pkill -f "$app_path/Contents/MacOS/idrive.*daemon" >/dev/null 2>&1 || true
  osascript -e 'tell application "Iris Drive" to quit' >/dev/null 2>&1 || true
  rm -rf "$app_base_dir"
  return "$status"
}

full="${IRIS_DRIVE_RELEASE_GATE_FULL:-0}"
while [[ $# -gt 0 ]]; do
  case "$1" in
    --full)
      full=1
      shift
      ;;
    -h | --help)
      usage
      exit 0
      ;;
    *)
      usage >&2
      exit 2
      ;;
  esac
done

export CARGO_INCREMENTAL="${CARGO_INCREMENTAL:-0}"
export LC_ALL="${LC_ALL:-C}"
export TZ="${TZ:-UTC}"
if [[ -z "${SOURCE_DATE_EPOCH:-}" ]]; then
  SOURCE_DATE_EPOCH="$(git log -1 --format=%ct HEAD 2>/dev/null || printf '0')"
  export SOURCE_DATE_EPOCH
fi

run_parallel_checks
run_rust_tests
run cargo build --workspace --release

case "$(uname -s)" in
  Darwin)
    if ! bool_true "${IRIS_DRIVE_RELEASE_GATE_MACOS_SKIP:-0}" \
      && [[ "${IRIS_DRIVE_RELEASE_GATE_MACOS:-1}" != "0" ]]; then
      run just macos-build
      run env \
        IRIS_DRIVE_MACOS_SIGNING="${IRIS_DRIVE_MACOS_SIGNING:-none}" \
        IRIS_DRIVE_DISABLE_DAEMON_SERVICE="${IRIS_DRIVE_RELEASE_GATE_MACOS_DAEMON_SERVICE:-true}" \
        just smoke-macos
      if idle_cpu_gate_enabled; then
        run run_macos_idle_cpu_gate
      fi
    fi
    if ! bool_true "${IRIS_DRIVE_RELEASE_GATE_IOS_SKIP:-0}" \
      && [[ "${IRIS_DRIVE_RELEASE_GATE_IOS:-1}" != "0" ]]; then
      # ios-smoke builds the simulator app before exercising it.
      run just ios-smoke
      run just ios-gui-smoke
      if idle_cpu_gate_enabled; then
        run env \
          IRIS_DRIVE_IDLE_CPU_REQUIRED_ROLES="${IRIS_DRIVE_RELEASE_GATE_IOS_IDLE_CPU_ROLES:-app}" \
          IRIS_DRIVE_IDLE_CPU_IOS_DEVICE="${IRIS_DRIVE_IOS_SIMULATOR_DEVICE:-${IRIS_DRIVE_IOS_DEVICE:-}}" \
          ./scripts/idle-cpu-gate.sh --platform ios
      fi
    fi
    if ! bool_true "${IRIS_DRIVE_RELEASE_GATE_ANDROID_SKIP:-0}" \
      && [[ "${IRIS_DRIVE_RELEASE_GATE_ANDROID:-1}" != "0" ]]; then
      run just android-build
      run env IRIS_DRIVE_ANDROID_KEEP_TEST_APP=true just android-gui-smoke
      if idle_cpu_gate_enabled; then
        run env \
          IRIS_DRIVE_IDLE_CPU_REQUIRED_ROLES="${IRIS_DRIVE_RELEASE_GATE_ANDROID_IDLE_CPU_ROLES:-app}" \
          IRIS_DRIVE_IDLE_CPU_WARMUP_SECS="${IRIS_DRIVE_RELEASE_GATE_ANDROID_IDLE_CPU_WARMUP_SECS:-90}" \
          IRIS_DRIVE_IDLE_CPU_ANDROID_PACKAGE="${IRIS_DRIVE_ANDROID_PACKAGE:-to.iris.drive.uitest}" \
          ./scripts/idle-cpu-gate.sh --platform android
      fi
    fi
    ;;
  Linux)
    run just linux-build
    if idle_cpu_gate_enabled && bool_true "${IRIS_DRIVE_RELEASE_GATE_LINUX_IDLE_CPU:-0}"; then
      run env \
        IRIS_DRIVE_IDLE_CPU_REQUIRED_ROLES="${IRIS_DRIVE_RELEASE_GATE_LINUX_IDLE_CPU_ROLES:-daemon}" \
        ./scripts/idle-cpu-gate.sh --platform linux
    fi
    ;;
esac

if bool_true "$full"; then
  run just e2e-5devices
else
  printf '[release-gate] skipping five-platform e2e; pass --full for just e2e-5devices\n' >&2
fi

printf '[release-gate] ok\n' >&2
