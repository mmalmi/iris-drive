#!/usr/bin/env bash

set -Eeuo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$ROOT"
DEV_LAB_ENV="${IRIS_DRIVE_DEV_LAB_ENV:-$HOME/.config/iris-drive/dev-lab.env}"
if [[ -f "$DEV_LAB_ENV" ]]; then
  set -a
  # shellcheck disable=SC1090
  source "$DEV_LAB_ENV"
  set +a
fi

usage() {
  cat <<'USAGE'
Usage: scripts/release-gate.sh [--full]

Runs the local release confidence gate. The default gate is deterministic and
host-local. --full additionally runs the five-platform lab e2e, which requires
the configured Linux, Windows, macOS, iOS, and Android hosts/devices.

Environment:
  IRIS_DRIVE_RELEASE_GATE_FULL=1       Same as --full.
  IRIS_DRIVE_RELEASE_GATE_FAST_PRECHECKED=1
                                        Reuse checks passed by verify-fast in
                                        this same invocation.
  IRIS_DRIVE_RELEASE_GATE_ANDROID=0    Skip local Android build/smoke.
  IRIS_DRIVE_RELEASE_GATE_IOS=0        Skip local iOS build/smoke.
  IRIS_DRIVE_RELEASE_GATE_MACOS=0      Skip the macOS build/smoke lane.
  IRIS_DRIVE_MACOS_SSH_HOST=<ssh-host> Route every macOS UI/idle lane to this VM.
  IRIS_DRIVE_RELEASE_GATE_MACOS_VM=0   Explicitly run macOS UI/idle locally;
                                        otherwise a configured VM is preferred.
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

source "$ROOT/scripts/lib/parallel-gate.sh"

run() {
  printf '[release-gate] %s\n' "$*" >&2
  "$@"
}

run_parallel_checks() {
  parallel_group_begin release-gate
  parallel_group_start release-workflow-tests python3 scripts/test_release_workflows.py
  parallel_group_start local-release-tests node --test \
    scripts/local-release.test.mjs scripts/local-release-version.test.mjs \
    scripts/local-release-windows-policy.test.mjs scripts/release-build-hygiene.test.mjs
  parallel_group_start fmt cargo fmt --check
  parallel_group_start structure just structure
  parallel_group_start workspace-tests cargo test --workspace --exclude idrive
  parallel_group_start idrive-tests run_rust_tests
  parallel_group_wait
}

run_parallel_functions() {
  local group="$1"
  shift
  local label task

  parallel_group_begin "release-gate-$group"
  while [[ $# -gt 0 ]]; do
    label="$1"
    task="$2"
    shift 2
    parallel_group_start "$label" "$task"
  done
  parallel_group_wait
}

run_rust_tests() {
  local compile_args=(--bin idrive --test cli_e2e --test daemon_sync_matrix)
  local cli_args=(--bin idrive --test cli_e2e)
  if [[ "${IRIS_DRIVE_RELEASE_GATE_FAST_PRECHECKED:-0}" != "1" ]]; then
    compile_args+=(--test link_input_e2e)
    cli_args+=(--test link_input_e2e)
  fi

  run cargo test -p idrive "${compile_args[@]}" --no-run
  parallel_group_begin release-gate-rust
  parallel_group_start cli-tests \
    cargo test -p idrive "${cli_args[@]}" -- --test-threads=1
  parallel_group_start daemon-tests \
    cargo test -p idrive --test daemon_sync_matrix -- --test-threads=1
  parallel_group_wait
}

macos_gate_enabled() {
  ! bool_true "${IRIS_DRIVE_RELEASE_GATE_MACOS_SKIP:-0}" \
    && [[ "${IRIS_DRIVE_RELEASE_GATE_MACOS:-1}" != "0" ]]
}

macos_vm_gate_enabled() {
  case "${IRIS_DRIVE_RELEASE_GATE_MACOS_VM:-auto}" in
    0 | false | FALSE | False | no | NO | No | off | OFF | Off) return 1 ;;
    1 | true | TRUE | True | yes | YES | Yes | on | ON | On) return 0 ;;
    auto | AUTO | Auto | "") ;;
    *)
      echo "unsupported IRIS_DRIVE_RELEASE_GATE_MACOS_VM=${IRIS_DRIVE_RELEASE_GATE_MACOS_VM}" >&2
      exit 2
      ;;
  esac
  [[ -n "${IRIS_DRIVE_MACOS_SSH_HOST:-}" ]] \
    || git remote get-url "${IRIS_DRIVE_DEV_VM_MACOS_REMOTE:-macos}" >/dev/null 2>&1
}

