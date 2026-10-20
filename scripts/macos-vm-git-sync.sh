#!/usr/bin/env bash
# Publish the exact working tree to an isolated macOS VM checkout.
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
REMOTE_REF="${IRIS_DRIVE_MACOS_GIT_REF:-refs/heads/codex/macos-vm-sync}"
if [[ "$GUEST_SRC_ROOT" != /* ]]; then
  remote_home="$(ssh -o BatchMode=yes "$SSH_HOST" 'printf %s "$HOME"')"
  GUEST_SRC_ROOT="$remote_home/$GUEST_SRC_ROOT"
fi
GUEST_REPO="$GUEST_SRC_ROOT/iris-drive-release-gate"
GUEST_BARE="${IRIS_DRIVE_MACOS_GUEST_BARE_REPO:-$GUEST_SRC_ROOT/iris-drive-release-gate.git}"

git_dir="$(git -C "$ROOT" rev-parse --path-format=absolute --git-dir)"
tmp_index="$(mktemp "$git_dir/macos-vm-index.XXXXXX")"
cleanup() { rm -f "$tmp_index"; }
trap cleanup EXIT
export GIT_INDEX_FILE="$tmp_index"
git -C "$ROOT" read-tree HEAD
git -C "$ROOT" add -A
tree="$(git -C "$ROOT" write-tree)"
sync_commit="$(printf 'Temporary macOS VM sync\n' | git -C "$ROOT" commit-tree "$tree")"

ssh -o BatchMode=yes "$SSH_HOST" \
  "mkdir -p '$(dirname "$GUEST_BARE")'; test -d '$GUEST_BARE' || git init --bare '$GUEST_BARE'"
GIT_SSH_COMMAND="ssh -o BatchMode=yes" \
  git -C "$ROOT" push --force "$SSH_HOST:$GUEST_BARE" "$sync_commit:$REMOTE_REF"
ssh -o BatchMode=yes "$SSH_HOST" "
  set -e
  if [ ! -d '$GUEST_REPO/.git' ]; then
    git clone '$GUEST_BARE' '$GUEST_REPO'
  fi
  if git -C '$GUEST_REPO' rev-parse --verify HEAD >/dev/null 2>&1; then
    git -C '$GUEST_REPO' reset --hard HEAD
  fi
  git -C '$GUEST_REPO' remote set-url origin '$GUEST_BARE'
  git -C '$GUEST_REPO' fetch origin '$REMOTE_REF'
  git -C '$GUEST_REPO' checkout -B '${REMOTE_REF#refs/heads/}' FETCH_HEAD
  git -C '$GUEST_REPO' reset --hard FETCH_HEAD
  git -C '$GUEST_REPO' clean -ffd -e artifacts/ -e target/ -e macos/.build/
"
printf 'MACOS_VM_GIT_SYNC_OK %s\n' "$sync_commit"
