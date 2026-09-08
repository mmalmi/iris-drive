# shellcheck shell=bash
# shellcheck disable=SC2154

DESKTOP_GUI_AUX_CONFIG=""
DESKTOP_GUI_AUX_PIDFILE=""
DESKTOP_GUI_AUX_NPUB=""
DESKTOP_GUI_REVERSE_FILE=""
DESKTOP_GUI_REVERSE_FILE_SHA=""
DESKTOP_GUI_PRIMARY_LINK_STARTED_AT=""
DESKTOP_GUI_PRIMARY_LINK_DEADLINE=""

monotonic_milliseconds() {
  perl -MTime::HiRes=clock_gettime,CLOCK_MONOTONIC \
    -e 'printf "%.0f\n", clock_gettime(CLOCK_MONOTONIC) * 1000'
}

wait_until_before() {
  local label="$1" deadline="$2" check="$3" start="${4:-$(monotonic_milliseconds)}" now
  while true; do
    now="$(monotonic_milliseconds)"
    if (( now > deadline )); then
      echo "timed out after $((deadline - start))ms waiting for $label" >&2
      print_statuses
      return 1
    fi
    if "$check"; then
      now="$(monotonic_milliseconds)"
      if (( now <= deadline )); then
        echo "ok: $label ($((now - start))ms)"
        return 0
      fi
      echo "late success after $((now - start))ms waiting for $label" >&2
      print_statuses
      return 1
    fi
    if (( now >= deadline )); then
      echo "timed out after $((deadline - start))ms waiting for $label" >&2
      print_statuses
      return 1
    fi
    sleep "$POLL_SECS"
  done
}

wait_until_for() {
  local start
  start="$(monotonic_milliseconds)"
  wait_until_before "$1" "$((start + $2 * 1000))" "$3" "$start"
}

wait_until() {
  wait_until_for "$1" "$TIMEOUT_SECS" "$2"
}

desktop_gui_target() {
  local label="$1"
  case "$(host_value "$label" kind):$label" in
    windows:*) printf 'windows' ;;
    posix:ubuntu* | posix:linux*) printf 'linux' ;;
    *) return 1 ;;
  esac
}

validate_desktop_gui_linking() {
  bool_true "$DESKTOP_GUI_LINKING" || return 0
  [[ -n "$windows_label" ]] || { echo "desktop GUI linking requires a Windows host" >&2; return 2; }
  [[ "$(desktop_gui_target "$owner_label" 2>/dev/null || true)" == "linux" ]] || {
    echo "desktop GUI linking requires the first/owner host to be Linux" >&2
    return 2
  }
  [[ "$SIDELOAD_APPKEYS" == "0" ]] || {
    echo "desktop GUI linking requires IRIS_DRIVE_E2E_SIDELOAD_APPKEYS=0" >&2
    return 2
  }
}

mark_desktop_gui_primary_approval_submission() {
  bool_true "$DESKTOP_GUI_LINKING" || return 0
  [[ -z "$DESKTOP_GUI_PRIMARY_LINK_STARTED_AT" ]] || return 0
  DESKTOP_GUI_PRIMARY_LINK_STARTED_AT="$(monotonic_milliseconds)"
  DESKTOP_GUI_PRIMARY_LINK_DEADLINE=$((DESKTOP_GUI_PRIMARY_LINK_STARTED_AT + LINK_TIMEOUT_SECS * 1000))
}

wait_for_all_linking_complete() {
  if bool_true "$DESKTOP_GUI_LINKING"; then
    [[ -n "$DESKTOP_GUI_PRIMARY_LINK_DEADLINE" ]] || {
      echo "desktop GUI approval deadline was not recorded" >&2
      return 1
    }
    wait_until_before \
      "all devices authorized with approval receipts drained" \
      "$DESKTOP_GUI_PRIMARY_LINK_DEADLINE" \
      all_linking_complete \
      "$DESKTOP_GUI_PRIMARY_LINK_STARTED_AT"
  else
    wait_until_for \
      "all devices authorized with approval receipts drained" \
      "$LINK_TIMEOUT_SECS" \
      all_linking_complete
  fi
}

