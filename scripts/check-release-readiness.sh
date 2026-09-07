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
require_executable scripts/lib/parallel-gate.sh
require_file scripts/lib/cross-vm-parallel-setup.sh
require_executable scripts/test_release_workflows.py
require_executable scripts/verify.sh
require_executable scripts/verify_full_native.sh
require_executable scripts/native_lab.py
require_executable scripts/native_state_reset.sh
require_file scripts/reset_windows_cloudfiles.ps1
require_file scripts/remove_fileprovider_domain.swift
require_executable scripts/idle-cpu-gate.sh
require_file scripts/idle-cpu-gate-windows.ps1
require_executable scripts/macos-release-smoke.sh
require_executable scripts/macos-vm-git-sync.sh
require_executable scripts/macos-vm-smoke.sh
require_executable scripts/macos-vm-idle-cpu.sh
require_executable scripts/macos-idle-cpu-smoke.sh
require_executable scripts/macos-vm-android-manual-link-e2e.sh
require_executable scripts/macos-android-manual-link-remote.sh
require_executable scripts/macos-profiles
require_executable scripts/ios-build
require_executable scripts/release-build-number.mjs
require_file scripts/release-build-hygiene.mjs
require_file scripts/release-build-hygiene.test.mjs
require_file scripts/release-zip.mjs
require_executable scripts/verify-linux-deb-license.mjs
require_file scripts/local-release-android-license.test.mjs
require_file scripts/local-release-assets.test.mjs
require_file scripts/local-release-platform-license.test.mjs
require_executable scripts/ios-profiles
require_executable scripts/testflight-internal
require_executable scripts/testflight-public
require_file scripts/macos-entitlements.mjs
require_file .env.release.example
require_file release-policy.json
require_file .env.zapstore.example
require_file zapstore.yaml
require_file LICENSE

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
require_contains scripts/local-release-lib.mjs "BUILD_NUMBER_EPOCH"
require_contains scripts/local-release.mjs "IRIS_DRIVE_IOS_TESTFLIGHT_CHANNELS"
require_contains scripts/local-release.mjs "IRIS_DRIVE_IOS_MARKETING_VERSION"
require_contains scripts/local-release.mjs "App Store Connect API key file"
require_contains scripts/local-release.mjs ".env.zapstore.local"
require_contains scripts/local-release.mjs "requireCompleteAppRelease"
require_contains scripts/local-release-lib.mjs "validateCanonicalReleaseAssetSet"
require_contains scripts/local-release.mjs "validateFinalReleaseBuildInputs"
require_contains scripts/local-release.mjs "validateFinalPublishInputs"
require_contains scripts/local-release.mjs "readProjectReleasePolicy"
require_contains scripts/local-release.mjs "assertZipEntriesEqualFiles"
require_contains scripts/local-release.mjs "verify-linux-deb-license.mjs"
require_contains release-policy.json '"signing": "unsigned"'
require_absent scripts/local-release.mjs "IRIS_DRIVE_WINDOWS_SIGNTOOL"
require_absent scripts/local-release.mjs "IRIS_DRIVE_ALLOW_UNSIGNED_WINDOWS"
require_contains scripts/local-release.mjs "Missing Zapstore signing key"
require_contains scripts/local-release.mjs "notarytool"
require_contains scripts/local-release.mjs "stapler"
require_contains scripts/local-release.mjs "macos-release-smoke.sh"
require_contains scripts/macos-release-smoke.sh "verify_file_provider_contract"
require_contains scripts/macos-release-smoke.sh "verify_embedded_license"
require_contains scripts/macos-release-smoke.sh 'com\.apple\.security\.application-groups.0'
require_contains scripts/macos-release-smoke.sh 'REGISTERED_APP_PATHS'
require_contains scripts/macos-release-smoke.sh '"$LSREGISTER" -u "$app"'
require_contains scripts/local-release.mjs "IRIS_DRIVE_RELEASE_RESOLVER_REFRESH_BASE_URLS"
require_contains scripts/local-release.mjs "api/resolve"
require_contains scripts/local-release.mjs "IRIS_DRIVE_MACOS_KEEP_PROVISIONED_ENTITLEMENTS"
require_contains scripts/local-release.mjs "dist', 'macos', 'provisioning.env"
require_contains scripts/local-release.mjs "MARKETING_VERSION="
require_contains scripts/local-release.mjs "-PirisDriveVersionName="
require_contains android/app/build.gradle.kts "irisDriveVersionName"
require_contains scripts/ios-build "ios-testflight-public"
require_contains scripts/ios-build "scripts/ios-profiles"
require_contains scripts/ios-build 'node "$ROOT/scripts/release-build-number.mjs" "$1"'
require_contains scripts/ios-build "testflight-internal"
require_contains scripts/ios-build "verify_ios_export_license"
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
require_contains Cargo.toml 'fips-core = { package = "nvpn-fips-core", version = "=0.4.74" }'
require_contains Cargo.toml 'fips-endpoint = { package = "nvpn-fips-endpoint", version = "=0.4.74" }'
require_contains Cargo.toml 'fips-tcp = { package = "nvpn-fips-tcp", version = "=0.2.1" }'
require_contains Cargo.toml 'fips-tcp-endpoint = { package = "nvpn-fips-tcp-endpoint", version = "=0.2.10" }'
require_contains Cargo.toml 'hashtree-core = "=0.2.89"'
require_contains Cargo.toml 'hashtree-config = "=0.2.83"'
require_contains Cargo.toml 'hashtree-embedded = "=0.2.87"'
require_contains Cargo.toml 'hashtree-fips-transport = { version = "=0.4.13"'
require_contains Cargo.toml 'hashtree-lmdb = "=0.2.88"'
require_contains Cargo.toml 'hashtree-network = "=0.2.87"'
require_contains Cargo.toml 'hashtree-nostr = "=0.2.87"'
require_contains Cargo.toml 'nostr-identity = "=0.3.1"'
require_contains Cargo.toml 'nostr-pubsub-fips = "=0.4.17"'
require_contains crates/iris-drive-core/src/fips_bootstrap.rs '"wss://fips1.iris.to/fips"'
require_contains crates/iris-drive-core/src/fips_bootstrap.rs '"wss://fips2.iris.to/fips"'
require_contains crates/iris-drive-core/Cargo.toml "fips-core.workspace = true"
require_absent Cargo.toml "[patch.crates-io]"
require_absent Cargo.toml "git = "
require_absent Cargo.toml 'path = "crates/hashtree-fips-transport"'
require_absent Cargo.toml 'path = "../nostr-social-graph'
require_absent linux/Cargo.toml "[patch.crates-io]"
require_contains linux/Cargo.toml 'license = "MIT"'
require_contains linux/Cargo.toml 'authors = ["Iris Drive contributors"]'
require_contains linux/Cargo.toml 'repository = "htree://self/iris-drive"'
require_contains linux/Cargo.toml 'copyright = "2026 Iris Drive contributors"'
require_contains linux/Cargo.toml 'license-file = ["../LICENSE", "0"]'
require_contains LICENSE "Permission is hereby granted, free of charge"
require_contains LICENSE 'THE SOFTWARE IS PROVIDED "AS IS"'
for lock in Cargo.lock linux/Cargo.lock; do
  require_registry_package "$lock" nvpn-fips-core 0.4.74 216c0132229957f0340c71dcc47f11634976d211df5c3643a8cc9c588f645a3b
  require_registry_package "$lock" nvpn-fips-endpoint 0.4.74 c026e32c1ca18a272f424578ff49600dedea8512076e46d3747d51ee62c97116
  require_registry_package "$lock" nvpn-fips-tcp 0.2.1 511e85644bc05a28503a09a1fe150a6fcc736aab0d67fab2784bf4a531ab332d
  require_registry_package "$lock" nvpn-fips-tcp-endpoint 0.2.10 f76d4278507a3d6215dc8bdb4ff3a9d8eb52c1f214f3ee8f502a40a559d3f858
  require_registry_package "$lock" hashtree-cli 0.2.142 64438e88efa88badb898d9de225d26e90b128437caa4b15abfa70c12631bfec0
  require_registry_package "$lock" hashtree-updater 0.2.83 e0153f9c43ba08f3e66531fc91113f54974bd20f559d90528414ab17ce2eb951
  require_registry_package "$lock" hashtree-config 0.2.83 661c0bec57ba49999860fc418a7e656714cd79d82a3c3ee272794b90bb49db76
  require_registry_package "$lock" hashtree-core 0.2.89 5fab53a9c7a45beaed44a2c35343b3228bd65c65601041b1338bdbdc71b283b2
  require_registry_package "$lock" hashtree-embedded 0.2.87 b0fb583de515f7b55bf7d7f33de8d44bb8f0b662eff2d65196f9cacecb33ca9d
  require_registry_package "$lock" hashtree-fips-transport 0.4.13 5271016d91840d331e72c3b4fd2f8fb18925f4f944aa0e9eb61e1d9047678eb6
  require_registry_package "$lock" hashtree-lmdb 0.2.88 c2d572a31703499c00549d4b6e71ed148a6d9e8b51e9c0c1f0049c7782092318
  require_registry_package "$lock" hashtree-network 0.2.87 aa83a68204dfbdc10f2fa9e810740c981e6f87c114a763ee56d8823df02c077e
  require_registry_package "$lock" hashtree-nostr 0.2.87 b7b7cbe929862e8089e74bd551a3d57adf45581d23d1e1b44c7da8a82798e1a2
  require_registry_package "$lock" hashtree-nostr-pubsub 0.2.83 6fbf53cd18ddd9caf53dab8483db6a36011c11e3dc62233e0343ff5147c5672b
  require_registry_package "$lock" nostr-pubsub 0.1.13 84bfacf8bb4c535ad4c80dc14bdef1dfe94b2b7064f8bf83509a137f8068e0e1
  require_registry_package "$lock" nostr-pubsub-fips 0.4.17 52ca75398f05280369036f0c9e496b4e535a9c48f68e1a934e1f5c18d552d686
  require_registry_package "$lock" nostr-pubsub-relay 0.1.11 8641200920d163b2d82c34e6f15605cff93a0546e3e0087fce8f3bddaa2329ca
