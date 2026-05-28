#!/usr/bin/env bash

set -Eeuo pipefail

usage() {
  cat <<'USAGE'
Usage:
  IRIS_DRIVE_E2E_UBUNTU_HOST=<ssh-host> \
  IRIS_DRIVE_E2E_WINDOWS_HOST=<ssh-host> \
  IRIS_DRIVE_E2E_MACOS_HOST=<ssh-host> \
  IRIS_DRIVE_E2E_IOS_HOST=<ssh-host> \
  IRIS_DRIVE_E2E_ANDROID_HOST=<ssh-host> \
    scripts/cross-vm-five-platform-e2e.sh [cross-vm-e2e args]

Runs the multidevice sync harness across five labeled peers:
  ubuntu   posix
  windows  windows
  macos    posix
  ios      posix daemon peer plus iOS simulator app smoke
  android  posix daemon peer plus Android adb app smoke

The Android host should be reachable by SSH, have the iris-drive checkout at
~/src/iris-drive, and have an online adb device or emulator selected by
IRIS_DRIVE_ANDROID_SERIAL or ANDROID_SERIAL on that host. The Android peer uses
provider commands in the sync harness; no mobile folder mount is created.
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
ANDROID_HOST="$(required_env IRIS_DRIVE_E2E_ANDROID_HOST)"

echo "[e2e-5devices] running iOS simulator smoke on $IOS_HOST" >&2
ssh "$IOS_HOST" 'cd "$HOME/src/iris-drive" && scripts/ios-simulator-smoke.sh'

echo "[e2e-5devices] running Android adb smoke on $ANDROID_HOST" >&2
ssh "$ANDROID_HOST" 'cd "$HOME/src/iris-drive" && scripts/mobile-android-smoke.sh'

if [[ -z "${IRIS_DRIVE_E2E_MOUNT_LABELS+x}" ]]; then
  export IRIS_DRIVE_E2E_MOUNT_LABELS="ubuntu macos"
fi

exec "$ROOT/scripts/cross-vm-e2e.sh" \
  --host "ubuntu=posix:$UBUNTU_HOST" \
  --host "windows=windows:$WINDOWS_HOST" \
  --host "macos=posix:$MACOS_HOST" \
  --host "ios=posix:$IOS_HOST" \
  --host "android=posix:$ANDROID_HOST" \
  "$@"
