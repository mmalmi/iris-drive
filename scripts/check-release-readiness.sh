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
  if [[ "$(grep -Fxc -- "name = \"$package\"" "$ROOT/$path")" != 1 ]]; then
    echo "expected one registry $package family in $path" >&2
    exit 1
  fi
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
require_file haps.toml
require_contains haps.toml '# https://haps.hashtree.cc'
require_file .env.zapstore.example
require_file zapstore.yaml
require_file LICENSE

require_contains Justfile "release-gate *args:"
require_contains Justfile "verify-fast:"
require_contains Justfile "verify-full:"
require_contains Justfile "verify-health:"
require_contains scripts/verify.sh 'cargo clippy --locked --workspace --all-targets --keep-going -- -D warnings'
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
require_contains scripts/local-release.mjs "loadReleaseEnv("
require_contains scripts/release-env.mjs ".env.zapstore.local"
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
require_contains scripts/local-release.mjs "macosReleaseSmokeCommand"
require_contains scripts/macos-release-smoke-host.mjs "macos-release-smoke.sh"
require_contains scripts/macos-release-smoke-host.mjs "macos-vm-release-smoke.sh"
# Hosted-build release route: stage, sign, gate, publish and read back.
require_executable scripts/release-hosted-stage.mjs
require_executable scripts/select-hosted-release-products.py
require_file scripts/release-android-signing.mjs
require_executable scripts/release-ios-hosted.mjs
require_executable scripts/release-gate-env.mjs
require_executable scripts/macos-vm-release-smoke.sh
require_file scripts/lib/macos-vm-host.sh
require_executable scripts/release-tools.mjs
require_file scripts/release-tools.lock.json
require_executable scripts/zapstore-readback.mjs
require_file scripts/fixtures/zapstore-readback-events.json
for route_test in tools hosted-stage android-signing ios-hosted gate-env macos-vm-smoke zapstore-readback route; do
  require_file "scripts/local-release-$route_test.test.mjs"
done
require_contains scripts/release-hosted-stage.mjs "verifyProductReceipt"
require_contains scripts/release-hosted-stage.mjs "stageHostedProduct"
require_contains scripts/local-release.mjs "releaseToolsEnv"
require_absent scripts/local-release.mjs "commandExists('htree')"
require_contains README.md "node scripts/release-hosted-stage.mjs --platform android --run"
require_contains README.md "node scripts/release-gate-env.mjs -- just release-gate"
require_contains README.md "node scripts/release-tools.mjs install"
require_contains README.md "node scripts/zapstore-readback.mjs --tag"
require_contains scripts/macos-release-smoke.sh "verify_file_provider_contract"
require_contains scripts/macos-release-smoke.sh "verify_embedded_license"
require_contains scripts/macos-release-smoke.sh 'com\.apple\.security\.application-groups.0'
require_contains scripts/macos-release-smoke.sh 'REGISTERED_APP_PATHS'
require_contains scripts/macos-release-smoke.sh '"$LSREGISTER" -u "$app"'
require_contains scripts/local-release.mjs "IRIS_DRIVE_RELEASE_RESOLVER_REFRESH_BASE_URLS"
require_contains scripts/local-release.mjs "api/resolve"
require_contains scripts/local-release.mjs "IRIS_DRIVE_MACOS_KEEP_PROVISIONED_ENTITLEMENTS"
require_contains scripts/local-release.mjs "join(distDir, 'macos', 'provisioning.env')"
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
require_contains Cargo.toml 'nvpn-fips-core = { version = "=0.4.123" }'
require_contains Cargo.toml 'nvpn-fips-endpoint = { version = "=0.4.123" }'
require_contains Cargo.toml 'nvpn-fips-tcp = { version = "=0.2.6" }'
require_contains Cargo.toml 'nvpn-fips-tcp-endpoint = { version = "=0.2.57" }'
require_contains Cargo.toml 'hashtree-core = "=0.3.1"'
require_contains Cargo.toml 'hashtree-config = "=0.2.83"'
require_contains Cargo.toml 'hashtree-embedded = "=0.4.2"'
require_contains Cargo.toml 'hashtree-fips-transport = { version = "=0.5.0", features = ["webrtc-endpoint"] }'
require_contains Cargo.toml 'hashtree-lmdb = "=0.3.0"'
require_contains Cargo.toml 'hashtree-network = "=0.3.0"'
require_contains Cargo.toml 'hashtree-nostr = "=0.4.0"'
require_contains Cargo.toml 'nostr-identity = "=0.3.1"'
require_contains Cargo.toml 'nostr-pubsub-fips = "=0.6.6"'
require_contains crates/iris-drive-core/src/fips_bootstrap.rs '"wss://fips1.iris.to/fips"'
require_contains crates/iris-drive-core/src/fips_bootstrap.rs '"wss://fips2.iris.to/fips"'
require_contains crates/iris-drive-core/Cargo.toml "nvpn-fips-core.workspace = true"
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
  require_registry_package "$lock" negentropy 0.5.3 fd6f12461d14b5768028d39fab3cdfbcefb0bc7f09e95f413a00b6fdedcc9c2d
  require_registry_package "$lock" hashtree-resolver 0.3.0 365c245edfd117d2d0bbfd4e84d0c9876930a6c970485a95b3083e0f5ce651e4
  require_registry_package "$lock" nvpn-fips-core 0.4.123 6b23ab830c7abdd3510dd2586688a0b29f4a34646d524e7937145df70903599c
  require_registry_package "$lock" nvpn-fips-endpoint 0.4.123 4fc3adaa33ed34c197e7d3dd3390fd1ea308c8ac3e2012caf65b3c533c5328b2
  require_registry_package "$lock" nvpn-fips-tcp 0.2.6 299df22559c4af63e7d276fcb90cf000f2f1cd5546c213f17d8860154c1383d7
  require_registry_package "$lock" nvpn-fips-tcp-endpoint 0.2.57 8f3fa2f912e0d105a7ee4166c15df11ac5179d453a152b339e351868ecb871eb
  require_registry_package "$lock" git-remote-htree 0.4.2 fee9332a3366c8a61709d12973c79451721d0372706abf8e37b2a6da39808f7e
  require_registry_package "$lock" hashtree-cli 0.4.6 4c692044a77a611977552d0ac0205a81602ef2098232b89b1a993f6bcbdeb34e
  require_registry_package "$lock" hashtree-updater 0.3.0 d74b436997bd5c98adeeef4d03177a9403f24eeb55c76ea0f3cb42a3a7c62e52
  require_registry_package "$lock" hashtree-config 0.2.83 661c0bec57ba49999860fc418a7e656714cd79d82a3c3ee272794b90bb49db76
  require_registry_package "$lock" hashtree-core 0.3.1 ad3de9ec303ed9a02551e91ef30d6ac62f44b1cb46f1fd44ab89f7570c4ff127
  require_registry_package "$lock" hashtree-embedded 0.4.2 aebe8a56e536dcf701444e79b1905b870e5748bc4d0150836314d17adab1fbd7
  require_registry_package "$lock" hashtree-fips-transport 0.5.0 fe8ec8fd82d673c7f0e6c692cabdbf89839cdcba9c5600ec9077e8f1e799d824
  require_registry_package "$lock" hashtree-lmdb 0.3.0 e175d64320b99ec4df162d2db593d18b25528a5362456dee0aac0e704c49c5e5
  require_registry_package "$lock" hashtree-network 0.3.0 e5e282964a08e2ee6851006a91823740672fa5f103d5de652532ee03186381c5
  require_registry_package "$lock" hashtree-nostr 0.4.0 381059616e65a3154eeac615a811ef18d854400958f78dfb8e11024f65800501
  require_registry_package "$lock" hashtree-nostr-pubsub 0.4.0 114ab31557c3e5466969fcfa9d7f0a294fbd7069bc501274f47860ff763b6b5b
  require_registry_package "$lock" nostr-pubsub 0.1.21 8e06419d62a5196b9d9d5f7ed50a54664a08cec1497de18cae5a2643547d3907
  require_registry_package "$lock" nostr-pubsub-fips 0.6.6 65a202b9198f979184b476ecb1b5614be02ac059e91901212ff3df7e4d9f19af
  require_registry_package "$lock" nostr-pubsub-reconcile 0.1.4 63865303e25e95741c1ccaf51873124bfb2ebe74660a8d4c9c7df15b02d2f18a
  require_registry_package "$lock" nostr-pubsub-social-graph 0.3.1 6f7f5d8b5ae252baf6bba6f1fafcd4d0df80a2a9b31eaccc3786016e35f25abd
  require_registry_package "$lock" hashtree-nostr-social-graph-heed 0.2.1-hashtree.1 f48e56163b5a87fadafb206194e37edd5c75c1a66f138225c9a36e6a6b2780e0
  require_registry_package "$lock" nostr-social-graph 0.2.2 94ad34129aa71f3cb8344e39d3a690c08c7d84b5a230a42e146efc0aaa3f2d22
  require_registry_package "$lock" nostr-social-memory 0.2.1 4b28fbf489df058c877fba6d660d041883683d40272b5dbb8cf56679f1c105a1
  require_registry_package "$lock" nostr-pubsub-relay 0.1.15 fe53648d219d8fc5e3f8c2606d3dced398c383521127aeeaa9f3b345d2d3a9a9
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
require_contains scripts/cross-vm-e2e.sh 'IRIS_DRIVE_IDLE_CPU_COMMAND_MATCH = $(ps_quote "$config")'
require_contains scripts/cross-vm-e2e.sh 'bash \"\$sampler\" --platform auto'
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
