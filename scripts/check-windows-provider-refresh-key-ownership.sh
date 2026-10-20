#!/usr/bin/env bash

set -Eeuo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
source "$ROOT/scripts/lib/source-assertions.sh"

require_contains crates/iris-drive-core/src/provider.rs "pub fn provider_refresh_key"
require_contains crates/iris-drive-cli/src/status.rs '"provider_refresh_key": provider_refresh_key'
require_contains windows/IrisDriveModels.cs 'String(ui, "provider_change_key")'
require_absent windows/IrisDriveModels.cs "BuildProviderRefreshKey"
require_absent windows/IrisDriveModels.cs 'ProviderRefreshKey = BuildProviderRefreshKey(root)'
require_absent windows/IrisDriveModels.cs 'String(summary.Value, "provider_refresh_key")'

echo "WINDOWS_PROVIDER_REFRESH_KEY_OWNERSHIP_OK"
