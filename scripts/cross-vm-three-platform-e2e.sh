#!/usr/bin/env bash

set -Eeuo pipefail

usage() {
  cat <<'USAGE'
Usage:
  IRIS_DRIVE_E2E_UBUNTU_HOST=<ssh-host> \
  IRIS_DRIVE_E2E_WINDOWS_HOST=<ssh-host> \
  IRIS_DRIVE_E2E_MACOS_HOST=<ssh-host> \
    scripts/cross-vm-three-platform-e2e.sh [cross-vm-e2e args]

Runs the real cross-VM sync harness across three labeled peers:
  ubuntu  posix
  windows windows
  macos   posix

Hostnames intentionally come from environment variables so private SSH names
stay in .local scripts, shell history, or operator config rather than tracked
repository files.
USAGE
}

required_env() {
  local name="$1"
  local value="${!name:-}"
  if [[ -z "$value" ]]; then
    echo "$name is required" >&2
    usage >&2
    exit 2
  fi
  printf "%s" "$value"
}

if [[ "${1:-}" == "-h" || "${1:-}" == "--help" ]]; then
  usage
  exit 0
fi

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
UBUNTU_HOST="$(required_env IRIS_DRIVE_E2E_UBUNTU_HOST)"
WINDOWS_HOST="$(required_env IRIS_DRIVE_E2E_WINDOWS_HOST)"
MACOS_HOST="$(required_env IRIS_DRIVE_E2E_MACOS_HOST)"

exec "$ROOT/scripts/cross-vm-e2e.sh" \
  --host "ubuntu=posix:$UBUNTU_HOST" \
  --host "windows=windows:$WINDOWS_HOST" \
  --host "macos=posix:$MACOS_HOST" \
  "$@"
