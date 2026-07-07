#!/usr/bin/env bash

iris_drive_ios_assert_plist_entitlement() {
  local plist="$1"
  local key="$2"
  local expected="$3"

  python3 - "$plist" "$key" "$expected" <<'PY'
import plistlib
import sys

path, key, expected = sys.argv[1:]
try:
    with open(path, "rb") as handle:
        entitlements = plistlib.load(handle)
except Exception as error:
    raise SystemExit(f"missing simulator entitlements at {path}: {error}")

actual = entitlements.get(key)
if expected == "true":
    ok = actual is True
elif isinstance(actual, list):
    ok = expected in actual
else:
    ok = actual == expected

if not ok:
    raise SystemExit(
        f"{path} entitlement {key!r} is {actual!r}, expected {expected!r}"
    )
PY
}

iris_drive_ios_assert_simulator_entitlements() {
  local derived_data="$1"
  local configuration="${2:-Debug}"
  local expected_app_group="${IRIS_DRIVE_IOS_APP_GROUP_IDENTIFIER:-group.fi.siriusbusiness.drive}"
  local base="$derived_data/Build/Intermediates.noindex/IrisDriveIOS.build/${configuration}-iphonesimulator"
  local app="$base/IrisDriveIOS.build/Iris Drive.app-Simulated.xcent"
  local fileprovider="$base/IrisDriveFileProvider.build/IrisDriveFileProvider.appex-Simulated.xcent"
  local share="$base/IrisDriveShareExtension.build/IrisDriveShareExtension.appex-Simulated.xcent"

  iris_drive_ios_assert_plist_entitlement \
    "$app" \
    "com.apple.security.application-groups" \
    "$expected_app_group"
  iris_drive_ios_assert_plist_entitlement \
    "$fileprovider" \
    "com.apple.security.application-groups" \
    "$expected_app_group"
  iris_drive_ios_assert_plist_entitlement \
    "$share" \
    "com.apple.security.application-groups" \
    "$expected_app_group"

  if [[ "$configuration" == "Debug" ]]; then
    iris_drive_ios_assert_plist_entitlement \
      "$app" \
      "com.apple.developer.fileprovider.testing-mode" \
      "true"
    iris_drive_ios_assert_plist_entitlement \
      "$fileprovider" \
      "com.apple.developer.fileprovider.testing-mode" \
      "true"
  fi
}
