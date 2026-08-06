#!/usr/bin/env bash
# Run the macOS idle/UI release lane on the isolated macOS VM checkout.
set -Eeuo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
ENV_FILE="${IRIS_DRIVE_DEV_LAB_ENV:-$HOME/.config/iris-drive/dev-lab.env}"
if [[ -f "$ENV_FILE" ]]; then
  set -a
  # shellcheck disable=SC1090
  source "$ENV_FILE"
  set +a
fi

SSH_HOST="${IRIS_DRIVE_MACOS_SSH_HOST:-${1:-${IRIS_DRIVE_DEV_VM_MACOS_SSH_HOST:-}}}"
REMOTE_NAME="${IRIS_DRIVE_DEV_VM_MACOS_REMOTE:-macos}"
if [[ -z "$SSH_HOST" ]] && remote_url="$(git -C "$ROOT" remote get-url "$REMOTE_NAME" 2>/dev/null)"; then
  SSH_HOST="${remote_url%%:*}"
  SSH_HOST="${SSH_HOST#*@}"
fi
[[ -n "$SSH_HOST" ]] || {
  echo "set IRIS_DRIVE_MACOS_SSH_HOST for the macOS VM idle/UI lane" >&2
  exit 2
}
GUEST_REPO="${IRIS_DRIVE_MACOS_GUEST_SRC_ROOT:-src}/iris-drive-release-gate"
REMOTE_ARTIFACT_DIR="artifacts/macos-idle-cpu"
LOCAL_ARTIFACT_DIR="${IRIS_DRIVE_MACOS_IDLE_ARTIFACT_DIR:-$ROOT/artifacts/macos-idle-cpu}"

case "${IRIS_DRIVE_MACOS_SKIP_GIT_SYNC:-0}" in
  1|true|TRUE|True|yes|YES|Yes|on|ON|On) ;;
  *) "$ROOT/scripts/macos-vm-git-sync.sh" "$SSH_HOST" ;;
esac

remote_command="cd '$GUEST_REPO' && rm -rf '$REMOTE_ARTIFACT_DIR' && mkdir -p '$REMOTE_ARTIFACT_DIR' && env"
for name in \
  IRIS_DRIVE_MACOS_SIGNING \
  IRIS_DRIVE_RELEASE_GATE_MACOS_IDLE_CPU_ROLES \
  IRIS_DRIVE_RELEASE_GATE_MACOS_IDLE_CPU_WARMUP_SECS \
  IRIS_DRIVE_IDLE_CPU_DURATION_SECS \
  IRIS_DRIVE_IDLE_CPU_INTERVAL_SECS \
  IRIS_DRIVE_IDLE_CPU_APP_MAX \
  IRIS_DRIVE_IDLE_CPU_DAEMON_MAX
do
  [[ -z "${!name:-}" ]] || remote_command+=" $name='${!name}'"
done
remote_command+=" ./scripts/macos-idle-cpu-smoke.sh >'$REMOTE_ARTIFACT_DIR/runner.log' 2>&1"

status=0
ssh -o BatchMode=yes "$SSH_HOST" "$remote_command" || status=$?
mkdir -p "$LOCAL_ARTIFACT_DIR"
scp -qr "$SSH_HOST:$GUEST_REPO/$REMOTE_ARTIFACT_DIR/." "$LOCAL_ARTIFACT_DIR/" || true
if ((status != 0)); then
  echo "macOS VM idle/UI gate failed; artifacts copied to $LOCAL_ARTIFACT_DIR" >&2
  tail -200 "$LOCAL_ARTIFACT_DIR/runner.log" >&2 2>/dev/null || true
  exit "$status"
fi
cat "$LOCAL_ARTIFACT_DIR/runner.log"
echo "MACOS_VM_IDLE_CPU_OK"
