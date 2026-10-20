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
    sync) : ;;
    status) printf '%s\\n' '{{"profile":{{"authorization_state":"authorized","roster_size":2,"pending_device_approval_receipt_count":0}}}}' ;;
    *) return 92 ;;
  esac
}}
start_linked_device_sync_observer linked owner 2 {shlex.quote(str(root/'linked'))} {shlex.quote(str(root/'ack'))} observer
wait "$observer"
[[ -s {shlex.quote(str(root/'linked'))} && -s {shlex.quote(str(root/'ack'))} ]]
"""
            result = subprocess.run(["bash", "-c", script], capture_output=True, text=True, timeout=10)
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
