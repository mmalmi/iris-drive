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
  ios      posix daemon peer plus iOS simulator app smoke and physical Iris Apps smoke
  android  posix daemon peer plus Android adb app smoke

The iOS and Android hosts may be SSH targets with the iris-drive checkout at
~/src/iris-drive, or the literal host "local" when the simulator/device is on
the current machine. The Android host must have an online adb device or
emulator selected by IRIS_DRIVE_ANDROID_SERIAL or ANDROID_SERIAL. The Android
peer uses provider commands in the sync harness; no mobile folder mount is
created. Physical QR/manual linking runs only when both phones are controlled by
one host (or IRIS_DRIVE_MOBILE_PHYSICAL_LINK_HOST names that host), and real
iOS app-group writes require IRIS_DRIVE_MOBILE_LINK_ISOLATED_DEVICE=1 on a
reserved test phone. Auto mode records an explicit skip otherwise.
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

run_host_repo_command() {
  local host="$1"
  shift
  if [[ "$host" == "local" ]]; then
    (cd "$ROOT" && "$@")
    return
  fi
  local quoted=()
  local arg
  for arg in "$@"; do
    quoted+=("$(printf "%q" "$arg")")
  done
  local status=0
  ssh "$host" "cd \"\$HOME/src/iris-drive\" && ${quoted[*]}" || status=$?
  if [[ "$status" -eq 255 ]]; then
    echo "infrastructure unavailable: SSH host $host became unreachable" >&2
    return 75
  fi
  return "$status"
}

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
source "$ROOT/scripts/lib/parallel-gate.sh"
UBUNTU_HOST="$(required_env IRIS_DRIVE_E2E_UBUNTU_HOST)"
WINDOWS_HOST="$(required_env IRIS_DRIVE_E2E_WINDOWS_HOST)"
MACOS_HOST="$(required_env IRIS_DRIVE_E2E_MACOS_HOST)"
IOS_HOST="$(required_env IRIS_DRIVE_E2E_IOS_HOST)"
ANDROID_HOST="$(required_env IRIS_DRIVE_E2E_ANDROID_HOST)"

if [[ -z "${IRIS_DRIVE_E2E_TIMEOUT_SECS+x}" ]]; then
  export IRIS_DRIVE_E2E_TIMEOUT_SECS=300
fi
# The release matrix must prove approval delivery rather than rewriting each
# linked profile with the owner's roster before its daemon starts.
export IRIS_DRIVE_E2E_SIDELOAD_APPKEYS="${IRIS_DRIVE_E2E_SIDELOAD_APPKEYS:-0}"
# Desktop request/approval automation is intentionally reserved for this full
# native matrix; ordinary cross-VM and release-gate runs keep their fast CLI
# setup path.
export IRIS_DRIVE_E2E_DESKTOP_GUI_LINKING="${IRIS_DRIVE_E2E_DESKTOP_GUI_LINKING:-1}"

run_linux_smoke() {
  echo "[e2e-5devices] running Linux GTK GUI smoke on $UBUNTU_HOST" >&2
  "$ROOT/scripts/desktop-gui-smoke.sh" linux "$UBUNTU_HOST"
}

run_windows_smoke() {
  echo "[e2e-5devices] running Windows WPF GUI smoke on $WINDOWS_HOST" >&2
  "$ROOT/scripts/desktop-gui-smoke.sh" windows "$WINDOWS_HOST"
}

