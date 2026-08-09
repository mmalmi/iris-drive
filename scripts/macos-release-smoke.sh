#!/usr/bin/env bash

set -Eeuo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
APP_PATH=""
ARCHIVE_PATH=""
DMG_PATH=""
ARTIFACT_ROOT="${IRIS_DRIVE_MACOS_RELEASE_SMOKE_ARTIFACT_ROOT:-$ROOT/target/release-smoke}"
STARTUP_TIMEOUT_SECONDS="${IRIS_DRIVE_MACOS_RELEASE_SMOKE_STARTUP_TIMEOUT_SECONDS:-30}"
ALIVE_SECONDS="${IRIS_DRIVE_MACOS_RELEASE_SMOKE_ALIVE_SECONDS:-3}"
RESULT_PATH="$ARTIFACT_ROOT/macos-release-smoke.json"
WORK_DIR=""
DMG_MOUNT=""
LAUNCHED_PIDS=()
RELAUNCH_APP_PATHS=()
REGISTERED_APP_PATHS=()
LSREGISTER="/System/Library/Frameworks/CoreServices.framework/Frameworks/LaunchServices.framework/Support/lsregister"

usage() {
  cat <<'USAGE'
Usage: scripts/macos-release-smoke.sh --app <Iris Drive.app> --archive <app.tar.gz> --dmg <dmg>

Validates notarized macOS release artifacts and launches the app bundles through
LaunchServices with isolated runtime state. This catches releases that pass
codesign/stapler checks but fail at spawn time.
USAGE
}

log() {
  printf '[macos-release-smoke] %s\n' "$*" >&2
}

run() {
  log "$*"
  "$@"
}

json_result() {
  local ok="$1"
  local error="${2:-}"
  mkdir -p "$(dirname "$RESULT_PATH")"
  OK="$ok" ERROR_TEXT="$error" RESULT_PATH="$RESULT_PATH" APP_PATH="$APP_PATH" \
    ARCHIVE_PATH="$ARCHIVE_PATH" DMG_PATH="$DMG_PATH" python3 <<'PY'
import json
import os
from datetime import datetime, timezone

result = {
    "ok": os.environ["OK"] == "true",
    "error": os.environ.get("ERROR_TEXT", ""),
    "appPath": os.environ.get("APP_PATH", ""),
    "archivePath": os.environ.get("ARCHIVE_PATH", ""),
    "dmgPath": os.environ.get("DMG_PATH", ""),
    "generatedAt": datetime.now(timezone.utc).strftime("%Y-%m-%dT%H:%M:%SZ"),
}
with open(os.environ["RESULT_PATH"], "w", encoding="utf-8") as handle:
    json.dump(result, handle, indent=2)
    handle.write("\n")
PY
}

fail() {
  local message="$1"
  json_result false "$message"
  log "failed: $message"
  exit 1
}

cleanup() {
  stop_launched_apps
  if [[ -x "$LSREGISTER" ]]; then
    for app in "${REGISTERED_APP_PATHS[@]:-}"; do
      "$LSREGISTER" -u "$app" >/dev/null 2>&1 || true
    done
  fi
  if [[ -n "$DMG_MOUNT" ]]; then
    hdiutil detach "$DMG_MOUNT" -quiet >/dev/null 2>&1 || true
  fi
  if [[ -n "$WORK_DIR" ]]; then
    rm -rf "$WORK_DIR" >/dev/null 2>&1 || true
  fi
  for app in "${RELAUNCH_APP_PATHS[@]:-}"; do
    if [[ -d "$app" ]]; then
      log "restoring app that was running before release smoke: $app"
      open "$app" >/dev/null 2>&1 || true
    fi
  done
}
trap cleanup EXIT

stop_launched_apps() {
  for pid in "${LAUNCHED_PIDS[@]:-}"; do
    kill "$pid" >/dev/null 2>&1 || true
  done
  for _ in {1..20}; do
    local any_alive=0
    for pid in "${LAUNCHED_PIDS[@]:-}"; do
      if kill -0 "$pid" >/dev/null 2>&1; then
        any_alive=1
      fi
    done
    [[ "$any_alive" -eq 0 ]] && break
    sleep 0.1
  done
  for pid in "${LAUNCHED_PIDS[@]:-}"; do
    if kill -0 "$pid" >/dev/null 2>&1; then
      log "force-stopping isolated release-smoke pid $pid"
      kill -KILL "$pid" >/dev/null 2>&1 || true
    fi
  done
  LAUNCHED_PIDS=()
}