run_desktop_gui_link_action_at() {
  local label="$1" config="$2" launch_link="$3" expected_state="$4" expected_app_key="${5:-}"
  local target host
  target="$(desktop_gui_target "$label")" || {
    echo "desktop GUI link action does not support host label $label" >&2
    return 1
  }
  host="$(host_value "$label" ssh)"
  echo "running shipped $target UI link action on $label ($expected_state)"
  if [[ "$target" == "windows" ]]; then
    IRIS_DRIVE_DEV_VM_WINDOWS_CONFIG_DIR="$config" \
      IRIS_DRIVE_DESKTOP_GUI_LAUNCH_LINK="$launch_link" \
      IRIS_DRIVE_DESKTOP_GUI_EXPECTED_STATE="$expected_state" \
      IRIS_DRIVE_DESKTOP_GUI_EXPECTED_APP_KEY="$expected_app_key" \
      IRIS_DRIVE_DESKTOP_GUI_ACTION_TIMEOUT_SECS="$LINK_TIMEOUT_SECS" \
      "$ROOT/scripts/desktop-gui-smoke.sh" windows "$host"
  else
    IRIS_DRIVE_DEV_VM_LINUX_CONFIG_DIR="$config" \
      IRIS_DRIVE_DESKTOP_GUI_LAUNCH_LINK="$launch_link" \
      IRIS_DRIVE_DESKTOP_GUI_EXPECTED_STATE="$expected_state" \
      IRIS_DRIVE_DESKTOP_GUI_EXPECTED_APP_KEY="$expected_app_key" \
      IRIS_DRIVE_DESKTOP_GUI_ACTION_TIMEOUT_SECS="$LINK_TIMEOUT_SECS" \
      "$ROOT/scripts/desktop-gui-smoke.sh" linux "$host"
  fi
}

run_desktop_gui_link_action() {
  run_desktop_gui_link_action_at \
    "$1" "$(host_value "$1" config)" "$2" "$3" "${4:-}"
}

run_timed_desktop_gui_primary_approval() {
  local request_url="$1" output action_pid status=0
  output="$(mktemp -t iris-drive-desktop-gui-approval.XXXXXX)"
  run_desktop_gui_link_action \
    "$owner_label" "$request_url" approval_queued \
    "$(host_value "$windows_label" app_key_npub)" >"$output" 2>&1 &
  action_pid=$!

  while ! grep -Fq 'IRIS_DRIVE_DESKTOP_GUI_APPROVAL_STARTED=1' "$output"; do
    if ! kill -0 "$action_pid" >/dev/null 2>&1; then
      wait "$action_pid" || status=$?
      cat "$output" >&2
      rm -f "$output"
      ((status != 0)) || status=1
      echo "desktop GUI approval exited before activation" >&2
      return "$status"
    fi
    sleep 0.02
  done

  mark_desktop_gui_primary_approval_submission
  wait "$action_pid" || status=$?
  cat "$output"
  if ((status == 0)) && ! grep -Fq 'IRIS_DRIVE_DESKTOP_GUI_APPROVAL_SUBMITTED=1' "$output"; then
    echo "desktop GUI approval exited without successful submission" >&2
    status=1
  fi
  rm -f "$output"
  if ((status == 0)); then
    # GTK approval restarts its daemon, which exits with the smoke GUI. Restore
    # the harness's mount and transport settings within the original deadline.
    start_daemon "$owner_label" || return $?
  fi
  return "$status"
}

