#!/usr/bin/env bash

set -Eeuo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"

usage() {
  cat <<'USAGE'
Usage:
  scripts/e2e-everything-3vms.sh [dev-lab args]

Runs the full development confidence battery:
  1. cargo test --workspace
  2. update/build/run the configured macOS, Ubuntu, and Windows dev VMs
  3. run the native 3-VM sync smoke against the real OS file-provider surfaces

The VM hostnames and signing config come from ~/.config/iris-drive/dev-lab.env
or the local git remotes consumed by scripts/dev-lab.sh. Keep private hostnames
there; do not commit them to this repo.
Per-hop sync timings are written to target/e2e-3vms-*-timings.jsonl.

Environment:
  IRIS_DRIVE_E2E_SKIP_CARGO=1   Skip cargo test --workspace.
  IRIS_DRIVE_E2E_SKIP_DEPLOY=1  Skip scripts/dev-lab.sh and only run the VM e2e smoke.
USAGE
}

if [[ "${1:-}" == "-h" || "${1:-}" == "--help" ]]; then
  usage
  exit 0
fi

log() {
  printf '[e2e-3vms] %s\n' "$*" >&2
}

cd "$ROOT"

if [[ "${IRIS_DRIVE_E2E_SKIP_CARGO:-0}" != "1" ]]; then
  log "running Rust workspace tests"
  cargo test --workspace
fi

if [[ "${IRIS_DRIVE_E2E_SKIP_DEPLOY:-0}" != "1" ]]; then
  log "updating/building/running configured dev VMs"
  "$ROOT/scripts/dev-lab.sh" "$@"
fi

log "running native 3-VM sync e2e"
"$ROOT/scripts/dev-vm-smoke.sh"
log "ok"