ios_gate_enabled() {
  ! bool_true "${IRIS_DRIVE_RELEASE_GATE_IOS_SKIP:-0}" \
    && [[ "${IRIS_DRIVE_RELEASE_GATE_IOS:-1}" != "0" ]]
}

android_gate_enabled() {
  ! bool_true "${IRIS_DRIVE_RELEASE_GATE_ANDROID_SKIP:-0}" \
    && [[ "${IRIS_DRIVE_RELEASE_GATE_ANDROID:-1}" != "0" ]]
}

run_macos_functional_gate() {
  macos_gate_enabled || return 0
  local link_journey_default=0
  bool_true "$full" && link_journey_default=1
  if macos_vm_gate_enabled; then
    run env \
      IRIS_DRIVE_MACOS_SMOKE_LINK_JOURNEY="${IRIS_DRIVE_MACOS_SMOKE_LINK_JOURNEY:-$link_journey_default}" \
      "${IRIS_DRIVE_RELEASE_GATE_MACOS_SMOKE_COMMAND:-./scripts/macos-vm-smoke.sh}"
    return
  fi
  run env \
    CARGO_TARGET_DIR="${CARGO_TARGET_DIR:-$ROOT/target}" \
    IRIS_DRIVE_MACOS_SIGNING="${IRIS_DRIVE_MACOS_SIGNING:-none}" \
    IRIS_DRIVE_DISABLE_DAEMON_SERVICE="${IRIS_DRIVE_RELEASE_GATE_MACOS_DAEMON_SERVICE:-true}" \
    IRIS_DRIVE_MACOS_SMOKE_LINK_JOURNEY="${IRIS_DRIVE_MACOS_SMOKE_LINK_JOURNEY:-$link_journey_default}" \
    just smoke-macos
}

run_ios_functional_gate() {
  ios_gate_enabled || return 0
  # The GUI build-for-testing produces the simulator app used by both suites.
  run just ios-gui-smoke
  run just ios-smoke --no-build
}

run_apple_functional_gates() {
  run_macos_functional_gate
  run_ios_functional_gate
}

run_android_functional_gate() {
  android_gate_enabled || return 0
  run env IRIS_DRIVE_ANDROID_KEEP_TEST_APP=true just android-gui-smoke
}

run_ios_idle_cpu_gate() {
  ios_gate_enabled || return 0
  run env \
    IRIS_DRIVE_IDLE_CPU_REQUIRED_ROLES="${IRIS_DRIVE_RELEASE_GATE_IOS_IDLE_CPU_ROLES:-app}" \
    IRIS_DRIVE_IDLE_CPU_IOS_DEVICE="${IRIS_DRIVE_IOS_SIMULATOR_DEVICE:-${IRIS_DRIVE_IOS_DEVICE:-}}" \
    ./scripts/idle-cpu-gate.sh --platform ios
}

run_apple_idle_cpu_gates() {
  if ios_gate_enabled; then
    run_ios_idle_cpu_gate
  fi
  if macos_gate_enabled; then
    # This gate terminates simulator processes to isolate the macOS sample, so
    # it must remain after the iOS idle sample.
    run run_macos_idle_cpu_gate
  fi
}

