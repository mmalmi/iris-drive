#!/usr/bin/env bash

set -Eeuo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"

require_file() {
  local path="$1"
  if [[ ! -f "$ROOT/$path" ]]; then
    echo "missing release readiness file: $path" >&2
    exit 1
  fi
}

require_executable() {
  local path="$1"
  require_file "$path"
  if [[ ! -x "$ROOT/$path" ]]; then
    echo "release readiness file is not executable: $path" >&2
    exit 1
  fi
}

require_contains() {
  local path="$1"
  local needle="$2"
  if ! grep -Fq -- "$needle" "$ROOT/$path"; then
    echo "missing '$needle' in $path" >&2
    exit 1
  fi
}

require_absent() {
  local path="$1"
  local needle="$2"
  if grep -Fq -- "$needle" "$ROOT/$path"; then
    echo "unexpected '$needle' in $path" >&2
    exit 1
  fi
}

require_registry_package() {
  local path="$1" package="$2" version="$3" checksum="$4" block
  block="$(awk -v package="$package" \
    '/^\[\[package\]\]$/ { capture = 0 } $0 == "name = \"" package "\"" { capture = 1 } capture' \
    "$ROOT/$path")"
  for needle in \
    "version = \"$version\"" \
    'source = "registry+https://github.com/rust-lang/crates.io-index"' \
    "checksum = \"$checksum\""
  do
    if ! grep -Fxq -- "$needle" <<<"$block"; then
      echo "missing registry $package $version provenance in $path" >&2
      exit 1
    fi
  done
}

require_executable scripts/release-gate.sh
require_executable scripts/verify.sh
require_executable scripts/verify_full_native.sh
require_executable scripts/native_lab.py
require_executable scripts/native_state_reset.sh
require_file scripts/reset_windows_cloudfiles.ps1
require_file scripts/remove_fileprovider_domain.swift
require_executable scripts/idle-cpu-gate.sh
require_file scripts/idle-cpu-gate-windows.ps1
require_executable scripts/macos-release-smoke.sh
require_executable scripts/macos-profiles
require_executable scripts/ios-build
require_executable scripts/ios-profiles
require_executable scripts/testflight-internal
require_executable scripts/testflight-public
require_file scripts/macos-entitlements.mjs
require_file .env.release.example
require_file .env.zapstore.example
require_file zapstore.yaml

