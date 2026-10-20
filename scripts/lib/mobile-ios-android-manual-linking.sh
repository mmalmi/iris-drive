#!/usr/bin/env bash

# Manual-entry half of the physical iOS/Android linking matrix. This file is
# sourced by mobile-ios-android-linking-e2e.sh after its shared device drivers.

app_key_link_request_from_config() {
  python3 - "$1" <<'PY'
import sys
import tomllib

with open(sys.argv[1], "rb") as handle:
    config = tomllib.load(handle)
request = (
    ((config.get("profile") or {}).get("outbound_app_key_link_request") or {})
    .get("request_url", "")
    .strip()
)
if not request:
    raise SystemExit(1)
print(request)
PY
}

wait_for_app_key_link_request() {
  local platform="$1"
  local seconds="$2"
  local config="$TMP/$platform-manual-request.toml"
  local deadline=$((SECONDS + seconds))
  local request
  while ((SECONDS < deadline)); do
    rm -f "$config"
    if [[ "$platform" == "ios" ]]; then
      copy_ios_config "$config" || true
    else
      copy_android_config "$config" || true
    fi
    if [[ -s "$config" ]] && request="$(app_key_link_request_from_config "$config")"; then
      printf '%s\n' "$request"
      return 0
    fi
    sleep 0.25
  done
  return 1
}

open_android_manual_approval() {
  wait_and_tap_android_text "Devices" 10 || fail "Android Devices tab was unavailable"
  if ! wait_for_android_ui text "Add Device" 3; then
    "$ADB" -s "$ANDROID_SERIAL_SELECTED" shell input swipe 540 1800 540 600 350 >/dev/null
  fi
  wait_and_tap_android_text "Add Device" 10 || fail "Android Add Device panel was unavailable"
  wait_for_android_ui desc "Manual device approval request" 5 \
    || fail "Android shipped manual approval field was unavailable"
  echo "IRIS_ANDROID_MANUAL_ENTRY_READY=1"
}

type_android_manual_approval_request() {
  local request="$1"
  [[ "$request" =~ ^[A-Za-z0-9:/._-]+$ ]] \
    || fail "production approval request contains characters unsafe for Android keyboard input"
  tap_android_ui desc "Manual device approval request" \
    || fail "Android manual approval field could not be focused"
  "$ADB" -s "$ANDROID_SERIAL_SELECTED" shell input text "$request" >/dev/null
  wait_for_android_ui text "Approve this device?" 15 \
    || fail "Android did not recognize iOS's complete manually entered request"
  echo "IRIS_ANDROID_MANUAL_REQUEST_ENTERED=1"
}

run_ios_owner_android_joiner_manual() {
  echo "[mobile-link] manual direction iOS owner -> Android joiner" >&2
  display_android_join_request_qr
  local request android_config="$TMP/android-manual-joiner-config.toml"
  request="$(wait_for_app_key_link_request android 15)" \
    || fail "could not read Android's production-generated approval request"
  start_ios_test \
    testIosOwnerApprovesAndroidThroughManualEntry \
    "$IOS_MANUAL_OWNER_FILE" \
    "$IOS_MANUAL_OWNER_CONTENT" \
    "$ANDROID_MANUAL_JOINER_FILE" \
    "$request"
  wait_for_ios_marker "IRIS_IOS_MANUAL_ENTRY_READY=1" 30 \
    || fail "iOS shipped manual approval field did not become ready"
  wait_for_ios_marker "IRIS_IOS_LIFECYCLE_RESUMED=1" 2 \
    || fail "iOS manual owner state did not survive background/restart coverage"
  wait_for_ios_marker "IRIS_IOS_MANUAL_REQUEST_ENTERED=1" 15 \
    || fail "iOS did not type Android's complete production approval request"
  wait_for_ios_marker "IRIS_IOS_MANUAL_CONFIRMATION_READY=1" 5 \
    || fail "iOS did not expose explicit confirmation for the manual request"

  IOS_TO_ANDROID_MANUAL_STARTED_MS="$(monotonic_milliseconds)"
  signal_ios_test submit-manual-approval \
    || fail "could not release iOS manual approval after starting the host timer"
  wait_for_ios_marker "IRIS_IOS_MANUAL_APPROVAL_SUBMITTED=1" 5 \
    || fail "iOS did not explicitly confirm the manually entered request"
  wait_for_android_authorized "$WAIT_SECS" \
    || fail "Android stayed on Waiting for approval after iOS manual confirmation"
  IOS_TO_ANDROID_MANUAL_FINISHED_MS="$(monotonic_milliseconds)"
  IOS_TO_ANDROID_MANUAL_MS="$(assert_delivery_bound \
    "manual iOS-to-Android" \
    "$IOS_TO_ANDROID_MANUAL_STARTED_MS" \
    "$IOS_TO_ANDROID_MANUAL_FINISHED_MS")"

  restart_android_and_assert_authorized
  exchange_post_link_provider_files \
    "$IOS_MANUAL_OWNER_FILE" "$IOS_MANUAL_OWNER_CONTENT" \
    "$ANDROID_MANUAL_JOINER_FILE" "$ANDROID_MANUAL_JOINER_CONTENT"
  wait_for_empty_receipts ios "$WAIT_SECS" \
    || fail "iOS manual owner retained its durable approval receipt after Android applied it"
  finish_ios_test testIosOwnerApprovesAndroidThroughManualEntry
  copy_android_config "$android_config" || fail "could not read Android manual joiner config"
  assert_authorized_config "$android_config" \
    || fail "Android did not persist manual-link authorization"
  assert_ios_provider_content "$ANDROID_MANUAL_JOINER_FILE" "$ANDROID_MANUAL_JOINER_CONTENT" \
    || fail "iOS provider content did not match Android's manual-link write"
}

