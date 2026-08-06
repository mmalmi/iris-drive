#!/usr/bin/env bash
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
if [[ -z "$SSH_HOST" ]] && git -C "$ROOT" remote get-url "$REMOTE_NAME" >/dev/null 2>&1; then
  remote_url="$(git -C "$ROOT" remote get-url "$REMOTE_NAME")"
  SSH_HOST="${remote_url%%:*}"
  SSH_HOST="${SSH_HOST#*@}"
fi
[[ -n "$SSH_HOST" ]] || {
  echo "set IRIS_DRIVE_MACOS_SSH_HOST or configure the macOS VM git remote" >&2
  exit 2
}

GUEST_SRC_ROOT="${IRIS_DRIVE_MACOS_GUEST_SRC_ROOT:-src}"
GUEST_REPO="$GUEST_SRC_ROOT/iris-drive-release-gate"
REMOTE_ARTIFACT_DIR="artifacts/macos-smoke"
LOCAL_ARTIFACT_DIR="${IRIS_DRIVE_MACOS_ARTIFACT_DIR:-$ROOT/artifacts/macos-smoke}"

case "${IRIS_DRIVE_MACOS_SKIP_GIT_SYNC:-0}" in
  1|true|TRUE|True|yes|YES|Yes|on|ON|On) ;;
  *) "$ROOT/scripts/macos-vm-git-sync.sh" "$SSH_HOST" ;;
esac

link_journey="${IRIS_DRIVE_MACOS_SMOKE_LINK_JOURNEY:-1}"
remote_command="cd '$GUEST_REPO' && rm -rf '$REMOTE_ARTIFACT_DIR' && mkdir -p '$REMOTE_ARTIFACT_DIR' && { set +e; state_dir=\$(mktemp -d -t iris-drive-macos-vm-smoke-state) || exit 1; env"
remote_command+=" IRIS_DRIVE_MACOS_SMOKE_ARTIFACT_DIR='$REMOTE_ARTIFACT_DIR'"
remote_command+=" IRIS_DRIVE_MACOS_SMOKE_STATE_DIR=\"\$state_dir\""
remote_command+=" IRIS_DRIVE_MACOS_SMOKE_PRESERVE_ARTIFACTS=1"
remote_command+=" IRIS_DRIVE_MACOS_SMOKE_LINK_JOURNEY='$link_journey'"
remote_command+=" IRIS_DRIVE_MACOS_SMOKE_SURVIVAL_SECONDS=0"
remote_command+=" IRIS_DRIVE_MACOS_SIGNING='${IRIS_DRIVE_MACOS_SIGNING:-none}'"
remote_command+=" ./scripts/macos-smoke.sh >'$REMOTE_ARTIFACT_DIR/runner.raw.log' 2>&1"
remote_command+="; smoke_exit=\$?; rm -rf \"\$state_dir\"; rm -f '$REMOTE_ARTIFACT_DIR/runner.raw.log'; exit \$smoke_exit; }"

status=0
ssh -o BatchMode=yes "$SSH_HOST" "$remote_command" || status=$?
mkdir -p "$LOCAL_ARTIFACT_DIR"
rm -f "$LOCAL_ARTIFACT_DIR/result.json"
scp "$SSH_HOST:$GUEST_REPO/$REMOTE_ARTIFACT_DIR/result.json" \
  "$LOCAL_ARTIFACT_DIR/result.json" >/dev/null 2>&1 || true
if (( status != 0 )); then
  echo "macOS VM smoke failed; sanitized result copied to $LOCAL_ARTIFACT_DIR/result.json" >&2
  exit "$status"
fi
echo "MACOS_VM_SMOKE_OK"
