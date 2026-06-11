#!/usr/bin/env bash

set -Eeuo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
APP="$ROOT/macos/Sources/IrisDriveMacApp.swift"
LIFECYCLE="$ROOT/macos/Sources/IrisDriveMacFileProvider.swift"
HELPER_ENTITLEMENTS="$ROOT/macos/idrive-helper.entitlements"
DEV_APP="$ROOT/scripts/macos-dev-app.sh"
DAEMON_RUNTIME="$ROOT/crates/iris-drive-cli/src/daemon/runtime.rs"

require_contains() {
  local file="$1"
  local label="$2"
  local needle="$3"
  if ! grep -Fq "$needle" "$file"; then
    echo "missing '$needle' in $label" >&2
    exit 1
  fi
}

require_not_contains() {
  local file="$1"
  local label="$2"
  local needle="$3"
  if grep -Fq "$needle" "$file"; then
    echo "unexpected '$needle' in $label" >&2
    exit 1
  fi
}

require_contains "$APP" "macos/Sources/IrisDriveMacApp.swift" "ensureFileProviderDomainAfterStatusIfNeeded"
require_contains "$APP" "macos/Sources/IrisDriveMacApp.swift" "removeFileProviderDomainRegistration("
require_contains "$LIFECYCLE" "macos/Sources/IrisDriveMacFileProvider.swift" "irisDriveFileProviderRegistrationIdentityKey"
require_contains "$LIFECYCLE" "macos/Sources/IrisDriveMacFileProvider.swift" "markFileProviderRegistrationCurrent"
require_contains "$LIFECYCLE" "macos/Sources/IrisDriveMacFileProvider.swift" "fileProviderRegistrationIdentityIsCurrent"
require_contains "$LIFECYCLE" "macos/Sources/IrisDriveMacFileProvider.swift" "shouldRepairFileProviderRegistration"
require_contains "$LIFECYCLE" "macos/Sources/IrisDriveMacFileProvider.swift" "repairFileProviderRegistration"
require_contains "$LIFECYCLE" "macos/Sources/IrisDriveMacFileProvider.swift" "Iris Drive repairing stale FileProvider domain registration"
require_contains "$LIFECYCLE" "macos/Sources/IrisDriveMacFileProvider.swift" "Iris Drive FileProvider domain removed without re-add"
require_contains "$HELPER_ENTITLEMENTS" "macos/idrive-helper.entitlements" "com.apple.security.inherit"
require_not_contains "$HELPER_ENTITLEMENTS" "macos/idrive-helper.entitlements" "com.apple.security.app-sandbox"
require_not_contains "$HELPER_ENTITLEMENTS" "macos/idrive-helper.entitlements" "com.apple.security.network.client"
require_not_contains "$HELPER_ENTITLEMENTS" "macos/idrive-helper.entitlements" "com.apple.security.network.server"
require_not_contains "$DEV_APP" "scripts/macos-dev-app.sh" "        return 0"
require_not_contains "$DEV_APP" "scripts/macos-dev-app.sh" 'app_path="$(build_app)"'
require_contains "$DEV_APP" "scripts/macos-dev-app.sh" 'app_base_dir="$BUILD_DIR/AppData"'
require_not_contains "$DEV_APP" "scripts/macos-dev-app.sh" '&& "$mode" != "development"'
require_contains "$DEV_APP" "scripts/macos-dev-app.sh" "ensure_daemon_service"
require_contains "$DEV_APP" "scripts/macos-dev-app.sh" "service install --launch --json"
require_not_contains "$APP" "macos/Sources/IrisDriveMacApp.swift" '"--no-gateway"'
require_contains "$APP" "macos/Sources/IrisDriveMacApp.swift" "startAppManagedDaemon"
require_contains "$APP" "macos/Sources/IrisDriveMacApp.swift" "status.primaryDriveGatewayURL = nil"
require_contains "$ROOT/macos/Sources/IrisDriveDaemonService.swift" "macos/Sources/IrisDriveDaemonService.swift" "macOS app sandbox cannot install LaunchAgents directly"
require_contains "$ROOT/macos/Sources/IrisDriveDaemonService.swift" "macos/Sources/IrisDriveDaemonService.swift" 'currentProcessHasEntitlement("com.apple.security.app-sandbox")'
require_contains "$ROOT/macos/Sources/IrisDriveDaemonService.swift" "macos/Sources/IrisDriveDaemonService.swift" 'arguments: ["service", "status", "--json"]'
require_contains "$ROOT/macos/Sources/IrisDriveControlPanel.swift" "macos/Sources/IrisDriveControlPanel.swift" "Open on drive.iris.to"
require_not_contains "$ROOT/macos/Sources/IrisDriveControlPanel.swift" "macos/Sources/IrisDriveControlPanel.swift" "return shareLocalGatewayLink(share, status: status)"
require_contains "$DAEMON_RUNTIME" "crates/iris-drive-cli/src/daemon/runtime.rs" "embedded_hashtree_requested"
require_contains "$DAEMON_RUNTIME" "crates/iris-drive-cli/src/daemon/runtime.rs" '"requested": embedded_hashtree_requested'

echo "MACOS_FILEPROVIDER_LIFECYCLE_OK"
