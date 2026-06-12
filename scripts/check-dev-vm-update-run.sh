#!/usr/bin/env bash

set -Eeuo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
TMPDIR="$(mktemp -d -t iris-drive-dev-vm-update-check.XXXXXX)"
REMOTE_SUFFIX="$$"
MACOS_REMOTE="check-dev-vm-macos-$REMOTE_SUFFIX"
UBUNTU_REMOTE="check-dev-vm-ubuntu-$REMOTE_SUFFIX"
WINDOWS_REMOTE="check-dev-vm-windows-$REMOTE_SUFFIX"

cleanup() {
  git -C "$ROOT" remote remove "$MACOS_REMOTE" >/dev/null 2>&1 || true
  git -C "$ROOT" remote remove "$UBUNTU_REMOTE" >/dev/null 2>&1 || true
  git -C "$ROOT" remote remove "$WINDOWS_REMOTE" >/dev/null 2>&1 || true
  rm -rf "$TMPDIR"
}
trap cleanup EXIT

mkdir -p "$TMPDIR/bin"
cat > "$TMPDIR/bin/ssh" <<'SSH'
#!/usr/bin/env bash
set -Eeuo pipefail
printf '%s\n' "$*" >> "$IRIS_DRIVE_FAKE_SSH_LOG"
cat >/dev/null || true
exit 0
SSH
chmod +x "$TMPDIR/bin/ssh"
git init -q "$TMPDIR/hashtree"
git init -q "$TMPDIR/fips"
git init -q "$TMPDIR/nostr-social-graph"
git init -q "$TMPDIR/cashu-service"

git -C "$ROOT" remote add "$MACOS_REMOTE" check-macos:~/git/iris-drive.git
git -C "$ROOT" remote add "$UBUNTU_REMOTE" check-ubuntu:~/git/iris-drive.git
git -C "$ROOT" remote add "$WINDOWS_REMOTE" check-windows:~/git/iris-drive.git
for repo in "$TMPDIR/hashtree" "$TMPDIR/fips"; do
  git -C "$repo" remote add "$MACOS_REMOTE" check-macos:~/git/repo.git
  git -C "$repo" remote add "$UBUNTU_REMOTE" check-ubuntu:~/git/repo.git
  git -C "$repo" remote add "$WINDOWS_REMOTE" check-windows:~/git/repo.git
done

export IRIS_DRIVE_FAKE_SSH_LOG="$TMPDIR/ssh.log"
export PATH="$TMPDIR/bin:$PATH"
export IRIS_DRIVE_DEV_LAB_ENV="$TMPDIR/missing-dev-lab.env"
export IRIS_DRIVE_HASHTREE_ROOT="$TMPDIR/hashtree"
export IRIS_DRIVE_FIPS_ROOT="$TMPDIR/fips"
export IRIS_DRIVE_SOCIAL_GRAPH_ROOT="$TMPDIR/nostr-social-graph"
export IRIS_DRIVE_CASHU_SERVICE_ROOT="$TMPDIR/cashu-service"
export IRIS_DRIVE_DEV_VM_MACOS_REMOTE="$MACOS_REMOTE"
export IRIS_DRIVE_DEV_VM_UBUNTU_REMOTE="$UBUNTU_REMOTE"
export IRIS_DRIVE_DEV_VM_WINDOWS_REMOTE="$WINDOWS_REMOTE"
export IRIS_DRIVE_DEV_VM_SSH_PROBE_TIMEOUT=1
export IRIS_DRIVE_DEV_VM_USE_NVPN_STATIC_HINTS=auto

"$ROOT/scripts/dev-vm-update-run.sh" --only ubuntu --skip-push --no-run >/dev/null

if grep -E '(^| )check-macos( |$)|(^| )check-windows( |$)' "$IRIS_DRIVE_FAKE_SSH_LOG" >/dev/null; then
  echo "--only ubuntu unexpectedly probed unselected VM targets:" >&2
  cat "$IRIS_DRIVE_FAKE_SSH_LOG" >&2
  exit 1
fi

if ! grep -E '(^| )check-ubuntu( |$)' "$IRIS_DRIVE_FAKE_SSH_LOG" >/dev/null; then
  echo "--only ubuntu did not run the selected target" >&2
  cat "$IRIS_DRIVE_FAKE_SSH_LOG" >&2
  exit 1
fi

if ! grep -F -- "-LogonType S4U" "$ROOT/scripts/dev-vm-update-run.sh" >/dev/null; then
  echo "Windows dev VM scheduled daemon launch must use S4U so it survives SSH sessions" >&2
  exit 1
fi

if ! grep -F 'CARGO_PROFILE_DEV_DEBUG="${CARGO_PROFILE_DEV_DEBUG_DEFAULT}"' "$ROOT/scripts/dev-vm-update-run.sh" >/dev/null; then
  echo "POSIX VM Cargo builds must disable dev debuginfo by default to fit small macOS VM disks" >&2
  exit 1
fi

if ! grep -F '$env:CARGO_PROFILE_DEV_DEBUG = $CargoProfileDevDebugDefault' "$ROOT/scripts/dev-vm-update-run.sh" >/dev/null; then
  echo "Windows VM Cargo builds must use the same dev debuginfo override" >&2
  exit 1
fi

if ! grep -F 'IRIS_DRIVE_SOCIAL_GRAPH_ROOT' "$ROOT/scripts/dev-vm-update-run.sh" >/dev/null ||
  ! grep -F 'nostr-social-graph' "$ROOT/scripts/dev-vm-update-run.sh" >/dev/null ||
  ! grep -F 'SOCIAL_GRAPH_BARE' "$ROOT/scripts/dev-vm-update-run.sh" >/dev/null ||
  ! grep -F 'IRIS_DRIVE_CASHU_SERVICE_ROOT' "$ROOT/scripts/dev-vm-update-run.sh" >/dev/null ||
  ! grep -F 'cashu-service' "$ROOT/scripts/dev-vm-update-run.sh" >/dev/null ||
  ! grep -F 'CASHU_SERVICE_BARE' "$ROOT/scripts/dev-vm-update-run.sh" >/dev/null; then
  echo "dev VM sync must deploy hashtree sibling path dependencies" >&2
  exit 1
fi

if ! grep -F 'entitlements.pop("com.apple.developer.associated-domains", None)' "$ROOT/scripts/dev-vm-update-run.sh" >/dev/null ||
  ! grep -F 'entitlements.pop("com.apple.developer.fileprovider.testing-mode", None)' "$ROOT/scripts/dev-vm-update-run.sh" >/dev/null ||
  ! grep -F 'entitlements.pop("com.apple.security.application-groups", None)' "$ROOT/scripts/dev-vm-update-run.sh" >/dev/null; then
  echo "macOS dev VM signing must strip provisioned-only entitlements unless FileProvider is explicitly required" >&2
  exit 1
fi

echo "DEV_VM_UPDATE_RUN_CHECK_OK"
