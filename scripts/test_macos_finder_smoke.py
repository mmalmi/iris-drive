"""Finder checks must read the currently launched shell's captured stderr."""
import os
from pathlib import Path
import subprocess
import tempfile
import unittest


class FinderSmokeLogs(unittest.TestCase):
    def revealed_path(self, owner_log, debug_log):
        helper = Path(__file__).with_name("macos-finder-smoke.sh")
        return subprocess.run(
            ["bash", "-euo", "pipefail", "-c",
             'source "$1"; latest_revealed_drive_path', "bash", str(helper)],
            env={**os.environ, "APP_STDERR": str(owner_log), "APP_DEBUG_LOG": str(debug_log)},
            capture_output=True, text=True,
        )

    def test_release_shell_without_writable_debug_log(self):
        with tempfile.TemporaryDirectory() as folder:
            root = Path(folder)
            log = root / "owner.stderr"
            path = "/CloudStorage/Iris Drive profile-ä"
            log.write_text("Iris Drive[123] Iris Drive mounted drive folder revealed: " + path + "\n")
            result = self.revealed_path(log, root / "unwritable-debug.log")
            self.assertEqual(result.returncode, 0, result.stderr)
            self.assertEqual(result.stdout.strip(), path)

    def test_previous_shell_debug_log_cannot_select_an_old_root(self):
        with tempfile.TemporaryDirectory() as folder:
            root = Path(folder)
            debug = root / "initial-debug.log"
            debug.write_text("Iris Drive mounted drive folder revealed: /old-profile\n")
            owner = root / "owner.stderr"
            owner.write_text("Iris Drive mounted drive folder revealed: /owner-profile\n")
            self.assertEqual(self.revealed_path(owner, debug).stdout.strip(), "/owner-profile")

    def test_missing_owner_reveal_is_a_failure(self):
        with tempfile.TemporaryDirectory() as folder:
            root = Path(folder)
            owner = root / "owner.stderr"
            owner.write_text("Iris Drive started\n")
            debug = root / "initial-debug.log"
            debug.write_text("Iris Drive mounted drive folder revealed: /old-profile\n")
            result = self.revealed_path(owner, debug)
            self.assertNotEqual(result.returncode, 0)
            self.assertEqual(result.stdout, "")


if __name__ == "__main__":
    unittest.main()
