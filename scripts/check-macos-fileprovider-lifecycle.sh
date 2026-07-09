#!/usr/bin/env bash

set -Eeuo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
APP="$ROOT/macos/Sources/IrisDriveMacApp.swift"
STARTUP="$ROOT/macos/Sources/IrisDriveStartup.swift"
LIFECYCLE="$ROOT/macos/Sources/IrisDriveMacFileProvider.swift"
RUNTIME_SUPPORT="$ROOT/macos/Shared/IrisDriveRuntimeSupport.swift"
MACOS_INFO="$ROOT/macos/Info.plist"
FILEPROVIDER_ITEM="$ROOT/macos/FileProvider/FileProviderItem.swift"
FILEPROVIDER_INFO="$ROOT/macos/FileProvider/Info.plist"
HELPER_ENTITLEMENTS="$ROOT/macos/idrive-helper.entitlements"
MACOS_PROJECT="$ROOT/macos/project.yml"
DEV_APP="$ROOT/scripts/macos-dev-app.sh"
DAEMON_RUNTIME="$ROOT/crates/iris-drive-cli/src/daemon/runtime.rs"
DAEMON_GATEWAY_RUNTIME="$ROOT/crates/iris-drive-cli/src/daemon/gateway_runtime.rs"
PROVIDER_COMMANDS="$ROOT/crates/iris-drive-cli/src/commands.rs"

require_contains() {
  local file="$1"
  local label="$2"
  local needle="$3"
  if ! grep -Fq -- "$needle" "$file"; then
    echo "missing '$needle' in $label" >&2
    exit 1
  fi
}

require_not_contains() {
  local file="$1"
  local label="$2"
  local needle="$3"
  if grep -Fq -- "$needle" "$file"; then
    echo "unexpected '$needle' in $label" >&2
    exit 1
  fi
}

require_plist_raw_value() {
  local file="$1"
  local key="$2"
  local expected="$3"
  local actual
  actual="$(plutil -extract "$key" raw "$file" 2>/dev/null || true)"
  if [[ "$actual" != "$expected" ]]; then
    echo "expected plist $key in $file to be '$expected', got '${actual:-<missing>}'" >&2
    exit 1
  fi
}

