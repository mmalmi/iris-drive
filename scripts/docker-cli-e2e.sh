#!/usr/bin/env bash

set -Eeuo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
SRC_ROOT="$(cd "$ROOT/.." && pwd)"

for required in \
  "$SRC_ROOT/hashtree/rust/crates/hashtree-fips-transport" \
  "$SRC_ROOT/fips/crates/fips-core"
do
  if [[ ! -d "$required" ]]; then
    echo "Missing required sibling checkout: $required" >&2
    exit 1
  fi
done

IMAGE="${IRIS_DRIVE_DOCKER_IMAGE:-rust:1.95-bookworm}"
TARGET_VOLUME="${IRIS_DRIVE_DOCKER_TARGET_VOLUME:-iris-drive-cargo-target}"
REGISTRY_VOLUME="${IRIS_DRIVE_DOCKER_REGISTRY_VOLUME:-iris-drive-cargo-registry}"
GIT_VOLUME="${IRIS_DRIVE_DOCKER_GIT_VOLUME:-iris-drive-cargo-git}"

exec docker run --rm --init \
  -v "$SRC_ROOT:/work:ro" \
  -v "$TARGET_VOLUME:/cargo-target" \
  -v "$REGISTRY_VOLUME:/usr/local/cargo/registry" \
  -v "$GIT_VOLUME:/usr/local/cargo/git" \
  -e CARGO_BUILD_JOBS="${CARGO_BUILD_JOBS:-1}" \
  -e CARGO_INCREMENTAL=0 \
  -e CARGO_PROFILE_DEV_DEBUG=0 \
  -e CARGO_PROFILE_TEST_DEBUG=0 \
  -e CARGO_TARGET_DIR=/cargo-target \
  -w /work/iris-drive \
  "$IMAGE" \
  bash -lc '
    set -Eeuo pipefail
    export PATH="/usr/local/cargo/bin:${PATH}"
    export DEBIAN_FRONTEND=noninteractive
    apt-get update
    apt-get install -y --no-install-recommends \
      ca-certificates \
      clang \
      libdbus-1-dev \
      libclang-dev \
      pkg-config
    rm -rf /var/lib/apt/lists/*
    cargo test -p idrive --test cli_e2e linked_devices_sync_each_others_files_through_cli -- --nocapture
  '
