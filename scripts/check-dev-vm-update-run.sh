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

if ! grep -F 'local_head="$(git -C "$ROOT" rev-parse HEAD)"' "$ROOT/scripts/dev-vm-smoke.sh" >/dev/null ||
  ! grep -F 'git -C ~/src/iris-drive rev-parse HEAD' "$ROOT/scripts/dev-vm-smoke.sh" >/dev/null ||
  ! grep -F 'rev-parse HEAD' "$ROOT/scripts/dev-vm-smoke.sh" >/dev/null; then
  echo "dev VM smoke revision checks must compare full commit IDs, not ambiguous short hashes" >&2
  exit 1
fi

if grep -F 'rev-parse --short HEAD' "$ROOT/scripts/dev-vm-smoke.sh" >/dev/null; then
  echo "dev VM smoke must not compare git rev-parse --short HEAD output across machines" >&2
  exit 1
fi

python3 - "$ROOT/scripts/dev-vm-smoke.sh" <<'PY'
import pathlib
import re
import sys

text = pathlib.Path(sys.argv[1]).read_text()
functions = re.findall(r"macos_config_dir\(\) \{(.*?)\n\}", text, re.S)
if len(functions) < 2:
    raise SystemExit("expected macos_config_dir helpers in dev-vm-smoke.sh")
for body in functions:
    newest = body.find("best_mtime")
    wildcard = body.find("Group\\ Containers/*.to.iris.drive")
    status = body.find("daemon-status.json")
    if newest < 0 or wildcard < 0 or status < 0:
        raise SystemExit(
            "macOS dev VM smoke must pick the active app-group config by daemon-status freshness"
        )
PY

if ! grep -F "IRIS_DRIVE_DEV_VM_LINUX_CONFIG_DIR=%s" "$ROOT/scripts/dev-vm-update-run.sh" >/dev/null ||
  ! grep -F "IRIS_DRIVE_DEV_VM_LINUX_MOUNTPOINT=%s" "$ROOT/scripts/dev-vm-update-run.sh" >/dev/null; then
  echo "POSIX VM runs must forward explicit Linux config and mountpoint overrides" >&2
  exit 1
fi

if grep -F '[[ -e "$mountpoint" ]] || return 0' "$ROOT/scripts/dev-vm-update-run.sh" >/dev/null ||
  ! grep -F 'local mounted=0' "$ROOT/scripts/dev-vm-update-run.sh" >/dev/null ||
  ! grep -F 'findmnt -rn --mountpoint "$mountpoint"' "$ROOT/scripts/dev-vm-update-run.sh" >/dev/null; then
  echo "dev VM stale FUSE detach must not rely on -e because broken mounts report ENOTCONN" >&2
  exit 1
fi

if ! grep -F 'STATIC_PEERS_COMPLETE_BY_INDEX' "$ROOT/scripts/dev-vm-update-run.sh" >/dev/null ||
  ! grep -F 'STATIC_PEERS_COMPLETE=' "$ROOT/scripts/dev-vm-update-run.sh" >/dev/null ||
  ! grep -F 'if [[ "$STATIC_PEERS_COMPLETE" == "1" ]]' "$ROOT/scripts/dev-vm-update-run.sh" >/dev/null ||
  ! grep -F 'FIPS_OPEN_DISCOVERY_MAX_PENDING_EFFECTIVE="16"' "$ROOT/scripts/dev-vm-update-run.sh" >/dev/null ||
  ! grep -F '$FipsOpenDiscoveryMaxPendingEffective = "16"' "$ROOT/scripts/dev-vm-update-run.sh" >/dev/null ||
  ! grep -F '$StaticPeersComplete' "$ROOT/scripts/dev-vm-update-run.sh" >/dev/null; then
  echo "partial static FIPS hints must keep bootstrap/open discovery enabled for missing VM edges" >&2
  exit 1
fi

if ! grep -F 'vpn_active' "$ROOT/scripts/dev-vm-update-run.sh" >/dev/null ||
  ! grep -F '$VpnActive' "$ROOT/scripts/dev-vm-update-run.sh" >/dev/null; then
  echo "nvpn static hint detection must reject stale tunnel_ip values when vpn_active is false" >&2
  exit 1
fi

if ! grep -F '$env:CARGO_PROFILE_DEV_DEBUG = $CargoProfileDevDebugDefault' "$ROOT/scripts/dev-vm-update-run.sh" >/dev/null; then
  echo "Windows VM Cargo builds must use the same dev debuginfo override" >&2
  exit 1
fi

if ! grep -F '$WindowsConfigDirOverride' "$ROOT/scripts/dev-vm-update-run.sh" >/dev/null; then
  echo "Windows VM runs must forward explicit config dir overrides" >&2
  exit 1
fi

if ! grep -F 'windows\bin\Debug\net8.0-windows\win-x64\publish\idrive.exe' "$ROOT/scripts/dev-vm-update-run.sh" >/dev/null ||
  ! grep -F '& $Idrive --config-dir $ConfigDir status' "$ROOT/scripts/dev-vm-update-run.sh" >/dev/null; then
  echo "Windows VM status checks must use the same config override as the daemon" >&2
  exit 1
fi

if ! grep -F 'skipping dirty check before first checkout' "$ROOT/scripts/dev-vm-update-run.sh" >/dev/null ||
  ! grep -F 'rev-parse --verify HEAD' "$ROOT/scripts/dev-vm-update-run.sh" >/dev/null ||
  ! grep -F '$CurrentBranch = ""' "$ROOT/scripts/dev-vm-update-run.sh" >/dev/null; then
  echo "Windows VM sync must tolerate fresh checkouts whose remote HEAD is unborn" >&2
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

if ! grep -F 'IRIS_DRIVE_DEV_VM_REQUIRE_FILEPROVIDER="${IRIS_DRIVE_DEV_VM_REQUIRE_FILEPROVIDER:-}"' "$ROOT/scripts/dev-vm-update-run.sh" >/dev/null ||
  ! grep -F 'IRIS_DRIVE_DEV_VM_MACOS_KEEP_PROVISIONED_DEBUG_ENTITLEMENTS="${IRIS_DRIVE_DEV_VM_MACOS_KEEP_PROVISIONED_DEBUG_ENTITLEMENTS:-}"' "$ROOT/scripts/dev-vm-update-run.sh" >/dev/null ||
  ! grep -F 'IRIS_DRIVE_DEV_VM_MACOS_KEEP_FILEPROVIDER_TESTING_MODE="${IRIS_DRIVE_DEV_VM_MACOS_KEEP_FILEPROVIDER_TESTING_MODE:-}"' "$ROOT/scripts/dev-vm-update-run.sh" >/dev/null; then
  echo "macOS dev VM entitlement filtering must pass FileProvider keep flags into Python" >&2
  exit 1
fi

if ! grep -F 'if value is None or value == "":' "$ROOT/scripts/dev-vm-update-run.sh" >/dev/null; then
  echo "macOS dev VM entitlement filtering must treat empty keep flags as defaulted" >&2
  exit 1
fi

echo "DEV_VM_UPDATE_RUN_CHECK_OK"