run_android_idle_cpu_gate() {
  android_gate_enabled || return 0
  run env \
    IRIS_DRIVE_IDLE_CPU_REQUIRED_ROLES="${IRIS_DRIVE_RELEASE_GATE_ANDROID_IDLE_CPU_ROLES:-app}" \
    IRIS_DRIVE_IDLE_CPU_WARMUP_SECS="${IRIS_DRIVE_RELEASE_GATE_ANDROID_IDLE_CPU_WARMUP_SECS:-90}" \
    IRIS_DRIVE_IDLE_CPU_ANDROID_PACKAGE="${IRIS_DRIVE_ANDROID_PACKAGE:-to.iris.drive.uitest}" \
    ./scripts/idle-cpu-gate.sh --platform android
}

terminate_booted_ios_simulator_instances() {
  [[ "$(uname -s)" == "Darwin" ]] || return 0
  command -v xcrun >/dev/null 2>&1 || return 0

  local app_bundle_id="${IRIS_DRIVE_IOS_BUNDLE_ID:-fi.siriusbusiness.drive}"
  local provider_bundle_id="${IRIS_DRIVE_IOS_FILE_PROVIDER_BUNDLE_ID:-fi.siriusbusiness.drive.FileProvider}"
  local devices
  if ! devices="$(
    xcrun simctl list devices booted --json 2>/dev/null |
      python3 -c 'import json,sys; data=json.load(sys.stdin); print("\n".join(d["udid"] for runtime, devices in data.get("devices", {}).items() if "iOS" in runtime for d in devices if d.get("state") == "Booted"))'
  )"; then
    return 0
  fi

  local device
  while IFS= read -r device; do
    [[ -n "$device" ]] || continue
    xcrun simctl terminate "$device" "$app_bundle_id" >/dev/null 2>&1 || true
    xcrun simctl terminate "$device" "$provider_bundle_id" >/dev/null 2>&1 || true
  done <<<"$devices"
}

run_macos_idle_cpu_gate() {
  macos_gate_enabled || return 0
  if macos_vm_gate_enabled; then
    run "${IRIS_DRIVE_RELEASE_GATE_MACOS_IDLE_COMMAND:-./scripts/macos-vm-idle-cpu.sh}"
    return
  fi

  # A prior iOS gate can leave its simulator process owning FIPS's shared
  # loopback rendezvous. That unrelated process must not drive the isolated
  # macOS idle sample into peer-routing work.
  terminate_booted_ios_simulator_instances
  run ./scripts/macos-idle-cpu-smoke.sh
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

if [[ "${IRIS_DRIVE_RELEASE_GATE_FAST_PRECHECKED:-0}" == "1" ]]; then
  printf '[release-gate] reusing prechecks passed by verify-fast\n' >&2
  run_rust_tests
else
  run_parallel_checks
fi
run cargo build --workspace --release

case "$(uname -s)" in
  Darwin)
    if macos_vm_gate_enabled; then
      run_parallel_functions native-functional \
        macos-vm run_macos_functional_gate \
        ios run_ios_functional_gate \
        android run_android_functional_gate
    else
      run_parallel_functions native-functional \
        apple run_apple_functional_gates \
        android run_android_functional_gate
    fi
    if ios_gate_enabled; then
      export IRIS_DRIVE_E2E_LOCAL_IOS_FUNCTIONAL_PRECHECKED=1
    fi
    if android_gate_enabled; then
      export IRIS_DRIVE_E2E_LOCAL_ANDROID_FUNCTIONAL_PRECHECKED=1
    fi
    if macos_gate_enabled && macos_vm_gate_enabled; then
      export IRIS_DRIVE_MACOS_VM_FUNCTIONAL_PRECHECKED=1
    fi
    if idle_cpu_gate_enabled; then
      # A VM-routed macOS sample is isolated from the local iOS simulator and
      # can overlap both mobile samples. Keep local Apple sampling serial.
      if macos_vm_gate_enabled; then
        run_parallel_functions native-idle-cpu \
          macos-vm-idle-cpu run_macos_idle_cpu_gate \
          ios-idle-cpu run_ios_idle_cpu_gate \
          android-idle-cpu run_android_idle_cpu_gate
      else
        run_parallel_functions native-idle-cpu \
          apple-idle-cpu run_apple_idle_cpu_gates \
          android-idle-cpu run_android_idle_cpu_gate
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
