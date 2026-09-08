# shellcheck shell=bash

iris_drive_ios_select_build_profile() {
  case "$RUST_IOS_TARGET" in
    aarch64-apple-ios-sim) RUST_IOS_ARCH=arm64 ;;
    x86_64-apple-ios) RUST_IOS_ARCH=x86_64 ;;
    *) echo "FAIL: unsupported Rust iOS simulator target: $RUST_IOS_TARGET" >&2; return 2 ;;
  esac
  case "$CONFIGURATION" in
    Debug) RUST_BUILD_PROFILE=debug ;;
    Release) RUST_BUILD_PROFILE=release ;;
    *) echo "FAIL: unsupported iOS simulator configuration: $CONFIGURATION" >&2; return 2 ;;
  esac
  RUST_LIB_DIR="$TARGET_DIR/$RUST_IOS_TARGET/$RUST_BUILD_PROFILE"
  RUST_STATIC_LIB="$RUST_LIB_DIR/libiris_drive_app_core.a"
}

iris_drive_ios_build_app_core() {
  local args=(build --locked -p iris-drive-app-core --target "$RUST_IOS_TARGET")
  if [[ "$RUST_BUILD_PROFILE" == release ]]; then args+=(--release); fi
  printf '[ios-simulator] cargo %s\n' "${args[*]}" >&2
  cargo "${args[@]}" || return $?
  if [[ ! -f "$RUST_STATIC_LIB" ]]; then
    echo "FAIL: static app-core library not found at $RUST_STATIC_LIB" >&2
    return 1
  fi
  printf '[ios-simulator] Xcode %s architecture=%s links %s\n' "$CONFIGURATION" "$RUST_IOS_ARCH" "$RUST_STATIC_LIB" >&2
}

iris_drive_ios_assert_build_configuration() {
  local app_path="$1"
  if [[ "$app_path" != "$DERIVED_DATA/Build/Products/$CONFIGURATION-iphonesimulator/Iris Drive.app" ]]; then
    echo "FAIL: simulator app does not match requested configuration: $app_path" >&2
    return 1
  fi
  if [[ "$CONFIGURATION" == Release && -n "$(find "$app_path" -name '*.debug.dylib' -print -quit)" ]]; then
    echo "FAIL: Release simulator app contains a Debug dylib: $app_path" >&2
    return 1
  fi
}
