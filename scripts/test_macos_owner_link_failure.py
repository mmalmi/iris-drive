"""A failed GUI action must stop a journey called from an if/negated context."""
import os
from pathlib import Path
import subprocess
import unittest

ROOT = Path(__file__).resolve().parents[1]


class OwnerLinkFailureTests(unittest.TestCase):
    def test_failed_steps_cannot_report_a_successful_journey(self):
        source = (ROOT / 'scripts/lib/macos-device-link-smoke.sh').read_text()
        function = source[source.index('run_macos_owner_device_link_journey() {'):]
        function = function.replace('/usr/bin/swift', 'swift_fixture')
        fixture = r'''
set -euo pipefail
IDRIVE_CLI=idrive_fixture
SMOKE_DIR=/tmp
APP_PATH=/fixture
ROOT=/fixture
MACOS_OWNER_LINK_TIMEOUT_SECS=15
validate_macos_owner_link_timeout() { :; }
macos_owner_link_status() { echo '{}'; }
json_get() {
 case "$1" in
  profile.roster_size) echo 1 ;;
  url) echo https://drive.iris.to/invite/fixture ;;
  app_key_link_request.url) echo https://drive.iris.to/approve-device/fixture ;;
  app_key_link_request.admin_app_key_npub) echo owner ;;
  current_app_key_npub) echo joiner ;;
 esac
}
idrive_fixture() { [[ "$*" != *'provider write'* || "$FAIL_STEP" != write ]]; }
mkdir_p_or_fail() { :; }
configure_macos_smoke_relay() { :; }
terminate_app_process() { :; }
terminate_smoke_daemon_processes() { :; }
stop_macos_forward_link_joiner_daemon() { :; }
stop_macos_owner_daemon() { :; }
start_macos_owner_link_daemon() { :; }
macos_owner_link_wait_for() { [[ "$1" != 'post-link provider write'* || "$FAIL_STEP" != visibility ]]; }
launch_macos_owner_link_shell() { [[ "$FAIL_STEP" != launch ]]; }
app_process_pids() { echo 100; }
open() { [[ "$FAIL_STEP" != open ]]; }
swift_fixture() { [[ "$FAIL_STEP" != "$3" ]]; }
sleep() { :; }
macos_owner_link_cancel_preserved_request() { :; }
macos_owner_link_monotonic_milliseconds() { echo 1; }
macos_owner_link_wait_before() { [[ "$FAIL_STEP" != approval ]]; }
'''
        # Avoid leaving any fixture writes even when testing the old false pass.
        function = function.replace('"$SMOKE_DIR/macos-owner-link-source.txt"', '"/dev/null"')
        script = fixture + function + '\nif run_macos_owner_device_link_journey config owner; then exit 0; else exit 23; fi\n'
        for step in ['launch', 'open', 'Cancel', 'Approve', 'approval', 'write', 'visibility']:
            with self.subTest(step=step):
                result = subprocess.run(['bash', '-c', script], capture_output=True, text=True,
                                        env={**os.environ, 'FAIL_STEP': step}, timeout=5)
                self.assertEqual(result.returncode, 23, result.stdout + result.stderr)
                self.assertNotIn('MACOS_OWNER_DEVICE_LINK_JOURNEY_OK', result.stdout)


if __name__ == '__main__':
    unittest.main()
