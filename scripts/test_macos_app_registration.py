"""Run the production registration helpers with task-owned command fixtures."""
import os
import plistlib
import shlex
import subprocess
import tempfile
import unittest
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]


class MacAppRegistrationTests(unittest.TestCase):
    def test_release_smoke_detaches_owned_dmg_after_copy_or_copy_failure(self):
        source = (ROOT / "scripts/macos-release-smoke.sh").read_text()
        setup = source[:source.index("\nusage() {")]
        cleanup = source[source.index("cleanup() {"):source.index("\nstop_launched_apps() {")]
        copy = source[source.index("copy_dmg_app() {"):source.index("\nlaunch_app() {")]
        start = source.index("\nstop_launched_apps\n", source.index('archive_app="$(extract_archive_app)"'))
        calls = source[start:source.index('\njson_result true ""')]
        for copy_fails in (False, True):
            with self.subTest(copy_fails=copy_fails), tempfile.TemporaryDirectory() as tmp:
                base = Path(tmp)
                work, mount = base / "work", base / "owned mount"
                work.mkdir()
                (mount / "Iris Drive.app").mkdir(parents=True)
                (base / "info.plist").write_bytes(plistlib.dumps({"images": []}))
                (base / "attach.plist").write_bytes(plistlib.dumps({
                    "system-entities": [{"mount-point": str(mount)}]}))
                foreign = base / "foreign-attached"
                foreign.write_text("preserve")
                script = base / "run.sh"
                script.write_text(setup + '''
WORK_DIR="$TASK_BASE/work"
DMG_PATH="$TASK_BASE/fixture.dmg"
LSREGISTER="$TASK_BASE/no-lsregister"
log() { :; }
run() { "$@"; }
fail() { exit 1; }
stop_launched_apps() { :; }
verify_app_bundle() { [[ -d "$1" ]]; }
launch_app() { [[ "$2" == "$WORK_DIR/dmg-copy/Iris Drive.app" && -d "$2" ]]; }
hdiutil() {
  printf '%s\\t' "$@" >>"$TASK_BASE/calls"; printf '\\n' >>"$TASK_BASE/calls"
  case "$1" in
    info) cat "$TASK_BASE/info.plist" ;;
    attach) touch "$TASK_BASE/attached"; cat "$TASK_BASE/attach.plist" ;;
    detach)
      [[ "$2" == "$TASK_BASE/owned mount" ]] || return 98
      rm "$TASK_BASE/attached"
      ;;
    *) return 99 ;;
  esac
}
ditto() {
  [[ "$TASK_COPY_FAILS" != true ]] || return 23
  mkdir -p "$2"
}
''' + cleanup + copy + calls)
                result = subprocess.run(["bash", str(script)], text=True, capture_output=True, timeout=10,
                    env=dict(os.environ, TASK_BASE=str(base), TASK_COPY_FAILS=str(copy_fails).lower()))
                self.assertEqual(result.returncode == 0, not copy_fails, result.stdout + result.stderr)
                commands = [line.rstrip("\t").split("\t") for line in (base / "calls").read_text().splitlines()]
                self.assertEqual([row for row in commands if row[0] == "detach"],
                    [["detach", str(mount), "-quiet"]], "the parent EXIT cleanup lost its owned mount")
                self.assertFalse((base / "attached").exists())
                self.assertFalse(work.exists())
                self.assertEqual(foreign.read_text(), "preserve")

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