require_contains Justfile "release-gate *args:"
require_contains Justfile "verify-fast:"
require_contains Justfile "verify-full:"
require_contains Justfile "verify-health:"
require_contains scripts/verify.sh 'cargo clippy --workspace --all-targets -- -D warnings'
require_contains scripts/native_lab.py 'infrastructure_unavailable'
require_contains Justfile "node scripts/local-release.mjs --build"
require_contains Justfile "release-publish:"
require_contains Justfile "release-final:"
require_contains scripts/local-release.mjs "--build"
require_contains scripts/local-release.mjs "--skip-zapstore"
require_contains scripts/local-release.mjs "publishZapstore"
require_contains scripts/local-release.mjs "scripts', 'ios-build'"
require_contains scripts/local-release.mjs "IRIS_DRIVE_IOS_TESTFLIGHT_CHANNELS"
require_contains scripts/local-release.mjs "IRIS_DRIVE_IOS_MARKETING_VERSION"
require_contains scripts/local-release.mjs "App Store Connect API key file"
require_contains scripts/local-release.mjs ".env.zapstore.local"
require_contains scripts/local-release.mjs "requireCompleteAppRelease"
require_contains scripts/local-release.mjs "validateFinalReleaseBuildInputs"
require_contains scripts/local-release.mjs "validateFinalPublishInputs"
require_contains scripts/local-release.mjs "windowsPeHasAuthenticodeSignature"
require_contains scripts/local-release.mjs "IRIS_DRIVE_ALLOW_UNSIGNED_WINDOWS"
require_contains scripts/local-release.mjs "Missing Zapstore signing key"
require_contains scripts/local-release.mjs "notarytool"
require_contains scripts/local-release.mjs "stapler"
require_contains scripts/local-release.mjs "macos-release-smoke.sh"
require_contains scripts/local-release.mjs "IRIS_DRIVE_RELEASE_RESOLVER_REFRESH_BASE_URLS"
require_contains scripts/local-release.mjs "api/resolve"
require_contains scripts/local-release.mjs "IRIS_DRIVE_MACOS_KEEP_PROVISIONED_ENTITLEMENTS"
require_contains scripts/local-release.mjs "dist', 'macos', 'provisioning.env"
require_contains scripts/local-release.mjs "MARKETING_VERSION="
require_contains scripts/local-release.mjs "-PirisDriveVersionName="
require_contains android/app/build.gradle.kts "irisDriveVersionName"
require_contains scripts/ios-build "ios-testflight-public"
require_contains scripts/ios-build "scripts/ios-profiles"
require_contains scripts/ios-build "testflight-internal"
require_contains scripts/ios-build 'FILE_PROVIDER_BUNDLE_ID="${IRIS_DRIVE_IOS_FILE_PROVIDER_BUNDLE_ID:-$BUNDLE_ID.FileProvider}"'
require_contains scripts/ios-build "IRIS_DRIVE_IOS_APP_GROUP_IDENTIFIER"
require_contains scripts/ios-build "IRIS_DRIVE_IOS_SIGNING_STYLE"
require_contains scripts/ios-build "-authenticationKeyPath"
require_contains scripts/testflight-internal "testflight-app-store-connect.mjs"
require_contains scripts/testflight-public "testflight-app-store-connect.mjs"
require_contains scripts/testflight-app-store-connect.mjs "betaAppReviewSubmissions"
require_contains scripts/ios-build "testFlightInternalTestingOnly"
require_contains scripts/ios-build "iTMSTransporter"
require_contains scripts/local-release-lib.mjs "validateReleaseAssetSet"
require_contains scripts/local-release-lib.mjs "plannedReleaseAssetNames"
require_contains android/app/build.gradle.kts "ANDROID_KEYSTORE_PATH"
require_contains scripts/release-gate.sh "just structure"
require_contains scripts/release-gate.sh "cargo test --workspace --exclude idrive"
require_contains scripts/release-gate.sh "--test daemon_sync_matrix"
require_contains scripts/release-gate.sh "cargo build --workspace --release"
require_contains Cargo.toml 'fips-core = "=0.4.6"'
require_contains Cargo.toml 'hashtree-core = "=0.2.86"'
require_contains Cargo.toml 'hashtree-config = "=0.2.83"'
require_contains Cargo.toml 'hashtree-embedded = "=0.2.86"'
require_contains Cargo.toml 'hashtree-fips-transport = { version = "=0.4.6"'
require_contains Cargo.toml 'hashtree-lmdb = "=0.2.85"'
require_contains Cargo.toml 'hashtree-network = "=0.2.87"'
require_contains Cargo.toml 'hashtree-nostr = "=0.2.83"'
require_contains Cargo.toml 'nostr-identity = "=0.3.1"'
require_contains crates/iris-drive-core/Cargo.toml "fips-core.workspace = true"
require_absent Cargo.toml "[patch.crates-io]"
require_absent Cargo.toml "git = "
require_absent Cargo.toml 'path = "crates/hashtree-fips-transport"'
require_absent Cargo.toml 'path = "../nostr-social-graph'
require_absent linux/Cargo.toml "[patch.crates-io]"
for lock in Cargo.lock linux/Cargo.lock; do
  require_registry_package "$lock" fips-core 0.4.6 12cc0df5e04a1aae16efa85313976e87eb037d6e7955b8a035febd91b00383dc
  require_registry_package "$lock" fips-tcp 0.2.0 d18861c5eca7c472fbbdbbfb498f8d2525405081a9a24b42633c600ba6f6e42a
  require_registry_package "$lock" fips-tcp-endpoint 0.2.0 8e3e01e352b709b80f4261e2cd7d0ffde2d3aaf175267b3960997e70f7305c12
  require_registry_package "$lock" hashtree-cli 0.2.100 eb8ee2266a43e58b4182da07c8829fa06b0e27e32d66164d6385ccedd6de2e89
  require_registry_package "$lock" hashtree-config 0.2.83 661c0bec57ba49999860fc418a7e656714cd79d82a3c3ee272794b90bb49db76
  require_registry_package "$lock" hashtree-core 0.2.86 574476b1fe122bddc7783ba0346dca42ec673a241128b0edf9e38166c1bb800f
  require_registry_package "$lock" hashtree-embedded 0.2.86 28482b66ec409702a50ceeac1a49f5fae42b141326822f394ad66a6ebc37562f
  require_registry_package "$lock" hashtree-fips-transport 0.4.6 3817b451831f915787090cb1ca33dac2e5313bc1e5afd2da515f0e57bb0c997f
  require_registry_package "$lock" hashtree-lmdb 0.2.85 e61f72986fce9c84f9fd03c72c581af092e25ea698e8b7bc54ddc18fe821286b
  require_registry_package "$lock" hashtree-network 0.2.87 aa83a68204dfbdc10f2fa9e810740c981e6f87c114a763ee56d8823df02c077e
  require_registry_package "$lock" hashtree-nostr 0.2.83 489b2bf6d5e57921409aeee0a199a0f82512d473d290143306fc053d815c6973
  require_registry_package "$lock" nostr-pubsub-fips 0.3.1 5663a6108ae432879d6d7441036b979605fc032011c0a6e81dbf1798ce844f6c
