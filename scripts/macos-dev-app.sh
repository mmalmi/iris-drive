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
INSTALL_APP_PATH="${IRIS_DRIVE_MACOS_INSTALL_APP:-$HOME/Applications/Iris Drive.app}"
HOST_ARCH="$(uname -m)"
APP_PROCESS_NAME="Iris Drive"
BUILD_APP_PATH=""

usage() {
  cat <<'EOF'
usage: scripts/macos-dev-app.sh build|run

Environment:
  .env.local
      Local defaults are auto-loaded when present. Shell environment variables
      take precedence over .env.local values.
  IRIS_DRIVE_MACOS_SIGNING=auto|none|development
      auto/default uses a local Apple Development identity when available,
      otherwise ad-hoc. development signs with a real local certificate.
  IRIS_DRIVE_MACOS_XCODE_MANAGED_SIGNING=0
      Use the local certificate signing path for development signing instead
      of Xcode-managed provisioning. Xcode-managed signing is the default so
      File Provider testing-mode installs launch and stay enabled.
  IRIS_DRIVE_MACOS_KEEP_FILEPROVIDER_TESTING_MODE=0
      Strip the File Provider testing entitlement from provisioned dev builds.
      The default keeps it so local installs are always enabled without a
      separate Finder approval click.
  IRIS_DRIVE_DEVELOPMENT_TEAM=<team id>
      Optional team id used to select a development signing identity.
  IRIS_DRIVE_ASC_AUTH_KEY_PATH / IRIS_DRIVE_ASC_AUTH_KEY_ID /
  IRIS_DRIVE_ASC_AUTH_KEY_ISSUER_ID
      Optional App Store Connect API key for provisioning updates when Xcode
      has no signed-in account.
  IRIS_DRIVE_MACOS_INSTALL_APP
      Stable app bundle to install and launch. Defaults to
      ~/Applications/Iris Drive.app.
EOF
}

log() {
  printf '%s\n' "$*" >&2
}

development_team() {
  printf '%s' "${IRIS_DRIVE_DEVELOPMENT_TEAM:-}"
}

environment_flag() {
  case "${1:-}" in
    1|true|TRUE|True|yes|YES|Yes|on|ON|On) return 0 ;;
    *) return 1 ;;
  esac
}

codesigning_identities() {
  security find-identity -v -p codesigning 2>/dev/null |
    sed -En 's/.*"([^"]+)".*/\1/p'
}

team_identifier_for_identity() {
  local identity="$1"
  local subject
  subject="$(
    security find-certificate -c "$identity" -p 2>/dev/null |
      openssl x509 -noout -subject -nameopt RFC2253 2>/dev/null |
      head -n 1
  )"
  printf '%s\n' "$subject" | sed -En 's/(^|.*,)?OU=([^,]+).*/\2/p'
}

development_signing_identity() {
  local requested_team
  local identity
  local team

  if [[ -n "${IRIS_DRIVE_CODESIGN_IDENTITY:-}" ]]; then
    printf '%s\n' "$IRIS_DRIVE_CODESIGN_IDENTITY"
    return 0
  fi

  requested_team="$(development_team)"
  while IFS= read -r identity; do
    [[ "$identity" == Apple\ Development:* ]] || continue
    team="$(team_identifier_for_identity "$identity")"
    if [[ -z "$requested_team" || "$team" == "$requested_team" ]]; then
      printf '%s\n' "$identity"
      return 0
    fi
  done < <(codesigning_identities)

  if [[ -n "$requested_team" ]]; then
    echo "No Apple Development identity found for team $requested_team." >&2
  else
    echo "No Apple Development signing identity found." >&2
  fi
  return 1
}

xcode_app_entitlements() {
  local entitlements="$DERIVED_DATA/Build/Intermediates.noindex/IrisDriveMac.build/$CONFIGURATION/IrisDriveMac.build/Iris Drive.app.xcent"
  if [[ -f "$entitlements" ]]; then
    printf '%s\n' "$entitlements"
  else
    printf '%s\n' "$ROOT/macos/IrisDriveMac.entitlements"
  fi
}

xcode_appex_entitlements() {
  local entitlements="$DERIVED_DATA/Build/Intermediates.noindex/IrisDriveMac.build/$CONFIGURATION/IrisDriveFileProvider.build/IrisDriveFileProvider.appex.xcent"
  if [[ -f "$entitlements" ]]; then
    printf '%s\n' "$entitlements"
  else
    printf '%s\n' "$ROOT/macos/FileProvider/FileProvider.entitlements"
  fi
}

