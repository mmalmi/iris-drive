#!/usr/bin/env bash

set -Eeuo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"

sh_quote() {
  printf "'%s'" "$(printf "%s" "$1" | sed "s/'/'\\\\''/g")"
}

ps_quote() {
  printf "'%s'" "$(printf "%s" "$1" | sed "s/'/''/g")"
}

run_posix_reset() {
  local host="$1"
  shift
  if [[ "$host" == "local" ]]; then
    (cd "$ROOT" && IRIS_NATIVE_LAB_ALLOW_RESET=1 "$@")
    return
  fi
  local -a quoted=()
  local argument
  for argument in "$@"; do
    quoted+=("$(sh_quote "$argument")")
  done
  ssh "$host" "cd \"\$HOME/src/iris-drive\" && IRIS_NATIVE_LAB_ALLOW_RESET=1 ${quoted[*]}" || exit 75
}

reset_windows_cloudfiles() {
  local host="$1"
  local config_dir="$2"
  local sync_root="$3"
  [[ "$host" != "local" ]] || {
    echo "Windows Cloud Files reset requires a Windows SSH host" >&2
    exit 75
  }
  {
    printf '$ConfigDir = %s\n' "$(ps_quote "$config_dir")"
    printf '$SyncRoot = %s\n' "$(ps_quote "$sync_root")"
    cat <<'REMOTE_PS'
$ErrorActionPreference = 'Stop'
$env:IRIS_NATIVE_LAB_ALLOW_RESET = '1'
try {
  $ResetScript = Join-Path $HOME 'src\iris-drive\scripts\reset_windows_cloudfiles.ps1'
  & $ResetScript -ConfigDir $ConfigDir -SyncRoot $SyncRoot
  if ($LASTEXITCODE -ne 0) { exit $LASTEXITCODE }
} catch {
  [Console]::Error.WriteLine("Cloud Files reset infrastructure failure: $($_.Exception.Message)")
  exit 75
}
REMOTE_PS
  } | ssh "$host" 'cmd /d /s /c "powershell -NoProfile -NonInteractive -ExecutionPolicy Bypass -Command ""`$script = [Console]::In.ReadToEnd(); & ([scriptblock]::Create(`$script))"""' || exit 75
}

if [[ "${IRIS_DRIVE_E2E_IOS_HOST}" == "local" ]]; then
  ios_simulator="${IRIS_DRIVE_LAB_ALLOCATED_IOS_SIMULATOR:-}"
  ios_device="${IRIS_DRIVE_LAB_ALLOCATED_IOS_DEVICE:-}"
else
  ios_simulator="${IRIS_DRIVE_LAB_IOS_SIMULATOR:-}"
  ios_device="${IRIS_DRIVE_LAB_IOS_DEVICE:-}"
fi
if [[ "${IRIS_DRIVE_E2E_ANDROID_HOST}" == "local" ]]; then
  android_device="${IRIS_DRIVE_LAB_ALLOCATED_ANDROID:-}"
else
  android_device="${IRIS_DRIVE_LAB_ANDROID_SERIAL:-}"
fi
if [[ -z "$ios_simulator" || -z "$ios_device" || -z "$android_device" ]]; then
  echo "managed native allocation is incomplete; run scripts/verify.sh full" >&2
  exit 75
fi
export IRIS_DRIVE_IOS_SIMULATOR_DEVICE="$ios_simulator"
export IRIS_DRIVE_IOS_DEVICE="$ios_device"
export IRIS_DRIVE_ANDROID_SERIAL="$android_device"

if [[ "${IRIS_NATIVE_LAB_RESET:-0}" == "1" ]]; then
  export IRIS_NATIVE_LAB_ALLOW_RESET=1
  run_posix_reset "${IRIS_DRIVE_E2E_IOS_HOST}" \
    scripts/native_state_reset.sh ios-simulator --udid "$ios_simulator" --erase
  run_posix_reset "${IRIS_DRIVE_E2E_ANDROID_HOST}" \
    scripts/native_state_reset.sh android \
    --serial "$android_device" \
    --bundle-id "${IRIS_DRIVE_ANDROID_PACKAGE:-to.iris.drive}" \
    --test-bundle-id "${IRIS_DRIVE_ANDROID_TEST_PACKAGE:-to.iris.drive.test}"

  if [[ -z "${IRIS_DRIVE_LAB_FILEPROVIDER_DOMAIN_ID:-}" ]]; then
    echo "IRIS_DRIVE_LAB_FILEPROVIDER_DOMAIN_ID is required for managed reset" >&2
    exit 75
  fi
  reset_args=(
    scripts/native_state_reset.sh
      macos-fileprovider
    --domain-id "$IRIS_DRIVE_LAB_FILEPROVIDER_DOMAIN_ID"
    --display-name "${IRIS_DRIVE_LAB_FILEPROVIDER_DISPLAY_NAME:-Iris Drive Lab}"
  )
  if [[ -n "${IRIS_DRIVE_LAB_FILEPROVIDER_STATE_DIR:-}" ]]; then
    reset_args+=(--state-dir "$IRIS_DRIVE_LAB_FILEPROVIDER_STATE_DIR")
  fi
  run_posix_reset "${IRIS_DRIVE_E2E_MACOS_HOST}" "${reset_args[@]}"

  if [[ -z "${IRIS_DRIVE_LAB_WINDOWS_CONFIG_DIR:-}" || -z "${IRIS_DRIVE_LAB_WINDOWS_SYNC_ROOT:-}" ]]; then
    echo "Windows lab config and sync-root paths are required for managed reset" >&2
    exit 75
  fi
  reset_windows_cloudfiles \
    "${IRIS_DRIVE_E2E_WINDOWS_HOST}" \
    "$IRIS_DRIVE_LAB_WINDOWS_CONFIG_DIR" \
    "$IRIS_DRIVE_LAB_WINDOWS_SYNC_ROOT"
fi

exec "$ROOT/scripts/release-gate.sh" --full