done
require_absent scripts/docker-cli-e2e.sh "Missing required sibling checkout"
require_contains scripts/docker-cli-e2e.sh '-v "$ROOT:/work/iris-drive:ro"'
require_contains scripts/release-gate.sh "IRIS_DRIVE_RELEASE_GATE_FULL"
require_contains scripts/release-gate.sh "IRIS_DRIVE_RELEASE_GATE_IDLE_CPU"
require_contains scripts/release-gate.sh "just macos-build"
require_contains scripts/release-gate.sh "just smoke-macos"
require_contains scripts/release-gate.sh "run_macos_idle_cpu_gate"
require_contains scripts/release-gate.sh "macOS idle CPU gate"
require_contains scripts/release-gate.sh "idle-cpu-gate.sh --platform macos"
require_contains scripts/release-gate.sh "ios-smoke builds the simulator app"
require_contains scripts/release-gate.sh "just ios-smoke"
require_contains scripts/release-gate.sh "just ios-gui-smoke"
require_contains scripts/release-gate.sh "idle-cpu-gate.sh --platform ios"
require_contains scripts/release-gate.sh "just android-build"
require_contains scripts/release-gate.sh "just android-gui-smoke"
require_contains scripts/release-gate.sh "idle-cpu-gate.sh --platform android"
require_contains scripts/release-gate.sh "just e2e-5devices"
require_contains scripts/idle-cpu-gate.sh "IRIS_DRIVE_IDLE_CPU_REQUIRED_ROLES"
require_contains scripts/idle-cpu-gate.sh "IRIS_DRIVE_IDLE_CPU_COMMAND_MATCH"
require_contains scripts/idle-cpu-gate.sh "/Iris Drive.app/Contents/PlugIns/IrisDriveFileProvider.appex/Contents/MacOS/IrisDriveFileProvider"
require_contains scripts/idle-cpu-gate.sh "idle-cpu-gate-windows.ps1"
require_contains scripts/idle-cpu-gate-windows.ps1 "IRIS_DRIVE_IDLE_CPU_REQUIRED_ROLES"
require_contains scripts/idle-cpu-gate-windows.ps1 "IRIS_DRIVE_IDLE_CPU_COMMAND_MATCH"
require_contains crates/iris-drive-core/src/daemon/tests/mod.rs "embedded_browser_does_not_pin_iris_sites_bootstrap_root"
require_contains ios/UITests/IrisDriveIOSUITests.swift "assertIrisAppsLauncherContentLoaded"
require_contains scripts/ios-gui-linking-smoke.sh "testOpenIrisAppsLoadsBrowserWithoutConnectionError"
require_contains scripts/ios-gui-linking-smoke.sh "testMyDriveShowsSyncStatusWithoutMobilePauseControls"
require_contains scripts/cross-vm-five-platform-e2e.sh "IRIS_DRIVE_E2E_UBUNTU_HOST"
require_contains scripts/cross-vm-five-platform-e2e.sh "IRIS_DRIVE_E2E_WINDOWS_HOST"
require_contains scripts/cross-vm-five-platform-e2e.sh "IRIS_DRIVE_E2E_MACOS_HOST"
require_contains scripts/cross-vm-five-platform-e2e.sh "IRIS_DRIVE_E2E_IOS_HOST"
require_contains scripts/cross-vm-five-platform-e2e.sh "IRIS_DRIVE_E2E_ANDROID_HOST"
require_contains scripts/cross-vm-five-platform-e2e.sh "scripts/ios-device-iris-apps-smoke.sh"
require_contains scripts/cross-vm-five-platform-e2e.sh "desktop-gui-smoke.sh\" linux"
require_contains scripts/cross-vm-five-platform-e2e.sh "desktop-gui-smoke.sh\" windows"
require_contains scripts/cross-vm-five-platform-e2e.sh "scripts/ios-gui-linking-smoke.sh"
require_contains scripts/cross-vm-five-platform-e2e.sh "scripts/android-gui-linking-smoke.sh"
require_contains scripts/cross-vm-five-platform-e2e.sh "scripts/mobile-android-smoke.sh --no-build"
require_contains scripts/cross-vm-e2e.sh "IRIS_DRIVE_E2E_IDLE_CPU_GATE"
require_contains scripts/cross-vm-e2e.sh "idle daemon CPU gate"
require_contains scripts/cross-vm-e2e.sh "idle-cpu-gate-windows.ps1"
require_contains scripts/cross-vm-e2e.sh "IRIS_DRIVE_IDLE_CPU_REQUIRED_ROLES = 'daemon'"
require_contains scripts/cross-vm-e2e.sh 'IRIS_DRIVE_IDLE_CPU_COMMAND_MATCH = \$repo'
require_contains scripts/cross-vm-e2e.sh 'idle-cpu-gate.sh\" --platform auto'
require_contains scripts/cross-vm-e2e.sh "https://drive.iris.to/approve-device/"
require_contains scripts/dev-vm-update-run.sh "IRIS_DRIVE_SOCIAL_GRAPH_ROOT"
require_contains scripts/dev-vm-update-run.sh "SOCIAL_GRAPH_BARE"
require_contains scripts/dev-vm-update-run.sh "nostr-social-graph"
require_contains zapstore.yaml "release_source: dist/zapstore-current-android-arm64.apk"
require_contains .env.release.example "IRIS_DRIVE_RELEASE_TREE=releases/iris-drive"
require_contains .env.release.example "IRIS_DRIVE_RELEASE_RESOLVER_REFRESH_BASE_URLS="
require_contains scripts/windows-publish.ps1 '[switch]$Installer'
require_contains scripts/windows-publish.ps1 '[switch]$RequireSigning'
require_contains scripts/windows-publish.ps1 "Invoke-WindowsSign"
require_contains scripts/windows-publish.ps1 "signtool"
require_contains scripts/windows-installer.iss "OutputBaseFilename"
require_contains .env.release.example "IRIS_DRIVE_IOS_TESTFLIGHT_CHANNELS=internal,public"
require_contains .env.release.example "IRIS_DRIVE_IOS_PROFILE_RECREATE=true"
require_contains .env.release.example "IRIS_DRIVE_IOS_PROFILES_ENV_PATH="
require_contains .env.release.example "IRIS_DRIVE_IOS_PUBLIC_TESTFLIGHT=1"
require_contains .env.release.example "IRIS_DRIVE_IOS_BUNDLE_ID=fi.siriusbusiness.drive"
require_contains .env.release.example "IRIS_DRIVE_IOS_SIGNING_STYLE=automatic"
require_contains ios/project.yml 'PRODUCT_BUNDLE_IDENTIFIER: $(IRIS_DRIVE_IOS_BUNDLE_ID)'
require_contains ios/project.yml 'PRODUCT_BUNDLE_IDENTIFIER: $(IRIS_DRIVE_IOS_FILE_PROVIDER_BUNDLE_ID)'
require_contains ios/project.yml 'PRODUCT_BUNDLE_IDENTIFIER: $(IRIS_DRIVE_IOS_SHARE_EXTENSION_BUNDLE_ID)'
require_contains .env.release.example "IRIS_DRIVE_MACOS_CODESIGN_RETRY_DELAY_SECONDS="
require_contains .env.release.example "IRIS_DRIVE_MACOS_NOTARY_KEYCHAIN_PROFILE="
require_contains .env.release.example "IRIS_DRIVE_WINDOWS_SIGNTOOL_CERT_SHA1="
require_contains .env.release.example "IRIS_DRIVE_WINDOWS_SIGNTOOL_PFX_PATH="
require_contains .env.release.example "IRIS_DRIVE_MACOS_KEEP_PROVISIONED_ENTITLEMENTS="
require_contains .env.release.example "IRIS_DRIVE_MACOS_PROFILE_TYPE=MAC_APP_DIRECT"
require_contains .env.release.example "IRIS_DRIVE_MACOS_PROFILES_ENV_PATH="
require_contains .env.release.example "IRIS_DRIVE_MACOS_APP_PROVISIONING_PROFILE="
require_contains .env.release.example "IRIS_DRIVE_MACOS_FILEPROVIDER_PROVISIONING_PROFILE="
require_contains scripts/macos-profiles "IRIS_DRIVE_PROFILES_PLATFORM=macos"
require_contains scripts/ios-profiles "to.iris.drive.macos"
require_contains scripts/ios-profiles "to.iris.drive.macos.FileProvider"
require_contains scripts/ios-profiles "MAC_APP_DIRECT"
require_contains scripts/macos-entitlements.mjs "com.apple.developer.associated-domains"
require_contains .env.release.example "IRIS_DRIVE_TESTFLIGHT_PUBLIC_GROUPS="
require_contains .env.zapstore.example "SIGN_WITH="
require_contains .gitignore ".env.zapstore.local"

echo "RELEASE_READINESS_OK"
