#!/usr/bin/env bash

set -Eeuo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"

load_env_file_defaults() {
  local env_file="$1"
  local line
  local key
  local value

  [[ -f "$env_file" ]] || return 0

  while IFS= read -r line || [[ -n "$line" ]]; do
    line="${line#"${line%%[![:space:]]*}"}"
    line="${line%"${line##*[![:space:]]}"}"
    [[ -z "$line" || "$line" == \#* || "$line" != *=* ]] && continue

    key="${line%%=*}"
    key="${key%"${key##*[![:space:]]}"}"
    key="${key#"${key%%[![:space:]]*}"}"
    [[ "$key" =~ ^[A-Za-z_][A-Za-z0-9_]*$ ]] || continue
    [[ -n "${!key:-}" ]] && continue

    value="${line#*=}"
    value="${value#"${value%%[![:space:]]*}"}"
    value="${value%"${value##*[![:space:]]}"}"
    if [[ "$value" == \"*\" && "$value" == *\" ]]; then
      value="${value:1:${#value}-2}"
    elif [[ "$value" == \'*\' && "$value" == *\' ]]; then
      value="${value:1:${#value}-2}"
    fi
    export "$key=$value"
  done < "$env_file"
}

load_env_file_defaults "$ROOT/.env.local"

export CARGO_TARGET_DIR="${CARGO_TARGET_DIR:-$HOME/.cache/cargo-target}"

PROJECT="$ROOT/macos/IrisDriveMac.xcodeproj"
SCHEME="IrisDriveMac"
CONFIGURATION="${IRIS_DRIVE_MACOS_XCODE_CONFIGURATION:-Debug}"
BUILD_DIR="${IRIS_DRIVE_MACOS_BUILD_DIR:-$ROOT/macos/.build}"
DERIVED_DATA="$BUILD_DIR/DerivedData"
BUILD_LOG="${IRIS_DRIVE_MACOS_BUILD_LOG:-/tmp/iris-drive-macos-build.log}"
HOST_ARCH="$(uname -m)"
APP_PROCESS_NAME="Iris Drive"

usage() {
  cat <<'EOF'
usage: scripts/macos-dev-app.sh build|run

Environment:
  .env.local
      Local defaults are auto-loaded when present. Shell environment variables
      take precedence over .env.local values.
  IRIS_DRIVE_MACOS_SIGNING=auto|none|development
      auto/default launches without restricted entitlements unless a development
      team is supplied. development requires Xcode account/profiles.
  IRIS_DRIVE_DEVELOPMENT_TEAM=<team id>
      Team id used for development signing.
  IRIS_DRIVE_ASC_AUTH_KEY_PATH / IRIS_DRIVE_ASC_AUTH_KEY_ID /
  IRIS_DRIVE_ASC_AUTH_KEY_ISSUER_ID
      Optional App Store Connect API key for provisioning updates when Xcode
      has no signed-in account.
EOF
}

log() {
  printf '%s\n' "$*" >&2
}

development_team() {
  printf '%s' "${IRIS_DRIVE_DEVELOPMENT_TEAM:-}"
}

require_env_var() {
  local name="$1"
  if [[ -z "${!name:-}" ]]; then
    echo "$name is required." >&2
    exit 2
  fi
}

signing_mode() {
  local mode="${IRIS_DRIVE_MACOS_SIGNING:-auto}"
  case "$mode" in
    auto)
      if [[ -n "$(development_team)" ]]; then
        printf 'development\n'
      else
        printf 'none\n'
      fi
      ;;
    none|development)
      printf '%s\n' "$mode"
      ;;
    *)
      echo "IRIS_DRIVE_MACOS_SIGNING must be auto, none, or development." >&2
      exit 2
      ;;
  esac
}

resolve_target_dir() {
  cargo metadata --no-deps --format-version 1 \
    | python3 -c 'import json, sys; print(json.load(sys.stdin)["target_directory"])'
}

resolve_app_path() {
  local settings
  local built_products_dir
  local full_product_name

  settings="$(
    xcodebuild \
      -project "$PROJECT" \
      -scheme "$SCHEME" \
      -configuration "$CONFIGURATION" \
      -derivedDataPath "$DERIVED_DATA" \
      -destination "platform=macOS,arch=$HOST_ARCH" \
      -showBuildSettings 2>/dev/null
  )"
  built_products_dir="$(awk -F' = ' '/^[[:space:]]*BUILT_PRODUCTS_DIR = / { print $2; exit }' <<<"$settings")"
  full_product_name="$(awk -F' = ' '/^[[:space:]]*FULL_PRODUCT_NAME = / { print $2; exit }' <<<"$settings")"

  if [[ -n "${built_products_dir:-}" && -n "${full_product_name:-}" ]]; then
    printf '%s/%s\n' "$built_products_dir" "$full_product_name"
    return 0
  fi

  find "$DERIVED_DATA/Build/Products" -maxdepth 3 -name 'Iris Drive.app' -type d -print -quit 2>/dev/null || true
}

build_xcode_app() {
  local mode="$1"
  local auth_args=()
  local args=(
    -project "$PROJECT"
    -scheme "$SCHEME"
    -configuration "$CONFIGURATION"
    -derivedDataPath "$DERIVED_DATA"
    -destination "platform=macOS,arch=$HOST_ARCH"
  )

  if [[ -n "${IRIS_DRIVE_ASC_AUTH_KEY_PATH:-}" \
    || -n "${IRIS_DRIVE_ASC_AUTH_KEY_ID:-}" \
    || -n "${IRIS_DRIVE_ASC_AUTH_KEY_ISSUER_ID:-}" ]]; then
    require_env_var IRIS_DRIVE_ASC_AUTH_KEY_PATH
    require_env_var IRIS_DRIVE_ASC_AUTH_KEY_ID
    require_env_var IRIS_DRIVE_ASC_AUTH_KEY_ISSUER_ID
    auth_args=(
      -authenticationKeyPath "$IRIS_DRIVE_ASC_AUTH_KEY_PATH"
      -authenticationKeyID "$IRIS_DRIVE_ASC_AUTH_KEY_ID"
      -authenticationKeyIssuerID "$IRIS_DRIVE_ASC_AUTH_KEY_ISSUER_ID"
    )
  fi

  if [[ "$mode" == "development" ]]; then
    local team
    team="$(development_team)"
    if [[ -z "$team" ]]; then
      echo "IRIS_DRIVE_MACOS_SIGNING=development requires IRIS_DRIVE_DEVELOPMENT_TEAM." >&2
      exit 2
    fi
    args+=(DEVELOPMENT_TEAM="$team")
    if [[ "${IRIS_DRIVE_ALLOW_PROVISIONING_UPDATES:-1}" != "0" ]]; then
      args+=("${auth_args[@]}" -allowProvisioningUpdates)
    fi
  else
    args+=(CODE_SIGNING_ALLOWED=NO)
  fi

  log "Building macOS app ($mode signing); log: $BUILD_LOG"
  xcodebuild "${args[@]}" build >"$BUILD_LOG"
}

signing_identity_for_app() {
  local app_path="$1"
  local identity="${IRIS_DRIVE_CODESIGN_IDENTITY:-}"

  if [[ -z "$identity" ]]; then
    identity="$(
      codesign -dv --verbose=4 "$app_path" 2>&1 \
        | awk -F= '/^Authority=Apple Development:/ { print $2; exit }'
    )"
  fi

  printf '%s\n' "${identity:-Apple Development}"
}

sign_helper() {
  local helper="$1"
  local mode="$2"
  local identity="${3:-}"

  if [[ "$mode" == "development" ]]; then
    codesign --force --sign "$identity" --entitlements "$ROOT/macos/IrisDriveMac.entitlements" "$helper" >&2
  else
    codesign --force --sign - "$helper" >&2
  fi
}

finalize_app_signature() {
  local app_path="$1"
  local mode="$2"
  local identity="${3:-}"

  if [[ "$mode" == "development" ]]; then
    codesign --force --sign "$identity" --entitlements "$ROOT/macos/IrisDriveMac.entitlements" "$app_path" >&2
    codesign --verify --strict --deep "$app_path" >&2
  fi
}

terminate_running_app() {
  if pgrep -x "$APP_PROCESS_NAME" >/dev/null 2>&1; then
    pkill -TERM -x "$APP_PROCESS_NAME" >/dev/null 2>&1 || true
    for _ in {1..40}; do
      if ! pgrep -x "$APP_PROCESS_NAME" >/dev/null 2>&1; then
        return 0
      fi
      sleep 0.1
    done
    pkill -x "$APP_PROCESS_NAME" >/dev/null 2>&1 || true
  fi
}

build_app() {
  local mode
  local target_dir
  local app_path
  local signing_identity=""

  mode="$(signing_mode)"
  log "Generating macOS project"
  (cd "$ROOT/macos" && xcodegen generate) >&2

  log "Building idrive helper"
  cargo build -p idrive >&2
  target_dir="$(resolve_target_dir)"

  build_xcode_app "$mode"
  app_path="$(resolve_app_path)"
  if [[ -z "${app_path:-}" || ! -d "$app_path" ]]; then
    echo "Built macOS app not found. Build log: $BUILD_LOG" >&2
    exit 1
  fi

  if [[ "$mode" == "development" ]]; then
    signing_identity="$(signing_identity_for_app "$app_path")"
  fi

  cp "$target_dir/debug/idrive" "$app_path/Contents/MacOS/idrive"
  chmod +x "$app_path/Contents/MacOS/idrive"
  sign_helper "$app_path/Contents/MacOS/idrive" "$mode" "$signing_identity"
  finalize_app_signature "$app_path" "$mode" "$signing_identity"

  touch "$app_path"
  /System/Library/Frameworks/CoreServices.framework/Frameworks/LaunchServices.framework/Support/lsregister \
    -f -R -trusted "$app_path" >/dev/null 2>&1 || true

  printf '%s\n' "$app_path"
}

run_app() {
  local app_path
  local app_base_dir="${IRIS_DRIVE_APP_BASE_DIR:-}"
  local mode

  mode="$(signing_mode)"
  app_path="$(build_app)"
  if [[ -z "$app_base_dir" && "$mode" != "development" ]]; then
    app_base_dir="$BUILD_DIR/AppData"
  fi

  terminate_running_app
  if [[ -n "$app_base_dir" ]]; then
    mkdir -p "$app_base_dir"
    open --env "IRIS_DRIVE_APP_BASE_DIR=$app_base_dir" "$app_path"
    echo "macOS app data: $app_base_dir"
  else
    open "$app_path"
  fi
  echo "macOS app launched: $app_path"
}

case "${1:-}" in
  build)
    build_app
    ;;
  run)
    run_app
    ;;
  -h|--help|help)
    usage
    ;;
  *)
    usage >&2
    exit 2
    ;;
esac
