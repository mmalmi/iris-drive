#!/usr/bin/env python3
"""Approval observation must survive test startup without extending acceptance."""
import json
from pathlib import Path
import shlex
import subprocess
import tempfile
import unittest

ROOT = Path(__file__).resolve().parents[1]
HELPER = ROOT / "scripts/lib/ios-linking-observer.sh"


class IosLinkingObserverTests(unittest.TestCase):
    def test_observer_survives_seventy_second_xctest_startup(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            state = root / "state.json"
            state.write_text("{}")
            (root / "linked-config").mkdir()
            (root / "owner-config").mkdir()
            script = f"""
set -Eeuo pipefail
source {shlex.quote(str(HELPER))}
STATE_FILE={shlex.quote(str(state))}
IDRIVE=idrive
LOCAL_RELAY_URL=ws://fixture
DEVICE_UDID=fixture
BUNDLE_ID=fixture
date() {{ printf '1000\\n'; }}
# Deterministically advance past the old observer's 60-second startup lifetime.
before_deadline() {{ python3 -c 'import sys; raise SystemExit(0 if float(sys.argv[1]) > 1070 else 1)' "$1"; }}
xcrun() {{ :; }}
idrive() {{
  case "$3" in
    daemon) while :; do sleep 1; done ;;
    status) printf '%s\\n' '{{"profile":{{"authorization_state":"authorized","roster_size":2,"pending_device_approval_receipt_count":0}}}}' ;;
    *) return 92 ;;
  esac
}}
start_linked_device_sync_observer {shlex.quote(str(root/'linked-config'))} {shlex.quote(str(root/'owner-config'))} 2 {shlex.quote(str(root/'linked'))} {shlex.quote(str(root/'ack'))} observer
wait "$observer"
[[ -s {shlex.quote(str(root/'linked'))} && -s {shlex.quote(str(root/'ack'))} ]]
"""
            result = subprocess.run(["bash", "-c", script], capture_output=True, text=True, timeout=10)
            self.assertEqual(result.returncode, 0, result.stderr)

    def run_blocked_relaunch_observer(self, root, epilogue):
        """Run the observer while the owner relaunch blocks, as during XCTest teardown.

        The linked device only becomes authorized after the relaunch has started,
        which is when hosted runs received the iOS approval. Markers, not sleeps,
        order every step; the timeouts only bound a failure.
        """
        bin_dir = root / "bin"
        bin_dir.mkdir()
        (root / "linked").mkdir()
        (root / "owner").mkdir()
        state = root / "state.json"
        state.write_text("{}")
        fake_idrive = bin_dir / "idrive"
        fake_idrive.write_text(f"""#!/usr/bin/env bash
set -u
M={shlex.quote(str(root))}
case "$2:$3" in
  "$M/owner:status")
    pending=1; [[ -e "$M/acked" ]] && pending=0
    printf '{{"profile":{{"roster_size":2,"pending_device_approval_receipt_count":%s}}}}\\n' "$pending" ;;
  "$M/linked:status")
    state=awaiting_approval; [[ -e "$M/linked-authorized" ]] && state=authorized
    printf '{{"profile":{{"authorization_state":"%s"}}}}\\n' "$state" ;;
  "$M/linked:sync")
    [[ -e "$M/launch-started" ]] && touch "$M/linked-authorized" "$M/acked"
    exit 0 ;;
  "$M/linked:daemon")
    echo "$$" >"$M/daemon.pid"
    while :; do
      [[ -e "$M/launch-started" ]] && touch "$M/linked-authorized" "$M/acked"
      sleep 0.05
    done ;;
  *) exit 92 ;;
esac
""")
        fake_idrive.chmod(0o755)
        fake_xcrun = bin_dir / "xcrun"
        fake_xcrun.write_text(f"""#!/usr/bin/env bash
M={shlex.quote(str(root))}
[[ "$1 $2" == "simctl launch" ]] || exit 0
touch "$M/launch-started"
for _ in $(seq 1 400); do [[ -e "$M/release-launch" ]] && exit 0; sleep 0.05; done
exit 0
""")
        fake_xcrun.chmod(0o755)
        script = f"""
set -Eeuo pipefail
export PATH={shlex.quote(str(bin_dir))}:"$PATH"
source {shlex.quote(str(HELPER))}
M={shlex.quote(str(root))}
STATE_FILE="$M/state.json"
IDRIVE={shlex.quote(str(fake_idrive))}
LOCAL_RELAY_URL=ws://fixture
DEVICE_UDID=fixture
BUNDLE_ID=fixture
observer=""
trap 'touch "$M/release-launch"; [[ -z "$observer" ]] || {{ kill "$observer" 2>/dev/null || true; wait "$observer" 2>/dev/null || true; }}' EXIT
start_linked_device_sync_observer "$M/linked" "$M/owner" 2 "$M/linked-observed" "$M/ack-observed" observer
{epilogue}
"""
        return subprocess.run(["bash", "-c", script], capture_output=True, text=True, timeout=30)

    def test_blocked_owner_relaunch_does_not_delay_linked_device(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            result = self.run_blocked_relaunch_observer(root, """
for _ in $(seq 1 200); do [[ -s "$M/linked-observed" ]] && break; sleep 0.05; done
[[ -s "$M/linked-observed" ]] || { echo "linked device not observed while owner relaunch blocked" >&2; exit 3; }
[[ ! -e "$M/release-launch" ]]
touch "$M/release-launch"
wait "$observer"; observer=""
[[ -s "$M/ack-observed" ]]
""")
            self.assertEqual(result.returncode, 0, result.stderr)

    def test_stopping_observer_stops_linked_daemon(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            result = self.run_blocked_relaunch_observer(root, """
for _ in $(seq 1 200); do [[ -s "$M/daemon.pid" && -e "$M/launch-started" ]] && break; sleep 0.05; done
daemon="$(cat "$M/daemon.pid")"
# Stop while the owner relaunch is still in flight, then let it return.
kill "$observer"
touch "$M/release-launch"
wait "$observer" || true; observer=""
for _ in $(seq 1 100); do kill -0 "$daemon" 2>/dev/null || exit 0; sleep 0.05; done
echo "linked daemon $daemon outlived its observer" >&2
exit 4
""")
            self.assertEqual(result.returncode, 0, result.stderr)

    def test_actual_approval_keeps_fifteen_second_deadline(self):
        with tempfile.TemporaryDirectory() as directory:
            state = Path(directory) / "state.json"
            for timestamp, expected in [("1970-01-01T00:17:50Z", 1085),
                                        ("1970-01-01T00:16:30Z", 1180)]:
                with self.subTest(timestamp=timestamp):
                    state.write_text(json.dumps({"ios_config_mutation_audit": [
                        {"action": "approve_device", "timestamp": timestamp, "error": ""}]}))
                    result = subprocess.run(["bash", "-c",
                        f"source {shlex.quote(str(HELPER))}; linked_device_observer_deadline 1000 {shlex.quote(str(state))}"],
                        capture_output=True, text=True, timeout=10)
                    self.assertEqual(result.returncode, 0, result.stderr)
                    self.assertEqual(float(result.stdout), expected)


if __name__ == "__main__":
    unittest.main()
