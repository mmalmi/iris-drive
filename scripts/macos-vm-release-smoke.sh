#!/usr/bin/env bash
# Run the notarized-release launch smoke (scripts/macos-release-smoke.sh) on
# the lab Mac VM instead of this Mac, so an Iris Drive in use here is not
# stopped and restarted. Takes the same --app/--archive/--dmg arguments; the
# VM checkout is synced to this commit first.
set -Eeuo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
source "$ROOT/scripts/lib/macos-vm-host.sh"

APP="" ARCHIVE="" DMG=""
while [[ $# -gt 0 ]]; do
  case "$1" in
    --app) APP="${2:-}"; shift 2 ;;
    --archive) ARCHIVE="${2:-}"; shift 2 ;;
    --dmg) DMG="${2:-}"; shift 2 ;;
    *) echo "unexpected argument: $1" >&2; exit 2 ;;
  esac
done
if [[ ! -d "$APP" || ! -f "$ARCHIVE" || ! -f "$DMG" ]]; then
  echo "usage: $0 --app <signed app> --archive <updater archive> --dmg <dmg>" >&2
  exit 2
fi

macos_vm_load_lab_env
SSH_HOST="$(macos_vm_ssh_host)" || exit 2
SSH=(ssh -o BatchMode=yes -o ConnectTimeout=15 "$SSH_HOST")
WAIT_SECONDS="${IRIS_DRIVE_MACOS_VM_RELEASE_SMOKE_WAIT_SECONDS:-5400}"

# Both the release gate and its own Mac smoke stop and start the app on the
# VM; wait for them rather than racing them.
wait_while() {
  local what="$1" waited=0
  shift
  while "$@"; do
    if (( waited >= WAIT_SECONDS )); then
      echo "$what is still running after ${WAIT_SECONDS}s" >&2
      return 1
    fi
    (( waited > 0 )) || echo "waiting for $what to finish" >&2
    sleep 15
    waited=$((waited + 15))
  done
}

wait_while "the local release gate" pgrep -f "scripts/release-gate.sh"
case "${IRIS_DRIVE_MACOS_SKIP_GIT_SYNC:-0}" in
  1|true|TRUE|True|yes|YES|Yes|on|ON|On) ;;
  *) "$ROOT/scripts/macos-vm-git-sync.sh" "$SSH_HOST" ;;
esac
wait_while "the Mac VM smoke" "${SSH[@]}" "pgrep -f 'scripts/[m]acos-smoke.sh' >/dev/null"

guest_root="${IRIS_DRIVE_MACOS_GUEST_SRC_ROOT:-src}"
[[ "$guest_root" == /* ]] || guest_root="\$HOME/$guest_root"
remote="iris-drive-release-smoke-$(date +%s)-$$"
archive_name="$(basename "$ARCHIVE")"
dmg_name="$(basename "$DMG")"
"${SSH[@]}" "mkdir -p \"\$HOME/$remote/app\""
scp -q -o BatchMode=yes "$ARCHIVE" "$DMG" "$SSH_HOST:$remote/"
status=0
"${SSH[@]}" "cd \"\$HOME/$remote\" && tar -xzf '$archive_name' -C app \
  && cd \"$guest_root/iris-drive-release-gate\" \
  && ./scripts/macos-release-smoke.sh --app \"\$HOME/$remote/app/Iris Drive.app\" \
    --archive \"\$HOME/$remote/$archive_name\" --dmg \"\$HOME/$remote/$dmg_name\"" || status=$?
if (( status == 0 )); then
  "${SSH[@]}" "rm -rf \"\$HOME/$remote\""
  echo "macOS release smoke passed on the Mac VM"
else
  echo "macOS release smoke failed on the Mac VM; kept ~/$remote there for inspection" >&2
fi
exit "$status"
