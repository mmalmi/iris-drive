#!/usr/bin/env python3
"""The physical gate reads a peer's file only after its bytes are stored locally.

`am instrument` force-stops the Drive app and its sync. A file listed by the
provider can still be downloading, so instrumenting then reads nothing.
"""
import hashlib
from pathlib import Path
import shlex
import subprocess
import tempfile
import unittest

ROOT = Path(__file__).resolve().parents[1]
SCRIPT = ROOT / "scripts/mobile-ios-android-linking-e2e.sh"
NAME = "ios-joiner-fixture.txt"
CONTENT = "physical iOS joiner post-link provider write fixture"


def script_functions(source, first, last):
    return source[source.index(first + "() {"):source.index(last + "() {")]


class MobileProviderReadbackTests(unittest.TestCase):
    def run_exchange(self, root, reads_until_local, wait_secs=20):
        """Run the post-link exchange against a phone whose bytes arrive late.

        The fake phone lists the file at once and stores its bytes only on the
        given local read; every adb call is appended to `events` in order.
        """
        sha = hashlib.sha256(CONTENT.encode()).hexdigest()
        write_sha = hashlib.sha256(b"android-content").hexdigest()
        fake_adb = root / "adb"
        fake_adb.write_text(f"""#!/usr/bin/env bash
M={shlex.quote(str(root))}
args="$*"
arg_after() {{ local key="$1"; shift; while [[ $# -gt 1 ]]; do [[ "$1" == "$key" ]] && {{ printf '%s' "$2"; return; }}; shift; done; }}
case "$args" in
  *"dump-provider-read"*)
    reads=$(( $(cat "$M/reads" 2>/dev/null || echo 0) + 1 )); echo "$reads" >"$M/reads"
    request="$(arg_after to.iris.drive.DEBUG_PROVIDER_REQUEST_ID "$@")"
    if (( reads >= {reads_until_local} )); then
      printf '{{"request_id":"%s","path":"{NAME}","size":{len(CONTENT)},"sha256":"{sha}"}}' "$request" >"$M/read.json"
      echo "local-read-match" >>"$M/events"
    else
      printf '{{"request_id":"%s","path":"{NAME}","error":"missing block"}}' "$request" >"$M/read.json"
      echo "local-read-missing" >>"$M/events"
    fi ;;
  *"dump-provider-list"*) echo "list" >>"$M/events" ;;
  *"cat files/debug-provider-list.json"*) printf '{{"entries":[{{"path":"{NAME}"}}]}}' ;;
  *"cat files/debug-provider-read.json"*) cat "$M/read.json" 2>/dev/null ;;
  *"am instrument"*"#readPostLinkFileThroughDocumentsProvider"*)
    echo "instrument-read" >>"$M/events"
    printf 'IRIS_ANDROID_PROVIDER_READ_SHA256={sha}\\nOK (1 test)\\n' ;;
  *"am instrument"*"#writePostLinkFileThroughDocumentsProvider"*)
    echo "instrument-write" >>"$M/events"
    printf 'IRIS_ANDROID_PROVIDER_WRITE_SHA256={write_sha}\\nOK (1 test)\\n' ;;
  *) : ;;
esac
""")
        fake_adb.chmod(0o755)
        source = SCRIPT.read_text(encoding="utf-8")
        functions = (
            script_functions(source, "sha256_text", "emit_skip")
            + script_functions(source, "start_android_app", "prepare_android_fresh")
            + script_functions(source, "wait_for_android_authorized", "assert_delivery_bound")
        )
        script = f"""
set -Eeuo pipefail
M={shlex.quote(str(root))}
ADB="$M/adb"
ANDROID_SERIAL_SELECTED=fixture
ANDROID_PACKAGE=to.iris.drive.uitest
ANDROID_ACTIVITY="$ANDROID_PACKAGE/to.iris.drive.app.MainActivity"
ANDROID_TEST_RUNNER="$ANDROID_PACKAGE.test/androidx.test.runner.AndroidJUnitRunner"
DEBUG_ACTION_EXTRA=to.iris.drive.DEBUG_ACTION
POST_LINK_WAIT_SECS={wait_secs}
RESULT_DIR="$M/results"
mkdir -p "$RESULT_DIR"
fail() {{ printf 'fail: %s\\n' "$*" >&2; exit 1; }}
signal_ios_test() {{ :; }}
wait_for_ios_marker() {{ :; }}
assert_ios_provider_write_marker() {{ :; }}
{functions}
exchange_post_link_provider_files {NAME} {shlex.quote(CONTENT)} android-file.txt android-content
"""
        result = subprocess.run(["bash", "-c", script], capture_output=True, text=True, timeout=60)
        events_file = root / "events"
        events = events_file.read_text().split() if events_file.exists() else []
        return result, events

    def test_instruments_read_only_after_bytes_are_local(self):
        with tempfile.TemporaryDirectory() as directory:
            result, events = self.run_exchange(Path(directory), reads_until_local=3)
            self.assertEqual(result.returncode, 0, result.stderr)
            self.assertIn("instrument-read", events, events)
            before_read = events[:events.index("instrument-read")]
            self.assertIn("local-read-match", before_read, events)
            self.assertEqual(before_read.count("local-read-missing"), 2, events)

    def test_reports_last_local_read_when_bytes_never_arrive(self):
        with tempfile.TemporaryDirectory() as directory:
            result, events = self.run_exchange(Path(directory), reads_until_local=10_000, wait_secs=3)
            self.assertNotEqual(result.returncode, 0)
            self.assertNotIn("instrument-read", events, events)
            self.assertIn("missing block", result.stderr)


if __name__ == "__main__":
    unittest.main()
