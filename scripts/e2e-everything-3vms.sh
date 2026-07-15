#!/usr/bin/env bash

set -Eeuo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"

usage() {
  cat <<'USAGE'
Usage:
  scripts/e2e-everything-3vms.sh [options] [dev-lab args]

Runs the full development confidence battery:
  1. cargo test --workspace -- --test-threads=1
  2. update/build/run the configured macOS, Ubuntu, and Windows dev VMs
  3. run the native 3-VM sync smoke against the real OS file-provider surfaces

The VM hostnames and signing config come from ~/.config/iris-drive/dev-lab.env
or the local git remotes consumed by scripts/dev-lab.sh. Keep private hostnames
there; do not commit them to this repo.
Per-hop sync timings are written to target/e2e-3vms-*-timings.jsonl.

Options:
  --fail-fast              Pass --fail-fast to the Rust test harness.
  --rust-filter FILTER     Run only matching Rust tests before the VM e2e.
                           Use FILTER=all or omit this option for all tests.
  --smoke-only PHASE       Run one native smoke phase: all, sync,
                           heavy-projection, linux-ui, windows-ui, desktop-ui,
                           or macos-ui.

Environment:
  IRIS_DRIVE_E2E_SKIP_CARGO=1   Skip cargo test --workspace.
  IRIS_DRIVE_E2E_SKIP_DEPLOY=1  Skip scripts/dev-lab.sh and only run the VM e2e smoke.
  IRIS_DRIVE_E2E_RUST_FILTER    Same as --rust-filter.
  IRIS_DRIVE_DEV_VM_CONNECTIVITY_TIMEOUT
                                  Override the FIPS roster wait before smoke.
USAGE
}

log() {
  printf '[e2e-3vms] %s\n' "$*" >&2
}

DEV_LAB_ARGS=()
RUST_FILTER="${IRIS_DRIVE_E2E_RUST_FILTER:-}"
RUST_FAIL_FAST=0

while [[ $# -gt 0 ]]; do
  case "$1" in
    -h|--help)
      usage
      exit 0
      ;;
    --fail-fast)
      RUST_FAIL_FAST=1
      shift
      ;;
    --rust-filter)
      [[ $# -ge 2 ]] || {
        log "ERROR: --rust-filter needs a value"
        exit 2
      }
      RUST_FILTER="$2"
      shift 2
      ;;
    --smoke-only)
      [[ $# -ge 2 ]] || {
        log "ERROR: --smoke-only needs a value"
        exit 2
      }
      export IRIS_DRIVE_DEV_VM_SMOKE_ONLY="$2"
      shift 2
      ;;
    --)
      shift
      DEV_LAB_ARGS+=("$@")
      break
      ;;
    *)
      DEV_LAB_ARGS+=("$1")
      shift
      ;;
  esac
done

cd "$ROOT"

if [[ -z "${IRIS_DRIVE_DEV_VM_CONNECTIVITY_TIMEOUT:-}" && -n "${IRIS_DRIVE_DEV_VM_MAX_SYNC_WAIT_TIMEOUT:-}" ]]; then
  export IRIS_DRIVE_DEV_VM_CONNECTIVITY_TIMEOUT="$IRIS_DRIVE_DEV_VM_MAX_SYNC_WAIT_TIMEOUT"
fi

if [[ "${IRIS_DRIVE_E2E_SKIP_CARGO:-0}" != "1" ]]; then
  log "running Rust workspace tests"
  if [[ "$RUST_FAIL_FAST" == "1" ]] && cargo nextest --version >/dev/null 2>&1; then
    nextest_args=(nextest run --workspace --fail-fast -j 1)
    if [[ -n "$RUST_FILTER" && "$RUST_FILTER" != "all" ]]; then
      nextest_args+=("$RUST_FILTER")
    fi
    cargo "${nextest_args[@]}"
  else
    if [[ "$RUST_FAIL_FAST" == "1" ]]; then
      log "cargo-nextest not available; stable cargo test cannot fail-fast inside one test binary"
    fi
    cargo_args=(test --workspace)
    if [[ -n "$RUST_FILTER" && "$RUST_FILTER" != "all" ]]; then
      cargo_args+=("$RUST_FILTER")
    fi
    cargo "${cargo_args[@]}" -- --test-threads=1
  fi
fi

if [[ "${IRIS_DRIVE_E2E_SKIP_DEPLOY:-0}" != "1" ]]; then
  log "updating/building/running configured dev VMs"
  if [[ ${#DEV_LAB_ARGS[@]} -gt 0 ]]; then
    "$ROOT/scripts/dev-lab.sh" "${DEV_LAB_ARGS[@]}"
  else
    "$ROOT/scripts/dev-lab.sh"
  fi
fi

log "running native 3-VM sync e2e"
"$ROOT/scripts/dev-vm-smoke.sh"
log "ok"
