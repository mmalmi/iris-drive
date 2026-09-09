"""Run the production registration helpers with task-owned command fixtures."""
import os
import shlex
import subprocess
import tempfile
import unittest
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]


class MacAppRegistrationTests(unittest.TestCase):
    def test_registration_preserves_foreign_apps_and_plugins(self):
        source = (ROOT / "scripts/macos-dev-app.sh").read_text()
        helpers = source[source.index("register_app_bundle() {"):source.index("build_xcode_app() {")]
        for own_registered in (False, True):
            with self.subTest(own_registered=own_registered), tempfile.TemporaryDirectory() as tmp:
                base = Path(tmp)
                owned, foreign, built = (base / name for name in ("Owned.app", "Foreign.app", "Built.app"))
                plugin = owned / "Contents/PlugIns/IrisDriveFileProvider.appex"
                foreign_plugin = foreign / "Contents/PlugIns/IrisDriveFileProvider.appex"
                plugin.mkdir(parents=True)
                foreign_plugin.mkdir(parents=True)
                built.mkdir()
                sentinel = foreign / "keep.txt"
                sentinel.write_text("unrelated app bytes")
                calls = base / "calls"
                lsregister = base / "lsregister"
                lsregister.write_text('#!/bin/bash\nprintf "%s\\t" lsregister "$@" >> "$TASK_CALLS"\nprintf "\\n" >> "$TASK_CALLS"\n')
                lsregister.chmod(0o700)
                script = base / "run.sh"
                script.write_text("#!/bin/bash\nset -Eeuo pipefail\n"
                    + f'launch_services_tool() {{ printf "%s\\n" {shlex.quote(str(lsregister))}; }}\n'
                    + '''record() { printf '%s\\t' "$@" >> "$TASK_CALLS"; printf '\\n' >> "$TASK_CALLS"; }
log() { :; }
mdfind() { printf '%s\\n' "$TASK_FOREIGN"; }
find() { printf '%s\\n' "$TASK_FOREIGN"; }
rm() {
  record rm "$@"
  if [[ "$#" == 2 && "$1" == -rf && "$2" == "$TASK_FOREIGN" ]]; then /bin/rm -rf -- "$2"; fi
}
pluginkit() {
  record pluginkit "$@"
  if [[ "$1" == -m ]]; then
    printf 'plugin\\tversion\\tstate\\t%s\\n' "$TASK_FOREIGN_PLUGIN"
    if [[ "$TASK_OWN_REGISTERED" == true ]]; then printf 'plugin\\tversion\\tstate\\t%s\\n' "$TASK_PLUGIN"; fi
  fi
}
''' + helpers + '\nregister_app_bundle "$TASK_OWNED" "$TASK_BUILT"\nregister_fileprovider_plugin "$TASK_OWNED"\n')
                result = subprocess.run(["bash", str(script)], text=True, capture_output=True, timeout=10,
                    env=dict(os.environ, TASK_CALLS=str(calls), TASK_FOREIGN=str(foreign),
                        TASK_FOREIGN_PLUGIN=str(foreign_plugin), TASK_PLUGIN=str(plugin),
                        TASK_OWNED=str(owned), TASK_BUILT=str(built),
                        TASK_OWN_REGISTERED=str(own_registered).lower()))
                self.assertEqual(result.returncode, 0, result.stdout + result.stderr)
                self.assertTrue(sentinel.is_file(), "another app was deleted")
                self.assertEqual(sentinel.read_text(), "unrelated app bytes")
                commands = [line.rstrip("\t").split("\t") for line in calls.read_text().splitlines()]
                self.assertFalse(any(row[0] == "rm" for row in commands), commands)
                self.assertNotIn(["lsregister", "-u", str(foreign)], commands)
                self.assertNotIn(["pluginkit", "-r", str(foreign_plugin)], commands)
                self.assertIn(["lsregister", "-f", "-R", "-trusted", str(owned)], commands)
                self.assertEqual(["pluginkit", "-a", str(plugin)] in commands, not own_registered)


if __name__ == "__main__":
    unittest.main()
