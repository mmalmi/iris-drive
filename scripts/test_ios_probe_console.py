import json
import os
from pathlib import Path
import shutil
import subprocess
import sys
import tempfile
import textwrap
import unittest


SMOKE = Path(__file__).with_name("ios-device-iris-apps-smoke.sh")
FAKE_XCRUN = r'''
import json, os, signal, sys, time
from pathlib import Path

args = sys.argv[1:]
state = Path(os.environ["PROBE_TEST_STATE"])
scenario = os.environ["PROBE_TEST_SCENARIO"]

def option(name):
    return args[args.index(name) + 1]

def receipt(data):
    Path(option("--json-output")).write_text(json.dumps(data))

if args[:3] == ["devicectl", "list", "devices"]:
    receipt({"result": {"devices": [{"hardwareProperties": {"productType": "iPhone", "udid": "fixture"},
             "connectionProperties": {"pairingState": "paired"}}]}})
elif args[:4] == ["devicectl", "device", "info", "displays"]:
    receipt({"result": {"backlightState": "on"}})
elif args[:4] == ["devicectl", "device", "install", "app"]:
    pass
elif args[:4] == ["devicectl", "device", "process", "launch"]:
    env = json.loads(option("--environment-variables"))
    def ended(*_):
        if scenario == "wrong-receipt":
            env["IRIS_DRIVE_DEBUG_PROBE_ID"] = "different launch"
        receipt({"info": {"outcome": "success"}, "result": {
            "launchOptions": {"environmentVariables": env}, "process": {"processIdentifier": 123}}})
        (state / "ended").touch()
        sys.exit(0)
    signal.signal(signal.SIGTERM, ended)
    (state / "probe-id").write_text(env["IRIS_DRIVE_DEBUG_PROBE_ID"])
    if "--console" in args:
        if scenario == "replacement":
            (state / "replacement").touch()  # New process reuses the old PID.
            ended()
        while True:
            signal.pause()
elif args[:4] == ["devicectl", "device", "copy", "from"]:
    deadline = time.monotonic() + 5
    while not (state / "probe-id").exists() and time.monotonic() < deadline:
        time.sleep(.01)
    dest = Path(option("--destination"))
    if option("--source").endswith(".png"):
        if scenario == "screenshot-error":
            sys.exit(1)
        if scenario == "interrupt":
            os.kill(os.getppid(), signal.SIGTERM)
        dest.write_bytes(b"fixture screenshot")
    else:
        result = dict(probe_id=(state / "probe-id").read_text(), opened=True, webview_loaded=True,
                      webview_ready_state="complete", webview_title="Iris: The Freedom Toolkit",
                      webview_screenshot_path="debug-iris-apps-webview-aaaaaaaa-bbbb-cccc-dddd-eeeeeeeeeeee.png",
                      gateway_status_present=True, gateway_status_running=True, gateway_status_state="running",
                      webview_body_text="drive chat contacts", route_host="fixture.iris.localhost",
                      webview_final_url="https://fixture.iris.localhost", webview_html_length=1000,
                      webview_html_prefix='<div id="app">')
        for prefix in ("route_http", "route_proxy_http", "embedded_route_http", "embedded_resolve_http",
                       "embedded_cached_resolve_http", "hashtree_status"):
            result[prefix + "_ok"] = True
            result[prefix + ("_code" if prefix == "hashtree_status" else "_status_code")] = 200
        dest.write_text(json.dumps(result))
else:
    (state / "unexpected-device-command").write_text(str(args))
    sys.exit(4)
'''


class IosProbeConsoleTests(unittest.TestCase):
    def exercise(self, scenario):
        temp = tempfile.TemporaryDirectory()
        self.addCleanup(temp.cleanup)
        root = Path(temp.name)
        script = root / "scripts" / SMOKE.name
        script.parent.mkdir()
        shutil.copy2(SMOKE, script)
        binaries = root / "bin"
        binaries.mkdir()
        for name in ("cargo", "xcodegen", "xcodebuild", "uname", "mktemp", "xcrun"):
            path = binaries / name
            if name == "xcrun":
                path.write_text("#!" + sys.executable + "\n" + textwrap.dedent(FAKE_XCRUN))
            elif name == "mktemp":
                path.write_text('#!/bin/sh\nif [ "$1" = -t ]; then exec /usr/bin/mktemp "$TMPDIR/$2.XXXXXX"; fi\nexec /usr/bin/mktemp "$@"\n')
            else:
                path.write_text("#!/bin/sh\n" + ("echo Darwin\n" if name == "uname" else "exit 0\n"))
            path.chmod(0o755)
        (root / "ios/.build/DeviceIrisAppsDerivedData/Build/Products/Release-iphoneos/Iris Drive.app").mkdir(parents=True)
        library = root / "aarch64-apple-ios/debug/libiris_drive_app_core.a"
        library.parent.mkdir(parents=True)
        library.touch()
        env = {**os.environ, "PATH": f"{binaries}:{os.environ['PATH']}", "CARGO_TARGET_DIR": str(root),
               "PROBE_TEST_STATE": str(root), "PROBE_TEST_SCENARIO": scenario, "TMPDIR": str(root),
               "IRIS_DRIVE_IOS_DEVICE": "fixture", "IRIS_DRIVE_IOS_DEVICE_IRIS_APPS_BUILD_LOG": str(root / "build.log")}
        result = subprocess.run(["bash", str(script)], env=env, capture_output=True, text=True, timeout=15)
        expected = {"screenshot-error": 1, "wrong-receipt": 1, "interrupt": 143}.get(scenario, 0)
        self.assertEqual(result.returncode, expected, result.stdout + result.stderr)
        self.assertFalse((root / "unexpected-device-command").exists())
        self.assertTrue((root / "ended").exists(), "owned probe was left running")
        if scenario == "wrong-receipt":
            self.assertIn("matching probe ID", result.stderr)
            self.assertNotIn("probe_pid=", result.stdout)
        else:
            self.assertIn("probe_pid=123", result.stdout)
        entries = dict(line.split("=", 1) for line in result.stdout.splitlines() if "=" in line and " " not in line)
        receipt = json.loads(Path(entries["probe_launch_receipt"]).read_text())
        self.assertEqual(receipt["result"]["process"]["processIdentifier"], 123)
        self.assertEqual(json.loads(Path(entries["probe_result"]).read_text())["probe_id"], (root / "probe-id").read_text())
        if expected == 0:
            self.assertTrue(Path(entries["webview_screenshot"]).is_file())
            self.assertIn("IOS_DEVICE_IRIS_APPS_SMOKE_OK", result.stdout)
        else:
            self.assertNotIn("IOS_DEVICE_IRIS_APPS_SMOKE_OK", result.stdout)
        if scenario == "replacement":
            self.assertTrue((root / "replacement").exists())

    def test_closes_the_owned_console_after_collecting_evidence(self):
        self.exercise("success")

    def test_closes_the_owned_console_when_collecting_the_screenshot_fails(self):
        self.exercise("screenshot-error")

    def test_naturally_exited_console_cannot_target_a_replacement(self):
        self.exercise("replacement")

    def test_interrupt_closes_the_console_and_preserves_exit_status(self):
        self.exercise("interrupt")

    def test_rejects_a_receipt_for_another_probe(self):
        self.exercise("wrong-receipt")


if __name__ == "__main__":
    unittest.main()