run_android_owner_ios_joiner_manual() {
  echo "[mobile-link] manual direction Android owner -> iOS joiner" >&2
  create_android_owner_through_ui
  start_ios_test \
    testIosJoinerWaitsForAndroidManualApproval \
    "$IOS_MANUAL_JOINER_FILE" \
    "$IOS_MANUAL_JOINER_CONTENT" \
    "$ANDROID_MANUAL_OWNER_FILE"
  wait_for_ios_marker "IRIS_IOS_MANUAL_REQUEST_READY=1" 30 \
    || fail "iOS shipped UI did not generate its manual approval request"
  wait_for_ios_marker "IRIS_IOS_LIFECYCLE_RESUMED=1" 2 \
    || fail "iOS manual request did not survive background and resume"

  local request ios_config="$TMP/ios-manual-joiner-config.toml"
  request="$(wait_for_app_key_link_request ios 15)" \
    || fail "could not read iOS's production-generated approval request"
  open_android_manual_approval
  type_android_manual_approval_request "$request"

  ANDROID_TO_IOS_MANUAL_STARTED_MS="$(monotonic_milliseconds)"
  tap_android_ui text "Approve" || fail "Android manual approval confirmation could not be tapped"
  wait_for_ios_marker "IRIS_IOS_MANUAL_AUTHORIZED=1" "$WAIT_SECS" \
    || fail "iOS stayed on Waiting for approval after Android manual confirmation"
  ANDROID_TO_IOS_MANUAL_FINISHED_MS="$(monotonic_milliseconds)"
  ANDROID_TO_IOS_MANUAL_MS="$(assert_delivery_bound \
    "manual Android-to-iOS" \
    "$ANDROID_TO_IOS_MANUAL_STARTED_MS" \
    "$ANDROID_TO_IOS_MANUAL_FINISHED_MS")"

  restart_android_and_assert_authorized
  exchange_post_link_provider_files \
    "$IOS_MANUAL_JOINER_FILE" "$IOS_MANUAL_JOINER_CONTENT" \
    "$ANDROID_MANUAL_OWNER_FILE" "$ANDROID_MANUAL_OWNER_CONTENT"
  wait_for_empty_receipts android "$WAIT_SECS" \
    || fail "Android manual owner retained its durable approval receipt after iOS applied it"
  finish_ios_test testIosJoinerWaitsForAndroidManualApproval
  copy_ios_config "$ios_config" || fail "could not read iOS manual joiner config"
  assert_authorized_config "$ios_config" || fail "iOS did not persist manual-link authorization"
  assert_ios_provider_content "$ANDROID_MANUAL_OWNER_FILE" "$ANDROID_MANUAL_OWNER_CONTENT" \
    || fail "iOS provider content did not match Android's manual-link write"
}

