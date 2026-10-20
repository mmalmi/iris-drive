import os
from pathlib import Path
import plistlib
import subprocess
import tempfile
import unittest


class MacSmokeProfileTests(unittest.TestCase):
    def parent(self, entitlements):
        with tempfile.TemporaryDirectory() as folder:
            root = Path(folder)
            fixture = root / "entitlements.plist"
            fixture.write_bytes(plistlib.dumps(entitlements))
            command = root / "codesign"
            command.write_text('#!/bin/sh\ncat "$TEST_ENTITLEMENTS"\n')
            command.chmod(0o755)
            helper = Path(__file__).with_name("lib") / "macos-smoke-profile.sh"
            result = subprocess.run(
                ["bash", "-euo", "pipefail", "-c",
                 'source "$1"; macos_smoke_profile_parent "/signed/Iris Drive.app"',
                 "bash", str(helper)], capture_output=True, text=True,
                env={**os.environ, "HOME": "/test-user", "TMPDIR": "/test-temp",
                     "PATH": str(root) + os.pathsep + os.environ["PATH"],
                     "TEST_ENTITLEMENTS": str(fixture)},
            )
            return result

    def test_sandbox_profile_uses_its_signed_shared_container(self):
        result = self.parent({"com.apple.security.app-sandbox": True,
                              "com.apple.security.application-groups": ["TEST.to.iris.drive"]})
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertEqual(result.stdout.strip(),
                         "/test-user/Library/Group Containers/TEST.to.iris.drive/Iris Drive Smoke")

    def test_unsigned_profile_keeps_temporary_directory(self):
        result = self.parent({})
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertEqual(result.stdout.strip(), "/test-temp")

    def test_missing_signed_container_fails(self):
        result = self.parent({"com.apple.security.app-sandbox": True})
        self.assertNotEqual(result.returncode, 0)
        self.assertEqual(result.stdout, "")


if __name__ == "__main__":
    unittest.main()
