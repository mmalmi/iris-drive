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
USAGE
}

bool_true() {
  case "${1:-}" in
    1 | true | TRUE | True | yes | YES | Yes | on | ON | On) return 0 ;;
    *) return 1 ;;
  esac
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

  start_check local-release-tests node --test scripts/local-release.test.mjs
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
  run cargo test -p idrive --bin idrive --test cli_e2e --test link_input_e2e -- --test-threads=1
  run cargo test -p idrive --test daemon_sync_matrix -- --test-threads=1
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
      run env IRIS_DRIVE_MACOS_SIGNING="${IRIS_DRIVE_MACOS_SIGNING:-none}" just smoke-macos
    fi
    if ! bool_true "${IRIS_DRIVE_RELEASE_GATE_IOS_SKIP:-0}" \
      && [[ "${IRIS_DRIVE_RELEASE_GATE_IOS:-1}" != "0" ]]; then
      run just ios-build
      run just ios-smoke
      run just ios-gui-smoke
    fi
    if ! bool_true "${IRIS_DRIVE_RELEASE_GATE_ANDROID_SKIP:-0}" \
      && [[ "${IRIS_DRIVE_RELEASE_GATE_ANDROID:-1}" != "0" ]]; then
      run just android-build
      run just android-gui-smoke
    fi
    ;;
  Linux)
    run just linux-build
    ;;
esac

if bool_true "$full"; then
  run just e2e-5devices
else
  printf '[release-gate] skipping five-platform e2e; pass --full for just e2e-5devices\n' >&2
fi

printf '[release-gate] ok\n' >&2
