#!/usr/bin/env bash

set -Eeuo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$ROOT"

# Avoid the retired machine-wide target even when a long-lived shell still
# exports it. An explicit non-legacy target remains supported.
if [[ "${CARGO_TARGET_DIR:-}" == "$HOME/.cache/cargo-target" ]]; then
  unset CARGO_TARGET_DIR
fi
if command -v sccache >/dev/null 2>&1; then
  export SCCACHE_BASEDIRS="${SCCACHE_BASEDIRS:-$ROOT}"
fi

usage() {
  cat <<'EOF'
usage: scripts/verify.sh fast|full|health

fast   Per-change Rust/core/contract checks. No simulator, phone, VM, or GUI.
full   Fast checks plus the reserved five-platform GUI/physical-device matrix.
health Preflight the configured full-matrix resources without running tests.

Full verification requires IRIS_DRIVE_E2E_UBUNTU_HOST,
IRIS_DRIVE_E2E_WINDOWS_HOST, IRIS_DRIVE_E2E_MACOS_HOST,
IRIS_DRIVE_E2E_IOS_HOST, and IRIS_DRIVE_E2E_ANDROID_HOST. Use "local" for
mobile resources attached to this Mac. Set IRIS_NATIVE_LAB_RESET=1 only with
dedicated lab simulators/devices; destructive resets are otherwise disabled.
EOF
}

run_fast() {
  python3 scripts/test_native_lab.py
  cargo fmt --all --check
  cargo clippy --workspace --all-targets -- -D warnings
  just structure
  node --test scripts/local-release*.test.mjs
  cargo test --workspace --exclude idrive
  cargo test -p idrive --bin idrive --test link_input_e2e
}

required_lab_env=(
  IRIS_DRIVE_E2E_UBUNTU_HOST
  IRIS_DRIVE_E2E_WINDOWS_HOST
  IRIS_DRIVE_E2E_MACOS_HOST
  IRIS_DRIVE_E2E_IOS_HOST
  IRIS_DRIVE_E2E_ANDROID_HOST
)

build_health_args() {
  HEALTH_ARGS=(--health command:ssh --health command:python3)
  ALLOCATION_ARGS=()
  local variable value
  for variable in "${required_lab_env[@]}"; do
    HEALTH_ARGS+=(--health "env:${variable}")
    value="${!variable:-}"
    if [[ -n "$value" && "$value" != "local" ]]; then
      HEALTH_ARGS+=(--health "ssh:${value}")
    fi
  done
  if [[ "${IRIS_DRIVE_E2E_IOS_HOST:-}" == "local" ]]; then
    HEALTH_ARGS+=(
      --health "ios-simulator:${IRIS_DRIVE_LAB_IOS_SIMULATOR:-auto}"
      --health "ios-device:${IRIS_DRIVE_LAB_IOS_DEVICE:-auto}"
    )
    ALLOCATION_ARGS+=(
      --allocation-env ios-simulator=IRIS_DRIVE_LAB_ALLOCATED_IOS_SIMULATOR
      --allocation-env ios-device=IRIS_DRIVE_LAB_ALLOCATED_IOS_DEVICE
    )
  elif [[ -n "${IRIS_DRIVE_E2E_IOS_HOST:-}" ]]; then
    HEALTH_ARGS+=(
      --health env:IRIS_DRIVE_LAB_IOS_SIMULATOR
      --health env:IRIS_DRIVE_LAB_IOS_DEVICE
    )
  fi
  if [[ "${IRIS_DRIVE_E2E_ANDROID_HOST:-}" == "local" ]]; then
    HEALTH_ARGS+=(--health "android:${IRIS_DRIVE_LAB_ANDROID_SERIAL:-auto}")
    ALLOCATION_ARGS+=(--allocation-env android=IRIS_DRIVE_LAB_ALLOCATED_ANDROID)
  elif [[ -n "${IRIS_DRIVE_E2E_ANDROID_HOST:-}" ]]; then
    HEALTH_ARGS+=(--health env:IRIS_DRIVE_LAB_ANDROID_SERIAL)
  fi
  if [[ "$(uname -s)" == "Darwin" ]]; then
    HEALTH_ARGS+=(--health local:macos)
  fi
}

run_managed_full() {
  build_health_args
  result="${IRIS_DRIVE_VERIFY_RESULT:-$ROOT/artifacts/verification/full-native-result.json}"
  python3 scripts/native_lab.py run \
    --resource "iris-drive-five-platform-native-matrix" \
    --result "$result" \
    "${HEALTH_ARGS[@]}" \
    "${ALLOCATION_ARGS[@]}" \
    -- scripts/verify_full_native.sh
}

case "${1:-}" in
  fast)
    run_fast
    ;;
  full)
    if [[ "${IRIS_VERIFY_SKIP_FAST:-0}" != "1" ]]; then
      run_fast
    fi
    run_managed_full
    ;;
  health)
    build_health_args
    python3 scripts/native_lab.py health "${HEALTH_ARGS[@]}"
    ;;
  *)
    usage >&2
    exit 2
    ;;
esac