idrive_helper_entitlements() {
  printf '%s\n' "$ROOT/macos/idrive-helper.entitlements"
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
      if development_signing_identity >/dev/null 2>&1; then
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

use_xcode_managed_signing() {
  environment_flag "${IRIS_DRIVE_MACOS_XCODE_MANAGED_SIGNING:-1}"
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

launch_services_tool() {
  printf '%s\n' "/System/Library/Frameworks/CoreServices.framework/Frameworks/LaunchServices.framework/Support/lsregister"
}

install_app_bundle() {
  local built_app_path="$1"

  if [[ "$INSTALL_APP_PATH" != *.app || "$INSTALL_APP_PATH" == "/" ]]; then
    echo "Unsafe IRIS_DRIVE_MACOS_INSTALL_APP: $INSTALL_APP_PATH" >&2
    exit 2
  fi
  mkdir -p "$(dirname "$INSTALL_APP_PATH")"
  rm -rf "$INSTALL_APP_PATH"
  ditto "$built_app_path" "$INSTALL_APP_PATH"
  printf '%s\n' "$INSTALL_APP_PATH"
}

register_app_bundle() {
  local app_path="$1"
  local built_app_path="$2"
  local lsregister
  local candidate
  local stale_root

  lsregister="$(launch_services_tool)"
  [[ -x "$lsregister" ]] || return 0

  "$lsregister" -u "$built_app_path" >/dev/null 2>&1 || true
  if command -v mdfind >/dev/null 2>&1; then
    mdfind "kMDItemCFBundleIdentifier == 'to.iris.drive.macos'" 2>/dev/null \
      | while IFS= read -r candidate; do
          [[ -n "$candidate" && "$candidate" != "$app_path" ]] || continue
          "$lsregister" -u "$candidate" >/dev/null 2>&1 || true
        done
  fi
  if [[ -d "$HOME/Library/Developer/Xcode/DerivedData" ]]; then
    find "$HOME/Library/Developer/Xcode/DerivedData" \
      -path "*/Build/Products/Debug/Iris Drive.app" \
      -type d -prune -print 2>/dev/null \
      | while IFS= read -r candidate; do
          [[ -n "$candidate" && "$candidate" != "$app_path" ]] || continue
          "$lsregister" -u "$candidate" >/dev/null 2>&1 || true
          rm -rf "$candidate"
        done
  fi
  for stale_root in /private/tmp/iris-drive-sign-tests /tmp/iris-drive-sign-tests; do
    [[ -d "$stale_root" ]] || continue
    find "$stale_root" \
      -name "*.app" \
      -type d -prune -print 2>/dev/null \
      | while IFS= read -r candidate; do
          "$lsregister" -u "$candidate" >/dev/null 2>&1 || true
        done
    rm -rf "$stale_root"
  done
  "$lsregister" -f -R -trusted "$app_path" >/dev/null 2>&1 || true
}

register_fileprovider_plugin() {
  local app_path="$1"
  local appex="$app_path/Contents/PlugIns/IrisDriveFileProvider.appex"
  local plugin_id="to.iris.drive.macos.FileProvider"
  local plugin

  [[ -d "$appex" ]] || return 0
  command -v pluginkit >/dev/null 2>&1 || return 0

  pluginkit -m -i "$plugin_id" -ADv 2>/dev/null \
    | awk -F '\t' 'NF >= 4 { print $4 }' \
    | while IFS= read -r plugin; do
        if [[ -n "$plugin" && "$plugin" != "$appex" ]]; then
          pluginkit -r "$plugin" >/dev/null 2>&1 || true
        fi
      done
  pluginkit -a "$appex" >/dev/null 2>&1 || true
  pluginkit -e use -i "$plugin_id" >/dev/null 2>&1 || true
}

build_xcode_app() {
  local mode="$1"
  local target_dir="$2"
  local rust_lib_dir="$target_dir/debug"
  local auth_args=()
  local args=(
    -project "$PROJECT"
    -scheme "$SCHEME"
    -configuration "$CONFIGURATION"
    -derivedDataPath "$DERIVED_DATA"
    -destination "platform=macOS,arch=$HOST_ARCH"
    "LIBRARY_SEARCH_PATHS=$rust_lib_dir"
    "OTHER_LDFLAGS=$rust_lib_dir/libiris_drive_app_core.a"
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

  if [[ "$mode" == "development" ]] && use_xcode_managed_signing; then
    local team
    team="$(development_team)"
    if [[ -z "$team" ]]; then
      team="$(team_identifier_for_identity "$(development_signing_identity)")"
    fi
    if [[ -z "$team" ]]; then
      echo "Could not infer IRIS_DRIVE_DEVELOPMENT_TEAM from a local Apple Development identity." >&2
      exit 2
    fi
    args+=(DEVELOPMENT_TEAM="$team")
    if [[ "${IRIS_DRIVE_ALLOW_PROVISIONING_UPDATES:-1}" != "0" ]]; then
      if [[ ${#auth_args[@]} -gt 0 ]]; then
        args+=("${auth_args[@]}")
      fi
      args+=(-allowProvisioningUpdates -allowProvisioningDeviceRegistration)
    fi
  else
    args+=(CODE_SIGNING_ALLOWED=NO)
  fi

  log "Building macOS app ($mode signing); log: $BUILD_LOG"
  xcodebuild "${args[@]}" build >"$BUILD_LOG"
}

prepare_development_entitlements() {
  local source="$1"
  local destination="$2"
  local team="$3"

  TEAM_IDENTIFIER="$team" python3 - "$source" "$destination" <<'PY'
import os
import plistlib
import sys

source, destination = sys.argv[1], sys.argv[2]
team = os.environ["TEAM_IDENTIFIER"]
with open(source, "rb") as handle:
    entitlements = plistlib.load(handle)

def expand(value):
    if isinstance(value, str):
        return value.replace("$(TeamIdentifierPrefix)", f"{team}.")
    if isinstance(value, list):
        return [expand(item) for item in value]
    if isinstance(value, dict):
        return {key: expand(item) for key, item in value.items()}
    return value

def truthy(name, default=False):
    value = os.environ.get(name)
    if value is None:
        return default
    return value in {"1", "true", "TRUE", "yes", "YES", "on", "ON"}

entitlements = expand(entitlements)
if not truthy("IRIS_DRIVE_MACOS_KEEP_PROVISIONED_DEBUG_ENTITLEMENTS"):
    entitlements.pop("com.apple.developer.associated-domains", None)
    if not truthy("IRIS_DRIVE_MACOS_KEEP_FILEPROVIDER_TESTING_MODE", True):
        entitlements.pop("com.apple.developer.fileprovider.testing-mode", None)
elif not truthy("IRIS_DRIVE_MACOS_KEEP_FILEPROVIDER_TESTING_MODE", True):
    entitlements.pop("com.apple.developer.fileprovider.testing-mode", None)

with open(destination, "wb") as handle:
    plistlib.dump(entitlements, handle, sort_keys=False)
PY
}

sign_helper() {
  local helper="$1"
  local mode="$2"
  local identity="${3:-}"

  if [[ "$mode" == "development" ]]; then
    codesign --force --sign "$identity" --entitlements "$(idrive_helper_entitlements)" "$helper" >&2
  else
    codesign --force --sign - "$helper" >&2
  fi
}

sign_fileprovider_extension() {
  local appex="$1"
  local mode="$2"
  local identity="${3:-}"
  local entitlements="${4:-}"

  if [[ "$mode" == "development" ]]; then
    codesign --force --sign "$identity" --entitlements "$entitlements" "$appex" >&2
  else
    codesign --force --sign - --entitlements "$(xcode_appex_entitlements)" "$appex" >&2
  fi
}

finalize_app_signature() {
  local app_path="$1"
  local mode="$2"
  local identity="${3:-}"
  local entitlements="${4:-}"

  if [[ "$mode" == "development" ]]; then
    codesign --force --sign "$identity" --entitlements "$entitlements" "$app_path" >&2
    codesign --verify --strict --deep "$app_path" >&2
  fi
}

terminate_running_app() {
  if pgrep -x "$APP_PROCESS_NAME" >/dev/null 2>&1; then
    pkill -TERM -x "$APP_PROCESS_NAME" >/dev/null 2>&1 || true
    for _ in {1..40}; do
      if ! pgrep -x "$APP_PROCESS_NAME" >/dev/null 2>&1; then
        break
      fi
      sleep 0.1
    done
    pkill -x "$APP_PROCESS_NAME" >/dev/null 2>&1 || true
  fi
  if pgrep -x "IrisDriveFileProvider" >/dev/null 2>&1; then
    pkill -TERM -x "IrisDriveFileProvider" >/dev/null 2>&1 || true
    for _ in {1..40}; do
      if ! pgrep -x "IrisDriveFileProvider" >/dev/null 2>&1; then
        break
      fi
      sleep 0.1
    done
    pkill -x "IrisDriveFileProvider" >/dev/null 2>&1 || true
  fi
}

build_app() {
  local mode
  local target_dir
  local built_app_path
  local app_path
  local signing_identity=""
  local signing_team=""
  local app_entitlements=""
  local appex_entitlements=""

  mode="$(signing_mode)"
  if command -v xcodegen >/dev/null 2>&1; then
    log "Generating macOS project"
    (cd "$ROOT/macos" && xcodegen generate) >&2
  elif [[ ! -d "$PROJECT" ]]; then
    echo "Missing $PROJECT and xcodegen is not installed." >&2
    exit 1
  else
    log "Using existing macOS project; xcodegen is not installed"
  fi

  log "Building idrive helper"
  cargo build -p idrive >&2
  log "Building app-core library"
  cargo build -p iris-drive-app-core >&2
  target_dir="$(resolve_target_dir)"

  build_xcode_app "$mode" "$target_dir"
  built_app_path="$(resolve_app_path)"
  if [[ -z "${built_app_path:-}" || ! -d "$built_app_path" ]]; then
    echo "Built macOS app not found. Build log: $BUILD_LOG" >&2
    exit 1
  fi
  terminate_running_app
  app_path="$(install_app_bundle "$built_app_path")"

  if [[ "$mode" == "development" ]]; then
    signing_identity="$(development_signing_identity)"
    signing_team="$(team_identifier_for_identity "$signing_identity")"
    if [[ -z "$signing_team" ]]; then
      echo "Could not resolve team identifier for signing identity: $signing_identity" >&2
      exit 2
    fi
    mkdir -p "$BUILD_DIR/Signing"
    app_entitlements="$BUILD_DIR/Signing/IrisDriveMac.development.entitlements"
    appex_entitlements="$BUILD_DIR/Signing/IrisDriveFileProvider.development.entitlements"
    prepare_development_entitlements "$(xcode_app_entitlements)" "$app_entitlements" "$signing_team"
    prepare_development_entitlements "$(xcode_appex_entitlements)" "$appex_entitlements" "$signing_team"
  fi

  cp "$target_dir/debug/idrive" "$app_path/Contents/MacOS/idrive"
  cp "$target_dir/debug/idrive" "$app_path/Contents/PlugIns/IrisDriveFileProvider.appex/Contents/MacOS/idrive"
  chmod +x "$app_path/Contents/MacOS/idrive"
  chmod +x "$app_path/Contents/PlugIns/IrisDriveFileProvider.appex/Contents/MacOS/idrive"
  sign_helper "$app_path/Contents/MacOS/idrive" "$mode" "$signing_identity"
  sign_helper "$app_path/Contents/PlugIns/IrisDriveFileProvider.appex/Contents/MacOS/idrive" "$mode" "$signing_identity"
  sign_fileprovider_extension "$app_path/Contents/PlugIns/IrisDriveFileProvider.appex" "$mode" "$signing_identity" "$appex_entitlements"
  finalize_app_signature "$app_path" "$mode" "$signing_identity" "$app_entitlements"

  touch "$app_path"
  register_app_bundle "$app_path" "$built_app_path"
  register_fileprovider_plugin "$app_path"

  BUILD_APP_PATH="$app_path"
  printf '%s\n' "$app_path"
}

run_app() {
  local app_path
  local app_base_dir="${IRIS_DRIVE_APP_BASE_DIR:-}"
  local mode

  mode="$(signing_mode)"
  build_app
  app_path="$BUILD_APP_PATH"
  if [[ -z "$app_base_dir" && "$mode" != "development" ]]; then
    app_base_dir="$BUILD_DIR/AppData"
  fi

  terminate_running_app
  if [[ -n "$app_base_dir" ]]; then
    mkdir -p "$app_base_dir"
    open \
      --env "IRIS_DRIVE_APP_BASE_DIR=$app_base_dir" \
      --env "IRIS_DRIVE_FILEPROVIDER_RESET_ON_START=true" \
      "$app_path"
    echo "macOS app data: $app_base_dir"
  else
    open \
      --env "IRIS_DRIVE_FILEPROVIDER_RESET_ON_START=true" \
      "$app_path"
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