require_contains "$APP" "macos/Sources/IrisDriveMacApp.swift" "ensureFileProviderDomainAfterStatusIfNeeded"
require_contains "$APP" "macos/Sources/IrisDriveMacApp.swift" "removeFileProviderDomainRegistration("
require_contains "$APP" "macos/Sources/IrisDriveMacApp.swift" "openFileProviderURL(_ url: URL, selectingItem: Bool)"
require_contains "$APP" "macos/Sources/IrisDriveMacApp.swift" "activateFileViewerSelecting([url])"
require_contains "$APP" "macos/Sources/IrisDriveMacApp.swift" "repairFileProviderDomainForOpenIfNeeded"
require_contains "$APP" "macos/Sources/IrisDriveMacApp.swift" "Opening FileProvider roots as plain file URLs"
require_contains "$APP" "macos/Sources/IrisDriveMacApp.swift" "Iris Drive mounted drive folder revealed"
require_not_contains "$APP" "macos/Sources/IrisDriveMacApp.swift" "Iris Drive mounted drive folder opened"
require_contains "$APP" "macos/Sources/IrisDriveMacApp.swift" 'let irisDriveFileProviderDomainDisplayName = ""'
require_contains "$APP" "macos/Sources/IrisDriveMacApp.swift" "irisDriveUserFacingDriveName"
require_not_contains "$APP" "macos/Sources/IrisDriveMacApp.swift" "selectFile(nil, inFileViewerRootedAtPath:"
require_contains "$MACOS_INFO" "macos/Info.plist" "NSLocalNetworkUsageDescription"
require_contains "$MACOS_INFO" "macos/Info.plist" "Find and connect directly to nearby Iris Drive devices on your local network."
require_contains "$MACOS_INFO" "macos/Info.plist" "NSBonjourServices"
require_contains "$MACOS_INFO" "macos/Info.plist" "_fips._udp"
require_contains "$MACOS_INFO" "macos/Info.plist" "NSAllowsLocalNetworking"
require_contains "$FILEPROVIDER_ITEM" "macos/FileProvider/FileProviderItem.swift" 'filename: "Iris Drive"'
require_contains "$FILEPROVIDER_ITEM" "macos/FileProvider/FileProviderItem.swift" "rootCid"
require_contains "$FILEPROVIDER_ITEM" "macos/FileProvider/FileProviderItem.swift" "--base-root-cid"
require_contains "$FILEPROVIDER_ITEM" "macos/FileProvider/FileProviderItem.swift" '"provider", "compose-path"'
require_contains "$FILEPROVIDER_ITEM" "macos/FileProvider/FileProviderItem.swift" "mutationBaseRootCid"
require_contains "$PROVIDER_COMMANDS" "crates/iris-drive-cli/src/commands.rs" "base_root_cid"
require_contains "$PROVIDER_COMMANDS" "crates/iris-drive-cli/src/commands.rs" "ComposePath"
require_contains "$MACOS_PROJECT" "macos/project.yml" "IrisDriveFileProvider:"
require_contains "$FILEPROVIDER_INFO" "macos/FileProvider/Info.plist" "CFBundleIcons"
require_contains "$FILEPROVIDER_INFO" "macos/FileProvider/Info.plist" "CFBundleSymbolName"
require_contains "$FILEPROVIDER_INFO" "macos/FileProvider/Info.plist" "smallcircle.filled.circle"
require_plist_raw_value "$FILEPROVIDER_INFO" "CFBundleSymbolName" "smallcircle.filled.circle"
require_contains "$LIFECYCLE" "macos/Sources/IrisDriveMacFileProvider.swift" "irisDriveFileProviderRegistrationIdentityKey"
require_contains "$LIFECYCLE" "macos/Sources/IrisDriveMacFileProvider.swift" "currentFileProviderProfileRegistrationIdentity"
require_contains "$LIFECYCLE" "macos/Sources/IrisDriveMacFileProvider.swift" "currentFileProviderAppRegistrationIdentity"
require_contains "$LIFECYCLE" "macos/Sources/IrisDriveMacFileProvider.swift" "Bundle.main"
require_contains "$LIFECYCLE" "macos/Sources/IrisDriveMacFileProvider.swift" "bundleURL.standardizedFileURL.resolvingSymlinksInPath"
require_contains "$LIFECYCLE" "macos/Sources/IrisDriveMacFileProvider.swift" "builtInPlugInsURL"
require_contains "$LIFECYCLE" "macos/Sources/IrisDriveMacFileProvider.swift" "IrisDriveFileProvider.appex"
require_contains "$LIFECYCLE" "macos/Sources/IrisDriveMacFileProvider.swift" "CFBundleVersion"
require_contains "$LIFECYCLE" "macos/Sources/IrisDriveMacFileProvider.swift" "markFileProviderRegistrationCurrent"
require_contains "$LIFECYCLE" "macos/Sources/IrisDriveMacFileProvider.swift" "fileProviderRegistrationIdentityIsCurrent"
require_contains "$LIFECYCLE" "macos/Sources/IrisDriveMacFileProvider.swift" "queryFileProviderDomainStateWithError"
require_contains "$LIFECYCLE" "macos/Sources/IrisDriveMacFileProvider.swift" "domain query failed after add"
require_contains "$LIFECYCLE" "macos/Sources/IrisDriveMacFileProvider.swift" "shouldRepairFileProviderRegistration"
require_contains "$LIFECYCLE" "macos/Sources/IrisDriveMacFileProvider.swift" "repairFileProviderRegistration"
require_contains "$LIFECYCLE" "macos/Sources/IrisDriveMacFileProvider.swift" "removeAllDomains"
require_contains "$LIFECYCLE" "macos/Sources/IrisDriveMacFileProvider.swift" "resetAllFileProviderDomains"
require_contains "$LIFECYCLE" "macos/Sources/IrisDriveMacFileProvider.swift" "repairAllFileProviderRegistrations"
require_contains "$LIFECYCLE" "macos/Sources/IrisDriveMacFileProvider.swift" "addFreshFileProviderDomain"
require_contains "$LIFECYCLE" "macos/Sources/IrisDriveMacFileProvider.swift" "Iris Drive repairing orphaned FileProvider domains"
require_contains "$LIFECYCLE" "macos/Sources/IrisDriveMacFileProvider.swift" "Iris Drive repairing stale FileProvider domain registration"
require_contains "$LIFECYCLE" "macos/Sources/IrisDriveMacFileProvider.swift" "Iris Drive FileProvider domain removed without re-add"
require_contains "$RUNTIME_SUPPORT" "macos/Shared/IrisDriveRuntimeSupport.swift" "existingAppGroupApplicationSupportDirectory"
require_contains "$RUNTIME_SUPPORT" "macos/Shared/IrisDriveRuntimeSupport.swift" 'hasSuffix(".to.iris.drive")'
require_contains "$RUNTIME_SUPPORT" "macos/Shared/IrisDriveRuntimeSupport.swift" "Config"
require_contains "$RUNTIME_SUPPORT" "macos/Shared/IrisDriveRuntimeSupport.swift" "config.toml"
require_contains "$HELPER_ENTITLEMENTS" "macos/idrive-helper.entitlements" "com.apple.security.inherit"
require_not_contains "$HELPER_ENTITLEMENTS" "macos/idrive-helper.entitlements" "com.apple.security.app-sandbox"
require_not_contains "$HELPER_ENTITLEMENTS" "macos/idrive-helper.entitlements" "com.apple.security.network.client"
require_not_contains "$HELPER_ENTITLEMENTS" "macos/idrive-helper.entitlements" "com.apple.security.network.server"
require_not_contains "$DEV_APP" "scripts/macos-dev-app.sh" "        return 0"
require_not_contains "$DEV_APP" "scripts/macos-dev-app.sh" 'app_path="$(build_app)"'
require_contains "$DEV_APP" "scripts/macos-dev-app.sh" "development_app_group_config_dir"
require_contains "$DEV_APP" "scripts/macos-dev-app.sh" "remove_daemon_service"
require_contains "$DEV_APP" "scripts/macos-dev-app.sh" "remove_repo_local_daemon_service"
require_contains "$DEV_APP" "scripts/macos-dev-app.sh" "app-group runtime is managed by sandboxed app"
require_contains "$DEV_APP" "scripts/macos-dev-app.sh" 'if [[ -z "$app_base_dir" && "$mode" != "development" ]]; then'
require_contains "$DEV_APP" "scripts/macos-dev-app.sh" "Removing stale macOS FileProvider pluginkit registration"
require_not_contains "$DEV_APP" "scripts/macos-dev-app.sh" "candidate_plugins"
require_not_contains "$DEV_APP" "scripts/macos-dev-app.sh" '$HOME/Applications/Iris Drive.app/Contents/PlugIns/IrisDriveFileProvider.appex'
require_not_contains "$DEV_APP" "scripts/macos-dev-app.sh" "/Applications/Iris Drive.app/Contents/PlugIns/IrisDriveFileProvider.appex"
require_contains "$DEV_APP" "scripts/macos-dev-app.sh" "registered_plugins"
require_contains "$DEV_APP" "scripts/macos-dev-app.sh" 'pluginkit -r "$plugin"'
require_contains "$DEV_APP" "scripts/macos-dev-app.sh" "Reusing existing macOS FileProvider pluginkit registration"
require_contains "$DEV_APP" "scripts/macos-dev-app.sh" "ensure_daemon_service"
require_contains "$DEV_APP" "scripts/macos-dev-app.sh" "service install --launch --json"
require_contains "$DEV_APP" "scripts/macos-dev-app.sh" "macos_process_command_matches"
require_contains "$DEV_APP" "scripts/macos-dev-app.sh" "default_install_app_path"
require_contains "$DEV_APP" "scripts/macos-dev-app.sh" "/Applications/Iris Drive.app"
require_contains "$DEV_APP" "scripts/macos-dev-app.sh" "macos/.build/Applications/Iris Drive.app"
require_contains "$DEV_APP" "scripts/macos-dev-app.sh" "IRIS_DRIVE_DISABLE_LOGIN_AGENT_SYNC=true"
require_contains "$DEV_APP" "scripts/macos-dev-app.sh" "IRIS_DRIVE_FILEPROVIDER_RESET_ON_START=true"
require_not_contains "$DEV_APP" "scripts/macos-dev-app.sh" 'pkill -TERM -x "$APP_PROCESS_NAME"'
require_not_contains "$DEV_APP" "scripts/macos-dev-app.sh" 'pkill -x "$APP_PROCESS_NAME"'
require_contains "$STARTUP" "macos/Sources/IrisDriveStartup.swift" "launchAgentSyncDisabled"
require_contains "$STARTUP" "macos/Sources/IrisDriveStartup.swift" "IRIS_DRIVE_DISABLE_LOGIN_AGENT_SYNC"
require_not_contains "$APP" "macos/Sources/IrisDriveMacApp.swift" '"--no-gateway"'
require_contains "$APP" "macos/Sources/IrisDriveMacApp.swift" "startAppManagedDaemon"
require_contains "$APP" "macos/Sources/IrisDriveMacApp.swift" "irisDriveAppManagedDaemonStatusRefreshMinimumInterval"
require_contains "$APP" "macos/Sources/IrisDriveMacApp.swift" "lastAppManagedDaemonStatusRefreshAt"
require_contains "$APP" "macos/Sources/IrisDriveMacApp.swift" "Iris Drive app-managed daemon status file refresh"
require_contains "$APP" "macos/Sources/IrisDriveMacApp.swift" "status.primaryDriveGatewayURL = nil"
require_contains "$APP" "macos/Sources/IrisDriveMacApp.swift" "self.daemonServiceActive = serviceRunning"
require_contains "$ROOT/macos/Sources/IrisDriveControlPanel.swift" "macos/Sources/IrisDriveControlPanel.swift" "Daemon offline"
require_contains "$ROOT/macos/Sources/IrisDriveControlPanel.swift" "macos/Sources/IrisDriveControlPanel.swift" "if status.localNhashResolverEnabled"
require_contains "$ROOT/macos/Sources/IrisDriveControlPanel.swift" "macos/Sources/IrisDriveControlPanel.swift" 'Label("Open Iris Apps", systemImage: "safari")'
require_contains "$ROOT/macos/Sources/IrisDriveControlPanel.swift" "macos/Sources/IrisDriveControlPanel.swift" ".disabled(status.sitesPortalURL == nil)"
require_contains "$ROOT/macos/Sources/IrisDriveDaemonService.swift" "macos/Sources/IrisDriveDaemonService.swift" "macOS app sandbox cannot install LaunchAgents directly"
require_contains "$ROOT/macos/Sources/IrisDriveDaemonService.swift" "macos/Sources/IrisDriveDaemonService.swift" 'currentProcessHasEntitlement("com.apple.security.app-sandbox")'
require_contains "$ROOT/macos/Sources/IrisDriveDaemonService.swift" "macos/Sources/IrisDriveDaemonService.swift" 'arguments: ["service", "status", "--json"]'
require_contains "$ROOT/macos/Sources/IrisDriveControlPanel.swift" "macos/Sources/IrisDriveControlPanel.swift" "Open on drive.iris.to"
require_not_contains "$ROOT/macos/Sources/IrisDriveControlPanel.swift" "macos/Sources/IrisDriveControlPanel.swift" "return shareLocalGatewayLink(share, status: status)"
require_contains "$DAEMON_RUNTIME" "crates/iris-drive-cli/src/daemon/runtime.rs" "embedded_hashtree_requested"
require_contains "$DAEMON_GATEWAY_RUNTIME" "crates/iris-drive-cli/src/daemon/gateway_runtime.rs" '"requested": embedded_hashtree_requested'

echo "MACOS_FILEPROVIDER_LIFECYCLE_OK"