while [[ $# -gt 0 ]]; do
  case "$1" in
    --app)
      APP_PATH="${2:-}"
      shift 2
      ;;
    --archive)
      ARCHIVE_PATH="${2:-}"
      shift 2
      ;;
    --dmg)
      DMG_PATH="${2:-}"
      shift 2
      ;;
    -h | --help)
      usage
      exit 0
      ;;
    *)
      usage >&2
      exit 2
      ;;
  esac
done

if [[ "$(uname -s)" != "Darwin" ]]; then
  fail "macOS release smoke requires macOS"
fi
[[ -d "$APP_PATH" ]] || fail "macOS app bundle not found: $APP_PATH"
[[ -f "$ARCHIVE_PATH" ]] || fail "macOS updater archive not found: $ARCHIVE_PATH"
[[ -f "$DMG_PATH" ]] || fail "macOS DMG not found: $DMG_PATH"

mkdir -p "$ARTIFACT_ROOT"
WORK_DIR="$(mktemp -d -t iris-drive-release-smoke.XXXXXX)"

bundle_executable() {
  local app="$1"
  /usr/libexec/PlistBuddy -c 'Print :CFBundleExecutable' "$app/Contents/Info.plist" 2>/dev/null \
    || basename "$app" .app
}

app_pids_for_path() {
  local app="$1"
  local executable="$2"
  pgrep -f "$app/Contents/MacOS/$executable" 2>/dev/null || true
}