desktop_gui_pair_has_direct_fips() {
  local left_status right_status left_npub right_npub
  left_npub="$(host_value "$owner_label" app_key_npub)"
  right_npub="$(host_value "$windows_label" app_key_npub)"
  left_status="$(idrive_cmd "$owner_label" status 2>/dev/null || true)"
  right_status="$(idrive_cmd "$windows_label" status 2>/dev/null || true)"
  jq -e --arg peer "$right_npub" \
    'any(.peers[]?; .app_key_npub == $peer and .fips_online == true and .fips_direct_online == true)' \
    >/dev/null 2>&1 <<<"$left_status" || return 1
  jq -e --arg peer "$left_npub" \
    'any(.peers[]?; .app_key_npub == $peer and .fips_online == true and .fips_direct_online == true)' \
    >/dev/null 2>&1 <<<"$right_status"
}

all_linking_complete() {
  local label status
  for label in "${LABELS[@]}"; do
    status="$(idrive_cmd "$label" status 2>/dev/null || true)"
    jq -e '
      .profile.authorization_state == "authorized" and
      .profile.pending_device_approval_receipt_count == 0
    ' >/dev/null 2>&1 <<<"$status" || return 1
  done
  if bool_true "$DESKTOP_GUI_LINKING"; then
    desktop_gui_pair_has_direct_fips
  fi
}

desktop_gui_aux_idrive() {
  local idrive args="" arg script
  idrive="$(host_value "$owner_label" idrive)"
  for arg in "$@"; do
    args+=" $(sh_quote "$arg")"
  done
  script="
set -Eeuo pipefail
$(sh_quote "$idrive") --config-dir $(sh_quote "$DESKTOP_GUI_AUX_CONFIG")$args
"
  remote_exec "$owner_label" "$script" | tr -d '\r'
}

desktop_gui_windows_can_admin() {
  local status
  status="$(idrive_cmd "$windows_label" status 2>/dev/null || true)"
  jq -e '.profile.can_admin_profile == true' >/dev/null 2>&1 <<<"$status"
}

