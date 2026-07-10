#!/usr/bin/env bash

set -Eeuo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"

usage() {
  cat <<'EOF'
usage:
  IRIS_NATIVE_LAB_ALLOW_RESET=1 scripts/native_state_reset.sh ios-simulator \
    --udid <udid> [--bundle-id <id> ...] [--erase]
  IRIS_NATIVE_LAB_ALLOW_RESET=1 scripts/native_state_reset.sh macos-fileprovider \
    --domain-id <id> --display-name <name> [--state-dir <dedicated-temp-dir>]
  IRIS_NATIVE_LAB_ALLOW_RESET=1 scripts/native_state_reset.sh android \
    --serial <serial> --bundle-id <id> [--test-bundle-id <id>]

The reset is intentionally gated. Use it only while the matching resource is
reserved by scripts/native_lab.py. State directories must be dedicated lab
directories beneath a temporary directory.
EOF
}

require_reset_authority() {
  if [[ "${IRIS_NATIVE_LAB_ALLOW_RESET:-0}" != "1" ]]; then
    echo "native state reset requires IRIS_NATIVE_LAB_ALLOW_RESET=1" >&2
    exit 75
  fi
}

safe_remove_state_dir() {
  local path="$1"
  python3 - "$path" <<'PY'
import shutil
import sys
import tempfile
from pathlib import Path

target = Path(sys.argv[1]).expanduser().resolve()
roots = {Path(tempfile.gettempdir()).resolve(), Path("/private/tmp").resolve(), Path("/tmp").resolve()}
if not any(root == target or root in target.parents for root in roots):
    raise SystemExit(f"refusing to remove non-temporary lab state: {target}")
if target in roots:
    raise SystemExit(f"refusing to remove temporary root itself: {target}")
shutil.rmtree(target, ignore_errors=True)
PY
}

reset_ios_simulator() {
  local udid=""
  local erase=0
  local -a bundle_ids=()
  while [[ $# -gt 0 ]]; do
    case "$1" in
      --udid) udid="$2"; shift 2 ;;
      --bundle-id) bundle_ids+=("$2"); shift 2 ;;
      --erase) erase=1; shift ;;
      *) usage >&2; exit 2 ;;
    esac
  done
  [[ -n "$udid" ]] || { usage >&2; exit 2; }
  xcrun simctl list devices available --json | python3 -c \
    'import json,sys; u=sys.argv[1]; d=json.load(sys.stdin).get("devices",{}); raise SystemExit(0 if any(x.get("udid")==u and x.get("isAvailable") for xs in d.values() for x in xs) else 75)' \
    "$udid"
  xcrun simctl shutdown "$udid" >/dev/null 2>&1 || true
  if [[ "$erase" == "1" ]]; then
    xcrun simctl erase "$udid" || exit 75
  else
    local bundle_id
    for bundle_id in "${bundle_ids[@]}"; do
      xcrun simctl uninstall "$udid" "$bundle_id" >/dev/null 2>&1 || true
    done
  fi
  xcrun simctl boot "$udid" >/dev/null 2>&1 || true
  xcrun simctl bootstatus "$udid" -b >/dev/null || exit 75
  printf 'reset ios-simulator %s\n' "$udid"
}

reset_macos_fileprovider() {
  local domain_id=""
  local display_name=""
  local state_dir=""
  while [[ $# -gt 0 ]]; do
    case "$1" in
      --domain-id) domain_id="$2"; shift 2 ;;
      --display-name) display_name="$2"; shift 2 ;;
      --state-dir) state_dir="$2"; shift 2 ;;
      *) usage >&2; exit 2 ;;
    esac
  done
  [[ -n "$domain_id" && -n "$display_name" ]] || { usage >&2; exit 2; }
  [[ "$(uname -s)" == "Darwin" ]] || exit 75
  swift "$ROOT/scripts/remove_fileprovider_domain.swift" "$domain_id" "$display_name" || exit 75
  if [[ -n "$state_dir" ]]; then
    safe_remove_state_dir "$state_dir" || exit 75
  fi
  printf 'reset macos-fileprovider %s\n' "$domain_id"
}

reset_android() {
  local serial=""
  local bundle_id=""
  local test_bundle_id=""
  while [[ $# -gt 0 ]]; do
    case "$1" in
      --serial) serial="$2"; shift 2 ;;
      --bundle-id) bundle_id="$2"; shift 2 ;;
      --test-bundle-id) test_bundle_id="$2"; shift 2 ;;
      *) usage >&2; exit 2 ;;
    esac
  done
  [[ -n "$serial" && -n "$bundle_id" ]] || { usage >&2; exit 2; }
  adb -s "$serial" get-state 2>/dev/null | grep -qx device || exit 75
  adb -s "$serial" shell am force-stop "$bundle_id" >/dev/null 2>&1 || true
  adb -s "$serial" shell pm clear "$bundle_id" >/dev/null || exit 75
  if [[ -n "$test_bundle_id" ]]; then
    adb -s "$serial" shell am force-stop "$test_bundle_id" >/dev/null 2>&1 || true
    adb -s "$serial" shell pm clear "$test_bundle_id" >/dev/null 2>&1 || true
  fi
  printf 'reset android %s %s\n' "$serial" "$bundle_id"
}

require_reset_authority
case "${1:-}" in
  ios-simulator) shift; reset_ios_simulator "$@" ;;
  macos-fileprovider) shift; reset_macos_fileprovider "$@" ;;
  android) shift; reset_android "$@" ;;
  *) usage >&2; exit 2 ;;
esac
