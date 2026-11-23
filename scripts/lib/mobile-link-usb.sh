# Sourced by mobile-ios-android-linking-e2e.sh after selecting the two physical devices.
start_mobile_link_transport() {
  case "${IRIS_DRIVE_MOBILE_LINK_TRANSPORT:-configured}" in
    configured) return ;;
    usb) ;;
    *) fail "choose configured or usb for IRIS_DRIVE_MOBILE_LINK_TRANSPORT" ;;
  esac
  [[ -z "${IRIS_DRIVE_USB_FIXTURE_PORTS:-}" ]] || return 0
  command -v iproxy >/dev/null || fail "USB physical linking requires iproxy"
  IRIS_DRIVE_NATIVE_MOBILE_RESERVED=1 IRIS_DRIVE_IOS_DEVICE="$IOS_DEVICE_SELECTED" \
    IRIS_DRIVE_ANDROID_SERIAL="$ANDROID_SERIAL_SELECTED" python3 "$ROOT/scripts/physical-link-usb-fixture.py" \
    --ios "$IOS_DEVICE_SELECTED" --adb "$ADB" --android "$ANDROID_SERIAL_SELECTED" "$0" "$@"
  exit "$?"
}

configure_android_link_transport() {
  [[ "${IRIS_DRIVE_MOBILE_LINK_TRANSPORT:-configured}" == usb ]] || return 0
  local relay_port store_port rest relay store
  IFS=, read -r relay_port store_port rest <<<"$IRIS_DRIVE_USB_FIXTURE_PORTS"
  relay="ws://127.0.0.1:$relay_port"
  store="http://127.0.0.1:$store_port"
  local -a launch=("$ADB" -s "$ANDROID_SERIAL_SELECTED" shell am start -W -n "$ANDROID_ACTIVITY"
    --es to.iris.drive.DEBUG_RELAY)
  "${launch[@]}" "$relay" --es "$DEBUG_ACTION_EXTRA" replace-relays >/dev/null
  "${launch[@]}" "$store" --es "$DEBUG_ACTION_EXTRA" replace-blossom >/dev/null
  local deadline=$((SECONDS + 10))
  while (( SECONDS < deadline )); do
    if copy_android_config "$TMP/usb-android-config.toml" && python3 - "$TMP/usb-android-config.toml" "$relay" "$store" <<'PY'
import sys
import tomllib
with open(sys.argv[1], "rb") as handle:
    config = tomllib.load(handle)
sys.exit(0 if config.get("relays") == [sys.argv[2]] and config.get("blossom_servers") == [sys.argv[3]] else 1)
PY
    then
      return 0
    fi
    sleep 0.1
  done
  fail "The isolated Android profile did not persist its USB relay and file service"
}