start_desktop_gui_aux_daemon() {
  local idrive log owner_addr port windows_addr windows_port windows_npub script
  idrive="$(host_value "$owner_label" idrive)"
  log="${DESKTOP_GUI_AUX_CONFIG%/*}/linux-gui-joiner.daemon.log"
  DESKTOP_GUI_AUX_PIDFILE="${DESKTOP_GUI_AUX_CONFIG%/*}/linux-gui-joiner.daemon.pid"
  owner_addr="$(host_value "$owner_label" fips_addr)"
  port=$((FIPS_PORT_BASE + ${#LABELS[@]}))
  windows_addr="$(host_value "$windows_label" fips_addr)"
  windows_port="$(host_value "$windows_label" fips_port)"
  windows_npub="$(host_value "$windows_label" app_key_npub)"
  [[ -n "$owner_addr" && -n "$windows_addr" && -n "$windows_port" && -n "$windows_npub" ]] || {
    echo "reverse desktop GUI link lacks a deterministic Windows FIPS endpoint" >&2
    return 1
  }
  script="
set -Eeuo pipefail
config=$(sh_quote "$DESKTOP_GUI_AUX_CONFIG")
pidfile=$(sh_quote "$DESKTOP_GUI_AUX_PIDFILE")
if [[ -f \"\$pidfile\" ]]; then
  old=\"\$(cat \"\$pidfile\" 2>/dev/null || true)\"
  [[ -z \"\$old\" ]] || kill \"\$old\" >/dev/null 2>&1 || true
fi
nohup env \\
  IRIS_DRIVE_FIPS_UDP_BIND_ADDR=$(sh_quote "0.0.0.0:$port") \\
  IRIS_DRIVE_FIPS_UDP_EXTERNAL_ADDR=$(sh_quote "$owner_addr:$port") \\
  IRIS_DRIVE_FIPS_UDP_PUBLIC=false \\
  IRIS_DRIVE_FIPS_ENABLE_LAN_DISCOVERY=true \\
  IRIS_DRIVE_FIPS_ENABLE_WEBRTC=true \\
  IRIS_DRIVE_FIPS_ENABLE_BOOTSTRAP=false \\
  IRIS_DRIVE_FIPS_OPEN_DISCOVERY_MAX_PENDING=0 \\
  IRIS_DRIVE_FIPS_STATIC_PEERS=$(sh_quote "$windows_npub=$windows_addr:$windows_port") \\
  $(sh_quote "$idrive") --config-dir \"\$config\" daemon \\
    --watch-debounce-ms 100 --gateway-port 0$(daemon_relay_args_posix) \\
    >$(sh_quote "$log") 2>&1 < /dev/null &
echo \$! >\"\$pidfile\"
"
  remote_exec "$owner_label" "$script"
}

stop_desktop_gui_aux_daemon() {
  [[ -n "$DESKTOP_GUI_AUX_PIDFILE" ]] || return 0
  local script
  script="
pidfile=$(sh_quote "$DESKTOP_GUI_AUX_PIDFILE")
if [[ -f \"\$pidfile\" ]]; then
  pid=\"\$(cat \"\$pidfile\" 2>/dev/null || true)\"
  [[ -z \"\$pid\" ]] || kill \"\$pid\" >/dev/null 2>&1 || true
fi
"
  remote_exec "$owner_label" "$script" >/dev/null 2>&1 || true
  DESKTOP_GUI_AUX_PIDFILE=""
}

desktop_gui_reverse_link_complete() {
  local label status windows_npub
  windows_npub="$(host_value "$windows_label" app_key_npub)"
  status="$(desktop_gui_aux_idrive status 2>/dev/null || true)"
  jq -e --arg peer "$windows_npub" '
    .profile.authorization_state == "authorized" and
    .profile.pending_device_approval_receipt_count == 0 and
    .network.fips.running == true and
    .network.fips.fresh == true and
    any(.peers[]?; .app_key_npub == $peer and .fips_online == true and .fips_direct_online == true)
  ' >/dev/null 2>&1 <<<"$status" || return 1
  for label in "${LABELS[@]}"; do
    status="$(idrive_cmd "$label" status 2>/dev/null || true)"
    jq -e '
      .profile.authorization_state == "authorized" and
      .profile.pending_device_approval_receipt_count == 0
    ' >/dev/null 2>&1 <<<"$status" || return 1
  done
  status="$(idrive_cmd "$windows_label" status 2>/dev/null || true)"
  jq -e --arg peer "$DESKTOP_GUI_AUX_NPUB" \
    'any(.peers[]?; .app_key_npub == $peer and .fips_online == true and .fips_direct_online == true)' \
    >/dev/null 2>&1 <<<"$status"
}

desktop_gui_reverse_file_visible() {
  local label listing
  listing="$(desktop_gui_aux_idrive list 2>/dev/null || true)"
  jq -e --arg path "$DESKTOP_GUI_REVERSE_FILE" --arg sha "$DESKTOP_GUI_REVERSE_FILE_SHA" \
    'any(.files[]?; .path == $path and .sha256 == $sha)' \
    >/dev/null 2>&1 <<<"$listing" || return 1
  for label in "${LABELS[@]}"; do
    listing="$(idrive_cmd "$label" list 2>/dev/null || true)"
    jq -e --arg path "$DESKTOP_GUI_REVERSE_FILE" --arg sha "$DESKTOP_GUI_REVERSE_FILE_SHA" \
      'any(.files[]?; .path == $path and .sha256 == $sha)' \
      >/dev/null 2>&1 <<<"$listing" || return 1
  done
}

write_desktop_gui_reverse_file() {
  local content="$1" source b64 script idrive
  source="${DESKTOP_GUI_AUX_CONFIG%/*}/linux-gui-joiner.file"
  b64="$(printf '%s' "$content" | base64 | tr -d '\n')"
  idrive="$(host_value "$owner_label" idrive)"
  script="
set -Eeuo pipefail
printf '%s' $(sh_quote "$b64") | base64 -d >$(sh_quote "$source")
$(sh_quote "$idrive") --config-dir $(sh_quote "$DESKTOP_GUI_AUX_CONFIG") \\
  provider write $(sh_quote "$DESKTOP_GUI_REVERSE_FILE") $(sh_quote "$source") >/dev/null
sha256sum $(sh_quote "$source") | awk '{print \$1}'
"
  remote_exec "$owner_label" "$script"
}

run_bidirectional_desktop_gui_linking() {
  bool_true "$DESKTOP_GUI_LINKING" || return 0
  echo "ok: Windows WPF join -> Linux GTK approval used real daemons/FIPS and drained its durable ACK"

  local windows_npub invite_json reverse_invite request_status request_url request_profile request_admin
  local content base reverse_started_at reverse_deadline
  windows_npub="$(host_value "$windows_label" app_key_npub)"
  idrive_cmd "$owner_label" app-keys appoint-admin "$windows_npub" >/dev/null
  wait_until_for "Windows admin promotion" "$LINK_TIMEOUT_SECS" desktop_gui_windows_can_admin

  invite_json="$(idrive_cmd "$windows_label" app-keys invite)"
  reverse_invite="$(jq -r '.url' <<<"$invite_json")"
  [[ "$reverse_invite" == https://drive.iris.to/invite/* ]] || {
    echo "promoted Windows admin did not create a canonical invite" >&2
    return 1
  }
  [[ "$(jq -r '.admin_app_key_npub' <<<"$invite_json")" == "$windows_npub" ]] || {
    echo "reverse invite is not owned by the Windows admin" >&2
    return 1
  }

  base="$(host_value "$owner_label" base)"
  DESKTOP_GUI_AUX_CONFIG="$base/linux-gui-joiner/config"
  remote_exec "$owner_label" "mkdir -p $(sh_quote "$DESKTOP_GUI_AUX_CONFIG")"
  run_desktop_gui_link_action_at \
    "$owner_label" "$DESKTOP_GUI_AUX_CONFIG" "$reverse_invite" awaiting_approval
  request_status="$(desktop_gui_aux_idrive status)"
  DESKTOP_GUI_AUX_NPUB="$(jq -r '.profile.current_app_key_npub' <<<"$request_status")"
  request_url="$(jq -r '.profile.app_key_link_request.url' <<<"$request_status")"
  request_profile="$(jq -r '.profile.app_key_link_request.profile_id' <<<"$request_status")"
  request_admin="$(jq -r '.profile.app_key_link_request.admin_app_key_npub' <<<"$request_status")"
  is_app_key_request_url "$request_url" \
    && [[ "$request_profile" == "$owner_profile_id" && "$request_admin" == "$windows_npub" ]] \
    && [[ -n "$DESKTOP_GUI_AUX_NPUB" && "$DESKTOP_GUI_AUX_NPUB" != null ]] || {
      echo "Linux GTK reverse link request metadata is invalid" >&2
      return 1
    }

  stop_daemon "$windows_label"
  reverse_started_at="$(monotonic_milliseconds)"
  reverse_deadline=$((reverse_started_at + LINK_TIMEOUT_SECS * 1000))
  run_desktop_gui_link_action "$windows_label" "$request_url" approval_queued "$DESKTOP_GUI_AUX_NPUB"
  start_daemon "$windows_label"
  start_desktop_gui_aux_daemon
  wait_until_before \
    "Linux GTK join -> Windows WPF approval, direct FIPS, and durable ACK" \
    "$reverse_deadline" \
    desktop_gui_reverse_link_complete \
    "$reverse_started_at"

  DESKTOP_GUI_REVERSE_FILE="e2e/$RUN_ID/desktop-gui-reverse-link.txt"
  content="written after Linux GTK joined through Windows WPF in $RUN_ID"
  DESKTOP_GUI_REVERSE_FILE_SHA="$(write_desktop_gui_reverse_file "$content")"
  wait_until "post-link file from the reverse-linked Linux device" desktop_gui_reverse_file_visible
  stop_desktop_gui_aux_daemon
}