run_apple_smokes() {
  if [[ "$IOS_HOST" != "local" \
    || "${IRIS_DRIVE_E2E_LOCAL_IOS_FUNCTIONAL_PRECHECKED:-0}" != "1" ]]; then
    echo "[e2e-5devices] running iOS simulator smoke on $IOS_HOST" >&2
    run_host_repo_command "$IOS_HOST" \
      env "IRIS_DRIVE_IOS_SIMULATOR_DEVICE=${IRIS_DRIVE_IOS_SIMULATOR_DEVICE:-}" \
      scripts/ios-simulator-smoke.sh

    echo "[e2e-5devices] running iOS GUI linking smoke on $IOS_HOST" >&2
    run_host_repo_command "$IOS_HOST" \
      env "IRIS_DRIVE_IOS_SIMULATOR_DEVICE=${IRIS_DRIVE_IOS_SIMULATOR_DEVICE:-}" \
      scripts/ios-gui-linking-smoke.sh
  else
    echo "[e2e-5devices] reusing local iOS functional smoke" >&2
  fi

  echo "[e2e-5devices] running iOS physical Iris Apps WebView smoke on $IOS_HOST" >&2
  run_host_repo_command "$IOS_HOST" \
    env "IRIS_DRIVE_IOS_DEVICE=${IRIS_DRIVE_IOS_DEVICE:-}" \
    scripts/ios-device-iris-apps-smoke.sh
}

run_android_smokes() {
  if [[ "$ANDROID_HOST" != "local" \
    || "${IRIS_DRIVE_E2E_LOCAL_ANDROID_FUNCTIONAL_PRECHECKED:-0}" != "1" ]]; then
    echo "[e2e-5devices] running Android GUI linking smoke on $ANDROID_HOST" >&2
    run_host_repo_command "$ANDROID_HOST" \
      env \
      "IRIS_DRIVE_ANDROID_SERIAL=${IRIS_DRIVE_ANDROID_SERIAL:-}" \
      "IRIS_DRIVE_ANDROID_USE_DIRECT_STATIC_PEER=${IRIS_DRIVE_ANDROID_USE_DIRECT_STATIC_PEER:-true}" \
      scripts/android-gui-linking-smoke.sh
  else
    echo "[e2e-5devices] reusing local Android functional smoke" >&2
  fi

  echo "[e2e-5devices] running Android adb provider smoke on $ANDROID_HOST" >&2
  run_host_repo_command "$ANDROID_HOST" \
    env "IRIS_DRIVE_ANDROID_SERIAL=${IRIS_DRIVE_ANDROID_SERIAL:-}" \
    scripts/mobile-android-smoke.sh --no-build
}

physical_mobile_link_mode="${IRIS_DRIVE_MOBILE_PHYSICAL_LINKING:-auto}"
physical_mobile_link_host="${IRIS_DRIVE_MOBILE_PHYSICAL_LINK_HOST:-}"

resolve_physical_mobile_link_host() {
  if [[ -z "$physical_mobile_link_host" && "$IOS_HOST" == "$ANDROID_HOST" ]]; then
    physical_mobile_link_host="$IOS_HOST"
  fi
  if [[ -n "$physical_mobile_link_host" ]]; then
    return
  fi
  case "$physical_mobile_link_mode" in
    0 | false | FALSE | False | no | NO | No | off | OFF | Off | auto | AUTO | Auto | "") ;;
    *)
      echo "physical mobile linking requires co-located device hosts or IRIS_DRIVE_MOBILE_PHYSICAL_LINK_HOST" >&2
      return 1
      ;;
  esac
}

