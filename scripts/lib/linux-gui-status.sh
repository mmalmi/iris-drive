# shellcheck shell=bash
# shellcheck disable=SC2154

status_matches_expected() {
  local status roster="{}"
  if [[ "$expected_state" == approval_queued ]]; then
    [[ "$approval_submitted" == 1 && -n "$expected_app_key_npub" ]] || return 1
    python3 "$repo/scripts/lib/linux-approve-device.py" --completed "$app_pid" || return 1
    roster="$("$idrive" --config-dir "$config_dir" app-keys list 2>/dev/null)" || return 1
  fi
  status="$("$idrive" --config-dir "$config_dir" status 2>/dev/null)" || return 1
  STATUS_JSON="$status" ROSTER_JSON="$roster" EXPECTED_STATE="$expected_state" \
    EXPECTED_APP_KEY_NPUB="$expected_app_key_npub" python3 - <<'PY'
import json
import os

status = json.loads(os.environ["STATUS_JSON"])
expected = os.environ["EXPECTED_STATE"]
profile = status.get("profile") or {}
if not status.get("initialized"):
    raise SystemExit(1)
if expected == "awaiting_approval":
    request = profile.get("app_key_link_request") or {}
    if profile.get("authorization_state") != "awaiting_approval":
        raise SystemExit(1)
    if not str(request.get("url") or "").startswith("https://drive.iris.to/approve-device/"):
        raise SystemExit(1)
elif expected == "approval_queued":
    if profile.get("authorization_state") != "authorized":
        raise SystemExit(1)
    actors = (json.loads(os.environ["ROSTER_JSON"]).get("app_keys") or {}).get("app_actors") or []
    if not any(actor.get("npub") == os.environ["EXPECTED_APP_KEY_NPUB"]
               and actor.get("has_dck_wrap") is True for actor in actors):
        raise SystemExit(1)
else:
    summary = status.get("summary") or {}
    network = status.get("network") or {}
    authorized = int(
        summary.get("authorized_app_key_count")
        or summary.get("authorized_device_count")
        or network.get("authorized_app_key_count")
        or network.get("authorized_device_count")
        or 0
    )
    if profile.get("authorization_state") != "authorized" or authorized < 1:
        raise SystemExit(1)
PY
}
