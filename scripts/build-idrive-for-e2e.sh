#!/usr/bin/env bash
set -Eeuo pipefail

profile="${1:-debug}"
override_idrive="${2:-}"
rebuild_idrive="${3:-1}"
repo="${IRIS_DRIVE_E2E_REPO:-$HOME/src/iris-drive}"
cargo_profile_arg=""
if [[ "$profile" == "release" ]]; then
  cargo_profile_arg="--release"
fi

supports_app_keys() {
  [[ -x "$1" ]] && "$1" app-keys --help >/dev/null 2>&1
}

idrive="$override_idrive"
if [[ -z "$idrive" && "$rebuild_idrive" != "0" && -f "$repo/Cargo.toml" ]] && command -v cargo >/dev/null 2>&1; then
  (cd "$repo" && cargo build -q $cargo_profile_arg -p idrive --bin idrive)
  idrive="$repo/target/$profile/idrive"
  [[ -x "$idrive" ]] || idrive="$HOME/.cache/cargo-target/$profile/idrive"
  [[ -x "$idrive" ]] || idrive="${CARGO_TARGET_DIR:+$CARGO_TARGET_DIR/$profile/idrive}"
fi

if [[ -z "$idrive" ]]; then
  for candidate in \
    "$repo/target/$profile/idrive" \
    "$HOME/.cache/cargo-target/$profile/idrive" \
    "${CARGO_TARGET_DIR:+$CARGO_TARGET_DIR/$profile/idrive}"
  do
    if supports_app_keys "$candidate"; then
      idrive="$candidate"
      break
    fi
  done
fi

if [[ -z "$idrive" && "$profile" == "debug" ]]; then
  for candidate in "$HOME/.cargo/bin/idrive" "$(command -v idrive || true)"; do
    if supports_app_keys "$candidate"; then
      idrive="$candidate"
      break
    fi
  done
fi

if ! supports_app_keys "$idrive" && [[ -f "$repo/Cargo.toml" ]] && command -v cargo >/dev/null 2>&1; then
  (cd "$repo" && cargo build -q $cargo_profile_arg -p idrive --bin idrive)
  idrive="$repo/target/$profile/idrive"
  [[ -x "$idrive" ]] || idrive="$HOME/.cache/cargo-target/$profile/idrive"
  [[ -x "$idrive" ]] || idrive="${CARGO_TARGET_DIR:+$CARGO_TARGET_DIR/$profile/idrive}"
fi

if ! supports_app_keys "$idrive"; then
  idrive="${CARGO_TARGET_DIR:+$CARGO_TARGET_DIR/$profile/idrive}"
  idrive="${idrive:-$repo/target/$profile/idrive}"
fi
supports_app_keys "$idrive" || idrive="$HOME/.cache/cargo-target/$profile/idrive"
if [[ "$profile" == "debug" ]]; then
  supports_app_keys "$idrive" || idrive="$HOME/.cargo/bin/idrive"
  supports_app_keys "$idrive" || idrive="$(command -v idrive || true)"
fi

if ! supports_app_keys "$idrive"; then
  echo "current idrive with app-keys support not found" >&2
  exit 1
fi

printf "%s\n" "$idrive"
