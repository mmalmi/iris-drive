# shellcheck shell=bash
# Lab Mac VM helpers shared by the VM smoke, git sync and release smoke.
# Callers set ROOT to the repository root.

macos_vm_load_lab_env() {
  local env_file="${IRIS_DRIVE_DEV_LAB_ENV:-$HOME/.config/iris-drive/dev-lab.env}"
  if [[ -f "$env_file" ]]; then
    set -a
    # shellcheck disable=SC1090
    source "$env_file"
    set +a
  fi
}

# SSH destination: IRIS_DRIVE_MACOS_SSH_HOST, the first argument,
# IRIS_DRIVE_DEV_VM_MACOS_SSH_HOST, or the host of the VM's git remote.
macos_vm_ssh_host() {
  local host="${IRIS_DRIVE_MACOS_SSH_HOST:-${1:-${IRIS_DRIVE_DEV_VM_MACOS_SSH_HOST:-}}}"
  local remote="${IRIS_DRIVE_DEV_VM_MACOS_REMOTE:-macos}" url
  if [[ -z "$host" ]] && url="$(git -C "$ROOT" remote get-url "$remote" 2>/dev/null)"; then
    host="${url%%:*}"
    host="${host#*@}"
  fi
  if [[ -z "$host" ]]; then
    echo "set IRIS_DRIVE_MACOS_SSH_HOST or configure the macOS VM git remote" >&2
    return 2
  fi
  printf '%s\n' "$host"
}
