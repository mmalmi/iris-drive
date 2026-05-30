#!/usr/bin/env bash

set -Eeuo pipefail

usage() {
  cat <<'USAGE'
Usage:
  IRIS_DRIVE_E2E_UBUNTU_HOST=<ssh-host> \
  IRIS_DRIVE_E2E_WINDOWS_HOST=<ssh-host> \
  IRIS_DRIVE_E2E_MACOS_HOST=<ssh-host> \
  IRIS_DRIVE_E2E_IOS_HOST=<ssh-host> \
    scripts/cross-vm-four-platform-e2e.sh [cross-vm-e2e args]

Runs the multidevice sync harness across four labeled peers:
  ubuntu  posix
  windows windows
  macos   posix
  ios     posix daemon peer plus iOS simulator app smoke

The iOS host should be a macOS machine reachable by SSH with Xcode, an iOS
simulator runtime, and the iris-drive checkout at ~/src/iris-drive. The iOS
peer uses provider commands in the sync harness; no mobile folder mount is
created.
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
IOS_HOST="$(required_env IRIS_DRIVE_E2E_IOS_HOST)"

echo "[e2e-4devices] running iOS simulator smoke on $IOS_HOST" >&2
ssh "$IOS_HOST" 'cd "$HOME/src/iris-drive" && scripts/ios-simulator-smoke.sh'

echo "[e2e-4devices] running iOS GUI linking smoke on $IOS_HOST" >&2
ssh "$IOS_HOST" 'cd "$HOME/src/iris-drive" && scripts/ios-gui-linking-smoke.sh'

if [[ -z "${IRIS_DRIVE_E2E_MOUNT_LABELS+x}" ]]; then
  export IRIS_DRIVE_E2E_MOUNT_LABELS="ubuntu macos"
fi

exec "$ROOT/scripts/cross-vm-e2e.sh" \
  --host "ubuntu=posix:$UBUNTU_HOST" \
  --host "windows=windows:$WINDOWS_HOST" \
  --host "macos=posix:$MACOS_HOST" \
  --host "ios=posix:$IOS_HOST" \
  "$@"
