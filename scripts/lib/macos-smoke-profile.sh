#!/usr/bin/env bash

macos_smoke_app_group_identifier() {
  codesign -d --entitlements :- "$1" 2>/dev/null | python3 -c '
import plistlib, sys
raw = sys.stdin.buffer.read()
if not raw.strip():
    sys.exit(0)
entitlements = plistlib.loads(raw)
groups = entitlements.get("com.apple.security.application-groups") or []
if groups:
    print(groups[0])
elif entitlements.get("com.apple.security.app-sandbox"):
    sys.exit("sandboxed smoke app has no shared profile container")
'
}

macos_smoke_profile_parent() {
  local group
  group="$(macos_smoke_app_group_identifier "$1")" || return
  if [[ -n "$group" ]]; then
    printf '%s\n' "$HOME/Library/Group Containers/$group/Iris Drive Smoke"
  else
    printf '%s\n' "${TMPDIR:-/tmp}"
  fi
}