done
require_absent scripts/docker-cli-e2e.sh "Missing required sibling checkout"
require_contains scripts/docker-cli-e2e.sh '-v "$ROOT:/work/iris-drive:ro"'
require_contains scripts/release-gate.sh "IRIS_DRIVE_RELEASE_GATE_FULL"
require_contains scripts/release-gate.sh "IRIS_DRIVE_RELEASE_GATE_FAST_PRECHECKED"
require_contains scripts/release-gate.sh "IRIS_DRIVE_RELEASE_GATE_IDLE_CPU"
require_contains scripts/release-gate.sh "just smoke-macos"
require_contains scripts/release-gate.sh "scripts/macos-vm-smoke.sh"
require_contains scripts/release-gate.sh "scripts/macos-vm-idle-cpu.sh"
require_contains scripts/release-gate.sh "IRIS_DRIVE_MACOS_SSH_HOST"
require_contains scripts/release-gate.sh "macos_vm_gate_enabled"
require_contains scripts/release-gate.sh "IRIS_DRIVE_RELEASE_GATE_MACOS_VM"
require_contains scripts/release-gate.sh "scripts/local-release*.test.mjs"
require_contains scripts/release-gate.sh "scripts/release-build-hygiene.test.mjs"
require_contains scripts/macos-idle-cpu-smoke.sh "macos-dev-app.sh"
require_absent scripts/release-gate.sh "just macos-build"
require_contains scripts/release-gate.sh "run_macos_idle_cpu_gate"
require_contains scripts/macos-idle-cpu-smoke.sh "macOS idle CPU gate"
require_contains scripts/macos-idle-cpu-smoke.sh "idle-cpu-gate.sh\" --platform macos"
require_contains scripts/release-gate.sh "terminate_booted_ios_simulator_instances"
require_contains scripts/release-gate.sh "IRIS_DRIVE_IOS_FILE_PROVIDER_BUNDLE_ID"
require_contains scripts/release-gate.sh "just ios-smoke --no-build"
require_contains scripts/release-gate.sh "just ios-gui-smoke"
require_contains scripts/release-gate.sh "idle-cpu-gate.sh --platform ios"
require_contains scripts/release-gate.sh "just android-gui-smoke"
require_absent scripts/release-gate.sh "just android-build"
require_contains scripts/release-gate.sh "idle-cpu-gate.sh --platform android"
require_contains scripts/release-gate.sh "just e2e-5devices"
require_contains scripts/idle-cpu-gate.sh "IRIS_DRIVE_IDLE_CPU_REQUIRED_ROLES"
require_contains scripts/idle-cpu-gate.sh "IRIS_DRIVE_IDLE_CPU_COMMAND_MATCH"
require_contains scripts/idle-cpu-gate.sh "cleanup_ios_idle_cpu"
require_contains scripts/idle-cpu-gate.sh "simctl terminate"
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
require_contains scripts/cross-vm-five-platform-e2e.sh "scripts/macos-vm-android-manual-link-e2e.sh"
require_contains scripts/cross-vm-five-platform-e2e.sh "IRIS_DRIVE_MOBILE_REUSE_ANDROID_ARTIFACTS=1"
require_contains scripts/cross-vm-e2e.sh "IRIS_DRIVE_E2E_IDLE_CPU_GATE"
require_contains scripts/cross-vm-e2e.sh 'SETUP_REMOTE_TIMEOUT_SECS="${IRIS_DRIVE_E2E_SETUP_REMOTE_TIMEOUT_SECS:-300}"'
require_contains scripts/cross-vm-e2e.sh "idle daemon CPU gate"
require_contains scripts/cross-vm-e2e.sh "idle-cpu-gate-windows.ps1"
require_contains scripts/cross-vm-e2e.sh "IRIS_DRIVE_IDLE_CPU_REQUIRED_ROLES = 'daemon'"
require_contains scripts/cross-vm-e2e.sh 'IRIS_DRIVE_IDLE_CPU_COMMAND_MATCH = \$repo'
require_contains scripts/cross-vm-e2e.sh 'idle-cpu-gate.sh\" --platform auto'
require_contains scripts/cross-vm-e2e.sh '${IRIS_DRIVE_IDLE_CPU_WARMUP_SECS:-180}'
require_contains scripts/cross-vm-e2e.sh "https://drive.iris.to/approve-device/"
require_contains scripts/dev-vm-update-run.sh "IRIS_DRIVE_SOCIAL_GRAPH_ROOT"
require_contains scripts/dev-vm-update-run.sh "SOCIAL_GRAPH_BARE"
require_contains scripts/dev-vm-update-run.sh "nostr-social-graph"
require_contains zapstore.yaml "release_source: dist/zapstore-current-android-arm64.apk"
require_contains .env.release.example "IRIS_DRIVE_RELEASE_TREE=releases/iris-drive"
require_contains .env.release.example "IRIS_DRIVE_RELEASE_RESOLVER_REFRESH_BASE_URLS="
require_contains scripts/windows-publish.ps1 '[switch]$Installer'
require_absent scripts/windows-publish.ps1 "RequireSigning"
require_absent scripts/windows-publish.ps1 "signtool"
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