write_mobile_linking_summary() {
  local summary="$1"
  python3 - \
    "$IOS_TO_ANDROID_MS" "$ANDROID_TO_IOS_MS" \
    "$IOS_TO_ANDROID_MANUAL_MS" "$ANDROID_TO_IOS_MANUAL_MS" \
    "$WAIT_SECS" \
    "$IOS_TO_ANDROID_STARTED_MS" "$IOS_TO_ANDROID_FINISHED_MS" \
    "$ANDROID_TO_IOS_STARTED_MS" "$ANDROID_TO_IOS_FINISHED_MS" \
    "$IOS_TO_ANDROID_MANUAL_STARTED_MS" "$IOS_TO_ANDROID_MANUAL_FINISHED_MS" \
    "$ANDROID_TO_IOS_MANUAL_STARTED_MS" "$ANDROID_TO_IOS_MANUAL_FINISHED_MS" \
    "$IOS_OWNER_CONTENT" "$IOS_JOINER_CONTENT" \
    "$ANDROID_OWNER_CONTENT" "$ANDROID_JOINER_CONTENT" \
    "$IOS_MANUAL_OWNER_CONTENT" "$IOS_MANUAL_JOINER_CONTENT" \
    "$ANDROID_MANUAL_OWNER_CONTENT" "$ANDROID_MANUAL_JOINER_CONTENT" \
    >"$summary" <<'PY'
import hashlib
import json
import sys

(
    ios_to_android,
    android_to_ios,
    ios_to_android_manual,
    android_to_ios_manual,
    ceiling,
    ios_to_android_started,
    ios_to_android_finished,
    android_to_ios_started,
    android_to_ios_finished,
    ios_to_android_manual_started,
    ios_to_android_manual_finished,
    android_to_ios_manual_started,
    android_to_ios_manual_finished,
) = map(int, sys.argv[1:14])
(
    ios_owner,
    ios_joiner,
    android_owner,
    android_joiner,
    ios_manual_owner,
    ios_manual_joiner,
    android_manual_owner,
    android_manual_joiner,
) = sys.argv[14:]

def digest(value: str) -> str:
    return hashlib.sha256(value.encode()).hexdigest()

print(json.dumps({
    "ok": True,
    "skipped": False,
    "physical_execution": True,
    "camera_transfer": "shipped UI only",
    "manual_entry_transfer": "complete production request typed through shipped UI",
    "qr_directions": ["ios-owner-to-android-joiner", "android-owner-to-ios-joiner"],
    "manual_directions": ["ios-owner-to-android-joiner", "android-owner-to-ios-joiner"],
    "manual_entry_ui_driven": {"ios_owner": True, "android_owner": True},
    "delivery_measurement_clock": "host_monotonic_ms",
    "delivery_ceiling_ms": ceiling * 1000,
    "ios_owner_to_android_joiner_ms": ios_to_android,
    "android_owner_to_ios_joiner_ms": android_to_ios,
    "ios_owner_to_android_joiner_manual_ms": ios_to_android_manual,
    "android_owner_to_ios_joiner_manual_ms": android_to_ios_manual,
    "delivery_intervals": {
        "ios_owner_to_android_joiner": {
            "start_ms": ios_to_android_started,
            "finish_ms": ios_to_android_finished,
            "start_checkpoint": "host_released_approval_submission",
            "finish_checkpoint": "host_observed_authorization",
        },
        "android_owner_to_ios_joiner": {
            "start_ms": android_to_ios_started,
            "finish_ms": android_to_ios_finished,
            "start_checkpoint": "host_began_approval_submission",
            "finish_checkpoint": "host_observed_authorization",
        },
        "ios_owner_to_android_joiner_manual": {
            "start_ms": ios_to_android_manual_started,
            "finish_ms": ios_to_android_manual_finished,
            "start_checkpoint": "host_released_approval_submission",
            "finish_checkpoint": "host_observed_authorization",
        },
        "android_owner_to_ios_joiner_manual": {
            "start_ms": android_to_ios_manual_started,
            "finish_ms": android_to_ios_manual_finished,
            "start_checkpoint": "host_began_approval_submission",
            "finish_checkpoint": "host_observed_authorization",
        },
    },
    "durable_applied_ack_cleared": True,
    "background_resume_and_restart": True,
    "post_link_provider_writes": {
        "ios_owner_to_android_joiner": [digest(ios_owner), digest(android_joiner)],
        "android_owner_to_ios_joiner": [digest(ios_joiner), digest(android_owner)],
        "ios_owner_to_android_joiner_manual": [
            digest(ios_manual_owner), digest(android_manual_joiner),
        ],
        "android_owner_to_ios_joiner_manual": [
            digest(ios_manual_joiner), digest(android_manual_owner),
        ],
    },
}, indent=2, sort_keys=True))
PY
}