run_physical_mobile_linking_smoke() {
  local reuse_android="${IRIS_DRIVE_MOBILE_REUSE_ANDROID_ARTIFACTS:-}"
  if [[ -z "$physical_mobile_link_host" ]]; then
    case "$physical_mobile_link_mode" in
      0 | false | FALSE | False | no | NO | No | off | OFF | Off)
        echo 'MOBILE_PHYSICAL_LINKING_SKIPPED mode=disabled reason=explicit evidence={"skipped":true,"reason":"explicit"}'
        return
        ;;
      auto | AUTO | Auto | "")
        echo 'MOBILE_PHYSICAL_LINKING_SKIPPED mode=auto reason=devices_not_colocated evidence={"skipped":true,"reason":"devices_not_colocated"}'
        return
        ;;
    esac
  fi
  if [[ -z "$reuse_android" ]]; then
    reuse_android=0
    if [[ "$physical_mobile_link_host" == "$ANDROID_HOST" \
      && "${IRIS_DRIVE_E2E_LOCAL_ANDROID_FUNCTIONAL_PRECHECKED:-0}" == "1" ]]; then
      reuse_android=1
    fi
  fi
  echo "[e2e-5devices] running physical iOS/Android QR/manual linking on $physical_mobile_link_host" >&2
  run_host_repo_command "$physical_mobile_link_host" \
    env \
    "IRIS_DRIVE_MOBILE_PHYSICAL_LINKING=${IRIS_DRIVE_MOBILE_PHYSICAL_LINKING:-auto}" \
    "IRIS_DRIVE_MOBILE_LINK_ISOLATED_DEVICE=${IRIS_DRIVE_MOBILE_LINK_ISOLATED_DEVICE:-0}" \
    "IRIS_DRIVE_MOBILE_REUSE_ANDROID_ARTIFACTS=$reuse_android" \
    "IRIS_DRIVE_IOS_DEVICE=${IRIS_DRIVE_IOS_DEVICE:-}" \
    "IRIS_DRIVE_ANDROID_SERIAL=${IRIS_DRIVE_ANDROID_SERIAL:-}" \
    scripts/mobile-ios-android-linking-e2e.sh
}

run_physical_macos_android_manual_linking_smoke() {
  local mode="${IRIS_DRIVE_MACOS_ANDROID_MANUAL_LINKING:-auto}"
  local mac_host="${IRIS_DRIVE_MACOS_SSH_HOST:-$MACOS_HOST}"
  local skip_sync=0
  if [[ "$mac_host" == local ]]; then
    case "$mode" in
      0|false|FALSE|False|no|NO|No|off|OFF|Off|auto|AUTO|Auto|"")
        echo 'MACOS_ANDROID_MANUAL_LINKING_SKIPPED mode=auto reason=macos_ssh_vm_unavailable evidence={"skipped":true,"reason":"macos_ssh_vm_unavailable"}'
        return
        ;;
      *)
        echo "physical macOS/Android manual linking requires an SSH macOS VM" >&2
        return 1
        ;;
    esac
  fi
  if [[ "${IRIS_DRIVE_MACOS_VM_FUNCTIONAL_PRECHECKED:-0}" == 1 ]]; then
    skip_sync=1
  fi
  echo "[e2e-5devices] running physical macOS/Android bidirectional manual linking" >&2
  run_host_repo_command "$ANDROID_HOST" \
    env \
    "IRIS_DRIVE_MACOS_ANDROID_MANUAL_LINKING=$mode" \
    "IRIS_DRIVE_MACOS_SSH_HOST=$mac_host" \
    "IRIS_DRIVE_MACOS_SKIP_GIT_SYNC=$skip_sync" \
    "IRIS_DRIVE_MOBILE_REUSE_ANDROID_ARTIFACTS=1" \
    "IRIS_DRIVE_ANDROID_SERIAL=${IRIS_DRIVE_ANDROID_SERIAL:-}" \
    scripts/macos-vm-android-manual-link-e2e.sh
}

resolve_physical_mobile_link_host

parallel_group_begin e2e-5devices
parallel_group_start linux run_linux_smoke
parallel_group_start windows run_windows_smoke
parallel_group_start apple run_apple_smokes
parallel_group_start android run_android_smokes
parallel_status=0
parallel_group_wait || parallel_status=$?
if [[ "$parallel_status" -ne 0 ]]; then
  exit "$parallel_status"
fi

run_physical_mobile_linking_smoke
run_physical_macos_android_manual_linking_smoke

if [[ -z "${IRIS_DRIVE_E2E_MOUNT_LABELS+x}" ]]; then
  export IRIS_DRIVE_E2E_MOUNT_LABELS="ubuntu"
fi

exec "$ROOT/scripts/cross-vm-e2e.sh" \
  --host "ubuntu=posix:$UBUNTU_HOST" \
  --host "windows=windows:$WINDOWS_HOST" \
  --host "macos=posix:$MACOS_HOST" \
  --host "ios=posix:$IOS_HOST" \
  --host "android=posix:$ANDROID_HOST" \
  "$@"