stop_conflicting_apps() {
  local bundle_identifier executable_path app candidate_identifier
  bundle_identifier="$(/usr/libexec/PlistBuddy -c 'Print :CFBundleIdentifier' \
    "$APP_PATH/Contents/Info.plist")"

  while IFS= read -r pid; do
    [[ -n "$pid" ]] || continue
    executable_path="$(lsof -a -p "$pid" -d txt -Fn 2>/dev/null \
      | sed -n 's/^n//p' \
      | grep '/Contents/MacOS/' \
      | head -n 1 \
      || true)"
    [[ "$executable_path" == */Contents/MacOS/* ]] || continue
    app="${executable_path%%/Contents/MacOS/*}"
    [[ -f "$app/Contents/Info.plist" ]] || continue
    candidate_identifier="$(/usr/libexec/PlistBuddy -c 'Print :CFBundleIdentifier' \
      "$app/Contents/Info.plist" 2>/dev/null || true)"
    [[ "$candidate_identifier" == "$bundle_identifier" ]] || continue

    log "stopping same-bundle app before isolated release smoke: $app (pid $pid)"
    RELAUNCH_APP_PATHS+=("$app")
    kill "$pid" >/dev/null 2>&1 || true
  done < <(pgrep -f '/Contents/MacOS/Iris Drive($| )' 2>/dev/null || true)

  for _ in {1..50}; do
    local any_alive=0
    for app in "${RELAUNCH_APP_PATHS[@]:-}"; do
      executable_path="$(bundle_executable "$app")"
      if [[ -n "$(app_pids_for_path "$app" "$executable_path")" ]]; then
        any_alive=1
      fi
    done
    [[ "$any_alive" -eq 0 ]] && return
    sleep 0.1
  done

  for app in "${RELAUNCH_APP_PATHS[@]:-}"; do
    executable_path="$(bundle_executable "$app")"
    while IFS= read -r pid; do
      [[ -n "$pid" ]] || continue
      log "force-stopping same-bundle app before release smoke: $app (pid $pid)"
      kill -KILL "$pid" >/dev/null 2>&1 || true
    done < <(app_pids_for_path "$app" "$executable_path")
  done
}

signature_team_identifier() {
  codesign -dv --verbose=4 "$1" 2>&1 \
    | sed -n 's/^TeamIdentifier=//p' \
    | head -n 1
}

entitlement_value() {
  local bundle="$1"
  local key_path="$2"
  codesign -d --entitlements :- "$bundle" 2>/dev/null \
    | plutil -extract "$key_path" raw -o - - 2>/dev/null
}

verify_file_provider_contract() {
  local app="$1"
  local extension="$app/Contents/PlugIns/IrisDriveFileProvider.appex"
  [[ "$(/usr/libexec/PlistBuddy -c 'Print :CFBundleIdentifier' "$app/Contents/Info.plist" 2>/dev/null)" == "to.iris.drive.macos" ]] \
    || fail "macOS app has the wrong bundle identifier"
  [[ -d "$extension" ]] || fail "macOS app is missing its File Provider extension"
  [[ "$(/usr/libexec/PlistBuddy -c 'Print :CFBundleIdentifier' "$extension/Contents/Info.plist" 2>/dev/null)" == "to.iris.drive.macos.FileProvider" ]] \
    || fail "macOS File Provider has the wrong bundle identifier"

  local app_team
  local extension_team
  app_team="$(signature_team_identifier "$app")"
  extension_team="$(signature_team_identifier "$extension")"
  [[ -n "$app_team" && "$app_team" != "not set" ]] \
    || fail "macOS app is ad-hoc signed"
  [[ "$extension_team" == "$app_team" ]] \
    || fail "macOS app and File Provider signing teams differ"

  local expected_group="$app_team.to.iris.drive"
  [[ "$(entitlement_value "$app" 'com\.apple\.security\.app-sandbox')" == "true" ]] \
    || fail "macOS app is missing its sandbox entitlement"
  [[ "$(entitlement_value "$extension" 'com\.apple\.security\.app-sandbox')" == "true" ]] \
    || fail "macOS File Provider is missing its sandbox entitlement"
  [[ "$(entitlement_value "$app" 'com\.apple\.security\.application-groups.0')" == "$expected_group" ]] \
    || fail "macOS app is missing its File Provider app group"
  [[ "$(entitlement_value "$extension" 'com\.apple\.security\.application-groups.0')" == "$expected_group" ]] \
    || fail "macOS File Provider is missing its app group"
}

verify_embedded_license() {
  local app="$1"
  local embedded_license="$app/Contents/Resources/LICENSE"
  [[ -f "$ROOT/LICENSE" ]] || fail "project license is missing"
  [[ -f "$embedded_license" ]] \
    || fail "macOS app is missing its embedded license"
  /usr/bin/cmp -s "$ROOT/LICENSE" "$embedded_license" \
    || fail "macOS app embedded license does not match the project license"
}

verify_app_bundle() {
  local app="$1"
  local executable
  verify_embedded_license "$app"
  executable="$(bundle_executable "$app")"
  [[ -x "$app/Contents/MacOS/$executable" ]] \
    || fail "app executable is missing or not executable: $app/Contents/MacOS/$executable"
  run codesign --verify --deep --strict --verbose=2 "$app"
  verify_file_provider_contract "$app"
  run xcrun stapler validate "$app"
  run spctl --assess --type execute --verbose=2 "$app"
}

verify_dmg() {
  run codesign --verify --strict "$DMG_PATH"
  run xcrun stapler validate "$DMG_PATH"
  run spctl --assess --type open --context context:primary-signature --verbose=2 "$DMG_PATH"
}

extract_archive_app() {
  local extract_dir="$WORK_DIR/archive"
  mkdir -p "$extract_dir"
  run tar -xzf "$ARCHIVE_PATH" -C "$extract_dir"
  local app
  app="$(find "$extract_dir" -maxdepth 1 -name '*.app' -type d | head -n 1)"
  [[ -n "$app" ]] || fail "no .app bundle found in updater archive"
  printf '%s\n' "$app"
}

copy_dmg_app() {
  local attach_plist="$WORK_DIR/hdiutil-attach.plist"
  local info_plist="$WORK_DIR/hdiutil-info.plist"
  local existing_devices
  hdiutil info -plist >"$info_plist" || true
  existing_devices="$(DMG_PATH="$DMG_PATH" python3 - "$info_plist" <<'PY' || true
import os
import plistlib
import sys

target = os.path.realpath(os.environ["DMG_PATH"])
try:
    with open(sys.argv[1], "rb") as handle:
        plist = plistlib.load(handle)
except Exception:
    sys.exit(0)
for image in plist.get("images", []):
    image_path = image.get("image-path")
    if not image_path or os.path.realpath(image_path) != target:
        continue
    for entity in image.get("system-entities", []):
        dev = entity.get("dev-entry")
        if dev:
            print(dev)
            break
PY
)"
  for device in $existing_devices; do
    log "detaching existing DMG attachment $device"
    hdiutil detach "$device" -quiet >/dev/null 2>&1 || true
  done
  log "hdiutil attach $DMG_PATH -nobrowse -readonly -plist"
  if ! hdiutil attach "$DMG_PATH" -nobrowse -readonly -plist >"$attach_plist"; then
    fail "hdiutil attach failed for DMG"
  fi
  DMG_MOUNT="$(python3 - "$attach_plist" <<'PY'
import plistlib
import sys

with open(sys.argv[1], "rb") as handle:
    plist = plistlib.load(handle)
for entity in plist.get("system-entities", []):
    mount = entity.get("mount-point")
    if mount:
        print(mount)
        break
PY
)"
  [[ -n "$DMG_MOUNT" && -d "$DMG_MOUNT" ]] || fail "DMG did not report a mounted volume"
  local mounted_app
  mounted_app="$(find "$DMG_MOUNT" -maxdepth 2 -name '*.app' -type d | head -n 1)"
  [[ -n "$mounted_app" ]] || fail "no .app bundle found in DMG"
  local copy_dir="$WORK_DIR/dmg-copy"
  local copied_app="$copy_dir/$(basename "$mounted_app")"
  mkdir -p "$copy_dir"
  run ditto "$mounted_app" "$copied_app"
  printf '%s\n' "$copied_app"
}

launch_app() {
  local label="$1"
  local app="$2"
  local executable
  executable="$(bundle_executable "$app")"
  local data_dir="$WORK_DIR/$label-data"
  local debug_dir="$WORK_DIR/$label-logs"
  local stdout_log="$ARTIFACT_ROOT/$label-open.stdout.log"
  local stderr_log="$ARTIFACT_ROOT/$label-open.stderr.log"
  local debug_log="$debug_dir/macos-app-debug.log"
  mkdir -p "$data_dir" "$debug_dir"
  : >"$stdout_log"
  : >"$stderr_log"

  touch "$app"
  if [[ -x "$LSREGISTER" ]]; then
    "$LSREGISTER" -f -R -trusted "$app" >/dev/null 2>&1 || true
    REGISTERED_APP_PATHS+=("$app")
  fi

  log "Launching $label through LaunchServices: $app"
  if ! open --stdout "$stdout_log" --stderr "$stderr_log" \
      --env "IRIS_DRIVE_APP_BASE_DIR=$data_dir" \
      --env "IRIS_DRIVE_DEBUG_LOG_DIR=$debug_dir" \
      --env "IRIS_DRIVE_DISABLE_FILEPROVIDER=1" \
      --env "IRIS_DRIVE_DISABLE_LOGIN_AGENT_SYNC=true" \
      --env "IRIS_DRIVE_DISABLE_SINGLE_INSTANCE=1" \
      --env "IRIS_DRIVE_EXTERNAL_DAEMON=1" \
      -n "$app" --args --hidden; then
    log "open stdout:"
    sed 's/^/[macos-release-smoke:open:stdout] /' "$stdout_log" >&2 || true
    log "open stderr:"
    sed 's/^/[macos-release-smoke:open:stderr] /' "$stderr_log" >&2 || true
    fail "LaunchServices failed to open $label"
  fi

  local deadline=$((SECONDS + STARTUP_TIMEOUT_SECONDS))
  local pid=""
  while (( SECONDS < deadline )); do
    pid="$(app_pids_for_path "$app" "$executable" | head -n 1 || true)"
    if [[ -n "$pid" ]]; then
      break
    fi
    sleep 0.5
  done

  [[ -n "$pid" ]] || fail "$label exited before it could be observed running"
  if [[ -f "$debug_log" ]] \
    && grep -q 'Iris Drive FileProvider integration enabled=' "$debug_log"; then
    log "$label wrote startup debug log"
  else
    log "$label did not write startup debug log; continuing with process liveness"
  fi
  LAUNCHED_PIDS+=("$pid")

  local alive_until=$((SECONDS + ALIVE_SECONDS))
  while (( SECONDS < alive_until )); do
    kill -0 "$pid" >/dev/null 2>&1 || fail "$label exited during startup"
    sleep 0.5
  done
}

verify_app_bundle "$APP_PATH"
verify_dmg

archive_app="$(extract_archive_app)"
verify_app_bundle "$archive_app"
stop_conflicting_apps
launch_app "archive-app" "$archive_app"
stop_launched_apps

dmg_app="$(copy_dmg_app)"
verify_app_bundle "$dmg_app"
launch_app "dmg-app" "$dmg_app"
stop_launched_apps

json_result true ""
log "MACOS_RELEASE_SMOKE_OK"
printf 'MACOS_RELEASE_SMOKE_OK\n'
