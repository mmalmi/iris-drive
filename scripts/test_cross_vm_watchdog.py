#!/usr/bin/env python3
"""Exercise the real remote-command watchdog with only transport replaced."""

import hashlib
import json
import os
from pathlib import Path
import shlex
import signal
import subprocess
import sys
import tempfile
import time
import unittest


ROOT = Path(__file__).resolve().parents[1]


def group_members(pgid):
    listing = subprocess.check_output(
        ["ps", "-axo", "pid=,ppid=,pgid=,stat=,command="], text=True)
    members = []
    for line in listing.splitlines():
        fields = line.split(None, 4)
        if len(fields) == 5 and int(fields[2]) == pgid and not fields[3].startswith("Z"):
            members.append(dict(zip(("pid", "ppid", "pgid", "stat", "command"), fields)))
    return members


class RemoteWatchdogTests(unittest.TestCase):
    def test_completed_commands_leave_no_timer_and_preserve_exit_status(self):
        for native_status in (0, 23):
            with self.subTest(native_status=native_status):
                self.run_command(0.15, native_status, 5, native_status)

    def test_timeout_stops_command_and_reaps_its_timer(self):
        self.run_command(10, 0, 1, 143)

    def run_command(self, delay, native_status, timeout, expected_status):
        source = (ROOT / "scripts/cross-vm-e2e.sh").read_text()
        body = source[source.index("remote_exec_with_timeout() {"):
                      source.index("\ndetect_local_fips_addr() {")]
        with tempfile.TemporaryDirectory() as directory:
            output = Path(directory) / "output"
            child = "import sys,time; time.sleep(float(sys.argv[1])); sys.exit(int(sys.argv[2]))"
            script = ("set -euo pipefail\n" + body + "\nrun_remote_exec() { exec "
                      + shlex.quote(sys.executable) + " -c " + shlex.quote(child)
                      + f" {delay} {native_status}; }}\n"
                      + shlex.quote(sys.executable) + " -c " + shlex.quote(child)
                      + " 60 0 &\nsibling=$!\n"
                      + "trap 'kill \"$sibling\" 2>/dev/null || true; wait \"$sibling\" 2>/dev/null || true' EXIT\n"
                      + f"if remote_exec_with_timeout fixture unused {timeout}; then status=0; "
                      + "else status=$?; fi\n"
                      + "kill -0 \"$sibling\" || exit 99\n"
                      + "printf 'UNRELATED_SIBLING_ALIVE\\n'\nexit \"$status\"\n")
            process = None
            started = time.monotonic()
            interrupted = (signal.SIGTERM, signal.SIGINT, signal.SIGHUP)
            handlers = {sig: signal.getsignal(sig) for sig in interrupted}

            def interrupt(signum, _frame):
                raise RuntimeError(f"Watchdog fixture interrupted by signal {signum}")

            for sig in interrupted:
                signal.signal(sig, interrupt)
            try:
                # Register the exact owned process group before a signal can interrupt setup.
                mask = signal.pthread_sigmask(signal.SIG_BLOCK, interrupted)
                try:
                    with output.open("w") as log:
                        process = subprocess.Popen(
                            ["/bin/bash", "-c", script], stdout=log, stderr=log,
                            start_new_session=True,
                            preexec_fn=lambda: signal.pthread_sigmask(signal.SIG_SETMASK, mask))
                finally:
                    signal.pthread_sigmask(signal.SIG_SETMASK, mask)
                status = process.wait(timeout=5)
                elapsed = time.monotonic() - started
                survivors = group_members(process.pid)
                record = {"delay": delay, "native_status": native_status, "timeout": timeout,
                          "status": status, "elapsed_seconds": elapsed, "survivors": survivors,
                          "output": output.read_text(),
                          "function_sha256": hashlib.sha256(body.encode()).hexdigest()}
                if os.environ.get("IRIS_WATCHDOG_PROOF"):
                    with open(os.environ["IRIS_WATCHDOG_PROOF"], "a") as proof:
                        proof.write(json.dumps(record) + "\n")
                self.assertEqual(status, expected_status, record)
                self.assertEqual(survivors, [], record)
                self.assertIn("UNRELATED_SIBLING_ALIVE", record["output"])
                if delay < timeout:
                    self.assertLess(elapsed, 0.9, "Normal completion waited for the one-second timer")
                else:
                    self.assertIn("remote command timed out after 1s", record["output"])
                    self.assertGreaterEqual(elapsed, 0.9)
                    self.assertLess(elapsed, 3)
            finally:
                # Even the intentionally failing old-code run leaves no owned process behind.
                signal.pthread_sigmask(signal.SIG_BLOCK, interrupted)
                try:
                    if process is not None:
                        for sig in (signal.SIGTERM, signal.SIGKILL):
                            try:
                                os.killpg(process.pid, sig)
                            except ProcessLookupError:
                                break
                            deadline = time.monotonic() + 1
                            while group_members(process.pid) and time.monotonic() < deadline:
                                time.sleep(0.01)
                        process.wait(timeout=2)
                        self.assertEqual(group_members(process.pid), [], "Fixture cleanup failed")
                finally:
                    for sig, handler in handlers.items():
                        signal.signal(sig, handler)
                    signal.pthread_sigmask(signal.SIG_SETMASK, mask)


if __name__ == "__main__":
    unittest.main()
