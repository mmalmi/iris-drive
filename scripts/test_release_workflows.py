#!/usr/bin/env python3

import os
import shlex
import shutil
import subprocess
import tempfile
import textwrap
import unittest
from pathlib import Path
from typing import Optional


ROOT = Path(__file__).resolve().parents[1]
WORKFLOW_TIMEOUT_SECONDS = 15


class ReleaseWorkflowTests(unittest.TestCase):
    def setUp(self) -> None:
        self.temp = tempfile.TemporaryDirectory()
        self.addCleanup(self.temp.cleanup)
        self.repo = Path(self.temp.name)
        self.scripts = self.repo / "scripts"
        self.bin = self.repo / "bin"
        self.state = self.repo / "state"
        self.scripts.mkdir()
        self.bin.mkdir()
        self.state.mkdir()

    def copy_script(self, name: str) -> Path:
        destination = self.scripts / name
        shutil.copy2(ROOT / "scripts" / name, destination)
        destination.chmod(0o755)
        helper = ROOT / "scripts" / "lib" / "parallel-gate.sh"
        if helper.exists():
            helper_destination = self.scripts / "lib" / "parallel-gate.sh"
            helper_destination.parent.mkdir(parents=True, exist_ok=True)
            shutil.copy2(helper, helper_destination)
        return destination

    def write_executable(self, path: Path, body: str) -> None:
        path.parent.mkdir(parents=True, exist_ok=True)
        path.write_text("#!/usr/bin/env bash\nset -Eeuo pipefail\n" + body, encoding="utf-8")
        path.chmod(0o755)

    def environment(self, **overrides: str) -> dict[str, str]:
        environment = os.environ.copy()
        environment.update(
            {
                "PATH": f"{self.bin}:{environment['PATH']}",
                "RELEASE_WORKFLOW_TEST_STATE": str(self.state),
            }
        )
        environment.update(overrides)
        return environment

    def run_ios_build_fixture(self, script_name: str, configuration: str, *,
                              mode: str = "--build-only", failure: str = "") -> tuple[subprocess.CompletedProcess, str]:
        source = (ROOT / "scripts" / script_name).read_text(encoding="utf-8")
        settings = source[source.index('source "$ROOT/scripts/ios-simulator-signing.sh"'):source.index('OWNER_CONFIG=')]
        resolution = source[source.index("resolve_app_path() {"):source.index("assert_static_app_core_linkage() {")]
        if script_name == "ios-simulator-smoke.sh":
            parser = source[source.index("BUILD_ONLY=0"):source.index("select_simulator() {")]
            cleanup = source[source.index("cleanup() {"):source.index("BUILD_ONLY=0")]
            phase = source[source.index('DEVICE_UDID="$(select_simulator)"'):source.index('xcrun simctl uninstall "$DEVICE_UDID"')]
        else:
            parser = cleanup = ""
            phase = source[source.index("cargo build -p idrive\n"):source.index('XCTESTRUN="$(resolve_xctestrun)"')]
        # Existing archives and apps must not hide a failed fresh build.
        for profile in ("debug", "release"):
            archive = self.state / "target/aarch64-apple-ios-sim" / profile / "libiris_drive_app_core.a"
            archive.parent.mkdir(parents=True, exist_ok=True)
            archive.write_text("older archive")
        for config in ("Debug", "Release"):
            (self.state / f"derived/Build/Products/{config}-iphonesimulator/Iris Drive.app").mkdir(parents=True, exist_ok=True)
        owner = self.state / "owner"
        owner.mkdir(exist_ok=True)
        script = self.scripts / "build-phase.sh"
        self.write_executable(script,
            f"ROOT={shlex.quote(str(ROOT))}\n" + settings + resolution
            + 'DERIVED_DATA="$RELEASE_WORKFLOW_TEST_STATE/derived"\n'
            + 'PROJECT="$RELEASE_WORKFLOW_TEST_STATE/project"\n'
            + 'BUILD_LOG="$RELEASE_WORKFLOW_TEST_STATE/build.log"\n'
            + 'SMOKE_COMPLETED=0\n' + cleanup
            + 'usage() { :; }\n' + parser
            + textwrap.dedent('''\
                record() { printf '<%s>' "$@" >>"$RELEASE_WORKFLOW_TEST_STATE/events"; printf '\\n' >>"$RELEASE_WORKFLOW_TEST_STATE/events"; }
                cargo() {
                  record cargo "$@"
                  [[ "$FIXTURE_FAILURE" != cargo ]] || return 23
                  if [[ "$*" == *iris-drive-app-core* ]]; then
                    printf 'current archive' >"$RUST_STATIC_LIB"
                  fi
                }
                xcodegen() { :; }
                xcodebuild() {
                  record xcodebuild "$@"
                  [[ "$FIXTURE_FAILURE" != xcode ]] || return 24
                  [[ "$(cat "$RUST_STATIC_LIB")" == 'current archive' ]]
                }
                select_simulator() { echo fixture-device; }
                resolve_share_source_app_path() { echo "$DERIVED_DATA"; }
                assert_static_app_core_linkage() { record linkage "$@"; }
                iris_drive_ios_assert_simulator_entitlements() { record entitlements "$@"; }
                wait_for_simulator_boot() { record boot "$@"; }
                xcrun() {
                  record xcrun "$@"
                  [[ "$2" == install ]] || return 94
                  [[ "$FIXTURE_FAILURE" != install ]] || return 25
                }
                stop_ios_smoke_blossom() { [[ "$FIXTURE_FAILURE" != cleanup ]] || return 26; }
                start_ios_smoke_blossom() { echo 'unexpected fixture startup' >&2; exit 95; }
            ''') + phase + '\nSMOKE_COMPLETED=1\n')
        events = self.state / "events"
        events.unlink(missing_ok=True)
        completed = subprocess.run(["/bin/bash", str(script), mode], capture_output=True, text=True,
            timeout=WORKFLOW_TIMEOUT_SECONDS, env=self.environment(
                IRIS_DRIVE_IOS_XCODE_CONFIGURATION=configuration, CARGO_TARGET_DIR=str(self.state / "target"),
                IRIS_DRIVE_IDRIVE_BIN="/usr/bin/true", FIXTURE_FAILURE=failure,
                OWNER_CONFIG=str(owner), LOCAL_RELAY_PID="", LOCAL_RELAY_READY="", LOCAL_RELAY_LOG=""))
        return completed, events.read_text() if events.exists() else ""

    def test_ios_simulator_builds_align_rust_and_xcode_profiles(self) -> None:
        for script in ("ios-simulator-smoke.sh", "ios-gui-linking-smoke.sh"):
            for configuration, profile in (("Debug", "debug"), ("Release", "release")):
                with self.subTest(script=script, configuration=configuration):
                    completed, events = self.run_ios_build_fixture(script, configuration)
                    self.assertEqual(completed.returncode, 0, completed.stderr)
                    cargo = next(line for line in events.splitlines() if line.startswith("<cargo>") and "<iris-drive-app-core>" in line)
                    self.assertEqual("<--release>" in cargo, configuration == "Release", cargo)
                    self.assertIn("<--locked>", cargo)
                    xcode = next(line for line in events.splitlines() if line.startswith("<xcodebuild>"))
                    library = self.state / "target/aarch64-apple-ios-sim" / profile
                    self.assertIn(f"<-configuration><{configuration}>", xcode)
                    self.assertIn(f"<LIBRARY_SEARCH_PATHS={library}>", xcode)
                    self.assertIn(f"<OTHER_LDFLAGS={library}/libiris_drive_app_core.a>", xcode)
                    self.assertNotIn("SWIFT_ACTIVE_COMPILATION_CONDITIONS", xcode)
            for failure, status in (("cargo", 23), ("xcode", 24)):
                with self.subTest(script=script, failure=failure):
                    completed, events = self.run_ios_build_fixture(script, "Release", failure=failure)
                    self.assertEqual(completed.returncode, status, completed.stderr)
                    self.assertNotIn("<linkage>", events)

    def test_ios_install_only_preserves_profile_and_propagates_failures(self) -> None:
        for failure, expected in (("", 0), ("cargo", 23), ("xcode", 24), ("install", 25), ("cleanup", 26)):
            with self.subTest(failure=failure):
                completed, events = self.run_ios_build_fixture("ios-simulator-smoke.sh", "Release", mode="--install-only", failure=failure)
                self.assertEqual(completed.returncode, expected, completed.stderr)
                self.assertNotIn("<idrive>", events, "install-only does not need the host CLI")
                self.assertNotIn("<uninstall>", events)
                self.assertNotIn("<launch>", events)
                self.assertNotIn("IOS_SIMULATOR_SMOKE_OK", completed.stdout)
                self.assertEqual("IOS_INSTALL_OK" in completed.stdout, failure in ("", "cleanup"))
                if failure in ("", "install", "cleanup"):
                    app = self.state / "derived/Build/Products/Release-iphonesimulator/Iris Drive.app"
                    self.assertIn(f"<xcrun><simctl><install><fixture-device><{app}>", events)
                    self.assertLess(events.index("<entitlements>"), events.index("<xcrun>"))

    def test_ios_idle_installs_release_on_the_same_unique_booted_simulator(self) -> None:
        source = (ROOT / "scripts/release-gate.sh").read_text(encoding="utf-8")
        phase = source[source.index('IOS_IDLE_CPU_PREPARED_DEVICE=""'):source.index("run_apple_idle_cpu_gates() {")]
        dispatch_start = source.index("    if idle_cpu_gate_enabled; then")
        dispatch = source[dispatch_start:source.index("    ;;\n  Linux)", dispatch_start)]
        self.write_executable(self.scripts / "ios-simulator-smoke.sh",
            'printf "install %s %s %s\\n" "$IRIS_DRIVE_IOS_XCODE_CONFIGURATION" "$IRIS_DRIVE_IOS_SIMULATOR_DEVICE" "$*" >>"$RELEASE_WORKFLOW_TEST_STATE/events"\n'
            'exit "$FIXTURE_INSTALL_STATUS"\n')
        self.write_executable(self.scripts / "idle-cpu-gate.sh",
            'printf "idle %s %s\\n" "$IRIS_DRIVE_IDLE_CPU_IOS_DEVICE" "$*" >>"$RELEASE_WORKFLOW_TEST_STATE/events"\n')
        self.write_executable(self.bin / "xcrun", 'printf "%s\\n" "$FIXTURE_DEVICES"\n')
        script = self.scripts / "idle-phase.sh"
        self.write_executable(script,
            'run() { "$@"; }; ios_gate_enabled() { return 0; }\n'
            'idle_cpu_gate_enabled() { return 0; }; macos_vm_gate_enabled() { return 0; }\n'
            'run_parallel_functions() {\n'
            '  printf "parallel %s\\n" "$1" >>"$RELEASE_WORKFLOW_TEST_STATE/events"\n'
            '  shift; while [[ $# -gt 0 ]]; do\n'
            '    if [[ "$2" == run_ios_idle_cpu_gate ]]; then "$2"; fi; shift 2\n'
            '  done\n'
            '}\n' + phase
            + 'if [[ "${1:-}" == --measurement-only ]]; then run_ios_idle_cpu_gate; exit; fi\n'
            + dispatch)
        import json
        for preferred, names, install_status, expected in (
            ("", ["Only"], 0, 0), ("Only", ["Other", "Only"], 0, 0),
            ("id-1", ["Same", "Same"], 0, 0), ("", ["A", "B"], 0, 1),
            ("Same", ["Same", "Same"], 0, 1), ("Missing", ["Other"], 0, 1),
            ("", [], 0, 1), ("Only", ["Only"], 27, 27),
        ):
            with self.subTest(preferred=preferred, names=names, install_status=install_status):
                events = self.state / "events"
                events.unlink(missing_ok=True)
                completed = subprocess.run(["/bin/bash", str(script)], cwd=self.repo, capture_output=True, text=True,
                    timeout=WORKFLOW_TIMEOUT_SECONDS, env=self.environment(
                        IRIS_DRIVE_IOS_SIMULATOR_DEVICE=preferred, IRIS_DRIVE_IOS_DEVICE="",
                        FIXTURE_INSTALL_STATUS=str(install_status), FIXTURE_DEVICES=json.dumps({"devices": {"iOS-fixture": [
                            {"name": name, "udid": f"id-{i}", "state": "Booted"} for i, name in enumerate(names)
                        ]}})))
                self.assertEqual(completed.returncode, expected, completed.stderr)
                calls = events.read_text().splitlines() if events.exists() else []
                if expected == 0:
                    udid = "id-1" if preferred in ("Only", "id-1") and len(names) == 2 else "id-0"
                    self.assertEqual(calls, [f"install Release {udid} --install-only", "parallel native-idle-cpu", f"idle {udid} --platform ios"])
                elif install_status:
                    self.assertEqual(len(calls), 1)
                else:
                    self.assertEqual(calls, [])
        events.unlink(missing_ok=True)
        completed = subprocess.run(["/bin/bash", str(script), "--measurement-only"], cwd=self.repo,
            capture_output=True, text=True, timeout=WORKFLOW_TIMEOUT_SECONDS,
            env=self.environment(IOS_IDLE_CPU_PREPARED_DEVICE="unverified-environment-value"))
        self.assertEqual(completed.returncode, 1, completed.stderr)
        self.assertIn("has not been prepared", completed.stderr)
        self.assertFalse(events.exists(), "measurement must not build or sample without preparation")

    def test_ios_gui_cleanup_preserves_incomplete_and_failed_journeys(self) -> None:
        source = (ROOT / "scripts/ios-gui-linking-smoke.sh").read_text(encoding="utf-8")
        cleanup = source[source.index("cleanup() {"):source.index("select_simulator() {")]
        run_ui = source[source.index("run_ui_test() {"):source.index("reset_sim_app_group_state() {")]
        completion = source[source.rindex("\nfi\n") + len("\nfi\n"):]
        products = self.state / "Build/Products"
        products.mkdir(parents=True)
        xctestrun = products / "fixture.xctestrun"
        xctestrun.write_text('<?xml version="1.0"?><plist version="1.0"><dict/></plist>')
        # Run the actual function, temporary xctestrun mutation, cleanup and
        # completion tail. Only the external Xcode process is replaced.
        for case, destination, xcode_status, cleanup_failure, expected in [
            ("nounset", False, 0, False, 1),
            ("failed_xcode", True, 23, False, 23),
            ("failed_cleanup", True, 0, True, 19),
            ("product_and_cleanup_failure", True, 23, True, 23),
            ("completed", True, 0, False, 0),
        ]:
            with self.subTest(case=case):
                owner = self.state / case
                owner.mkdir()
                script = self.scripts / (case + ".sh")
                self.write_executable(
                    script,
                    "SMOKE_COMPLETED=0\n"
                    + f"source {shlex.quote(str(ROOT / 'scripts/lib/ios-blossom-smoke.sh'))}\n"
                    + cleanup + run_ui
                    + f"xcodebuild() {{ return {xcode_status}; }}\n"
                    + "ios_xcuitest_accessibility_session_disabled_after() { return 1; }\n"
                    + ('rm() { if [[ "$1" == -rf ]]; then return 19; fi; command rm "$@"; }\n' if cleanup_failure else "")
                    + ("DESTINATION=fixture\n" if destination else "unset DESTINATION\n")
                    + "run_ui_test fixture/test\n" + completion,
                )
                environment = self.environment(
                    DERIVED_DATA=str(self.state), XCTESTRUN=str(xctestrun),
                    BUILD_LOG=str(self.state / "xcode.log"), SIM_APP_BASE_DIR=str(self.state),
                    DEVICE_UDID="fixture", OWNER_CONFIG=str(owner), LINKED_CONFIG="", MANUAL_LINKED_CONFIG="",
                    OWNER_DAEMON_PID="", LOCAL_RELAY_PID="", LINKED_SYNC_OBSERVER_PID="",
                    MANUAL_LINKED_SYNC_OBSERVER_PID="", OWNER_DAEMON_LOG="", LOCAL_RELAY_READY="",
                    LOCAL_RELAY_LOG="", LOCAL_RELAY_EVENT_LOG="", LINKED_SYNC_OBSERVATION_FILE="",
                    LINKED_ACK_OBSERVATION_FILE="", MANUAL_LINKED_SYNC_OBSERVATION_FILE="",
                    MANUAL_LINKED_ACK_OBSERVATION_FILE="",
                )
                completed = subprocess.run(
                    ["/bin/bash", str(script)], capture_output=True, text=True,
                    timeout=WORKFLOW_TIMEOUT_SECONDS, env=environment,
                )
                self.assertEqual(completed.returncode, expected, completed.stderr)
                self.assertEqual("IOS_GUI_LINKING_SMOKE_OK" in completed.stdout, destination and xcode_status == 0)
                self.assertEqual(owner.exists(), cleanup_failure, "owned fixture cleanup result differed")

    def test_ios_standalone_cleanup_preserves_incomplete_and_selected_success(self) -> None:
        source = (ROOT / "scripts/ios-simulator-smoke.sh").read_text(encoding="utf-8")
        cleanup = source[source.index("cleanup() {"):source.index("BUILD_ONLY=0")]
        removal = source[source.index("safe_remove_sim_container() {"):source.index("simulator_state() {")]
        completion = source[source.rindex("\nfi\n") + len("\nfi\n"):]
        parser = source[source.index("BUILD_ONLY=0"):source.index("select_simulator() {")]
        build_tail = source[source.index('if [[ "$BUILD_ONLY" == "1" ]]; then'):source.index('wait_for_simulator_boot "$DEVICE_UDID"')]
        for case, body, expected in [
            ("nounset", 'unset DEVICE_UDID; safe_remove_sim_container /fixture', 1),
            ("failed_command", 'false', 1),
            ("full", completion, 0),
            ("build_only", 'BUILD_ONLY=1\n' + build_tail, 0),
            ("help", 'usage() { :; }; set -- --help\n' + parser, 0),
        ]:
            with self.subTest(case=case):
                owner = self.state / case
                owner.mkdir()
                script = self.scripts / (case + ".sh")
                self.write_executable(script, "SMOKE_COMPLETED=0\n"
                    + f"source {shlex.quote(str(ROOT / 'scripts/lib/ios-blossom-smoke.sh'))}\n"
                    + cleanup + removal
                    + 'assert_ios_smoke_blossom_handoff() { :; }; approved_json="fixture"\n'
                    + body + "\n")
                completed = subprocess.run(["/bin/bash", str(script)], capture_output=True, text=True,
                    timeout=WORKFLOW_TIMEOUT_SECONDS, env=self.environment(OWNER_CONFIG=str(owner),
                        LOCAL_RELAY_PID="", LOCAL_RELAY_READY="", LOCAL_RELAY_LOG="", DEVICE_UDID="fixture", APP_PATH="fixture"))
                self.assertEqual(completed.returncode, expected, completed.stderr)
                if case == "full":
                    self.assertIn("IOS_SIMULATOR_SMOKE_OK", completed.stdout)
                elif case == "build_only":
                    self.assertIn("IOS_BUILD_OK", completed.stdout)
                self.assertFalse(owner.exists(), "production cleanup did not remove the owned fixture")

    def test_ios_approval_observes_the_running_receiver_without_restarting_it(self) -> None:
        source = (ROOT / "scripts/ios-gui-linking-smoke.sh").read_text(encoding="utf-8")
        # Execute the production approval phase with an already-running receiver.
        # Only its external CLI and simulator boundaries are fixture commands.
        start = source.index("approve_status=0\n", source.index('request_url="$(owner_inbound_request_url'))
        end = source.index('if [[ "${IRIS_DRIVE_IOS_CLI_OWNER_ACK_ONLY', start)
        wait_start = source.index("wait_for_config_status_before() {")
        wait_end = source.index("assert_config_link_state() {", wait_start)
        self.write_executable(
            self.bin / "idrive",
            'printf "%s\\n" "$3" >>"$RELEASE_WORKFLOW_TEST_STATE/events"\n'
            'case "$3" in\n'
            '  approve) printf \'{"roster_size":2,"published_approval_events":1,"approval_publish_error":null}\\n\' ;;\n'
            '  status) printf \'{"profile":{"authorization_state":"authorized"}}\\n\' ;;\n'
            '  *) exit 92 ;;\n'
            'esac\n',
        )
        script = self.scripts / "ios-approval-phase.sh"
        self.write_executable(
            script,
            f"source {shlex.quote(str(ROOT / 'scripts/lib/ios-linking-observer.sh'))}\n"
            + source[wait_start:wait_end]
            + 'assert_ios_smoke_blossom_handoff() { :; }\n'
            + 'launch_sim_app() { echo "receiver restarted during approval" >&2; exit 91; }\n'
            + 'wait_for_approval_ack() {\n'
            + '  [[ "$1" == "$OWNER_CONFIG" && "$3" == "$cli_owner_approval_deadline" ]]\n'
            + '  before_deadline "$3"\n'
            + '  printf "ack\\n" >>"$RELEASE_WORKFLOW_TEST_STATE/events"\n'
            + '}\n'
            + source[start:end],
        )
        completed = subprocess.run(
            [str(script)],
            check=False,
            capture_output=True,
            text=True,
            timeout=WORKFLOW_TIMEOUT_SECONDS,
            env=self.environment(
                IDRIVE=str(self.bin / "idrive"),
                OWNER_CONFIG=str(self.state / "owner"),
                SIM_APP_BASE_DIR=str(self.state / "receiver"),
                request_url="fixture-request",
                owner_fips_peer="fixture-peer",
            ),
        )
        self.assertEqual(completed.returncode, 0, completed.stderr)
        self.assertEqual((self.state / "events").read_text().splitlines(), ["approve", "status", "ack"])

    def test_ios_resets_route_fresh_profiles_to_local_blossom_before_launch(self) -> None:
        source = (ROOT / "scripts/ios-gui-linking-smoke.sh").read_text(encoding="utf-8")
        for function, following in [
            ("reset_sim_app_state", "clear_sim_env"),
            ("reset_sim_app_group_state", "verify_share_sheet_import"),
        ]:
            with self.subTest(function=function):
                body = source[source.index(function + "() {"):source.index(following + "() {")]
                script = self.scripts / "ios-reset.sh"
                self.write_executable(
                    script,
                    'xcrun() {\n'
                    '  if [[ "$2" == get_app_container ]]; then printf "%s/container\\n" "$RELEASE_WORKFLOW_TEST_STATE"; fi\n'
                    '  if [[ "$2" == spawn && "$4" == launchctl ]]; then [[ -f "$SIM_APP_BASE_DIR/local-blossom-configured" && -f "$SIM_APP_BASE_DIR/local-relay-configured" ]] || exit 93; fi\n'
                    '}\n'
                    'safe_remove_sim_container() { rm -rf "$1"; }\n'
                    'clear_sim_env() { [[ -f "$SIM_APP_BASE_DIR/local-blossom-configured" && -f "$SIM_APP_BASE_DIR/local-relay-configured" ]] || exit 93; }\n'
                    'configure_ios_smoke_relay() { touch \"$1/local-relay-configured\"; }\n'
                    'configure_ios_smoke_blossom() { touch "$1/local-blossom-configured"; }\n'
                    + body + function + '\n'
                    + '[[ -f "$SIM_APP_BASE_DIR/local-blossom-configured" && -f "$SIM_APP_BASE_DIR/local-relay-configured" ]]\n',
                )
                completed = subprocess.run(
                    [str(script)], capture_output=True, text=True, timeout=WORKFLOW_TIMEOUT_SECONDS,
                    env=self.environment(DEVICE_UDID="fixture", BUNDLE_ID="fixture", APP_PATH="fixture",
                                         APP_GROUP_ID="fixture", SHARE_SOURCE_BUNDLE_ID="fixture", SHARE_SOURCE_APP_PATH="fixture"),
                )
                self.assertEqual(completed.returncode, 0, completed.stderr)

    def test_ios_reverse_approval_restores_fixtures_after_files_reset(self) -> None:
        source = (ROOT / "scripts/ios-gui-linking-smoke.sh").read_text(encoding="utf-8")
        start = source.index('xcrun simctl terminate', source.index('app_invite="$(python3'))
        end = source.index('linked_json="$(', start)
        script = self.scripts / "ios-reverse-approval-setup.sh"
        self.write_executable(
            script,
            'mkdir -p "$SIM_APP_BASE_DIR" "$LINKED_CONFIG"\n'
            'xcrun() { [[ "$2" == terminate ]]; touch "$SIM_APP_BASE_DIR/stopped"; }\n'
            'configure_ios_smoke_blossom() { [[ -f "$SIM_APP_BASE_DIR/stopped" ]]; touch "$1/blossom"; }\n'
            'configure_ios_smoke_relay() { [[ -f "$SIM_APP_BASE_DIR/stopped" ]]; touch "$1/relay"; }\n'
            + source[start:end]
            + '[[ -f "$SIM_APP_BASE_DIR/blossom" && -f "$SIM_APP_BASE_DIR/relay" ]]\n',
        )
        completed = subprocess.run(
            [str(script)], capture_output=True, text=True, timeout=WORKFLOW_TIMEOUT_SECONDS,
            env=self.environment(DEVICE_UDID="fixture", BUNDLE_ID="fixture",
                                 SIM_APP_BASE_DIR=str(self.state / "owner"), LINKED_CONFIG=str(self.state / "receiver")),
        )
        self.assertEqual(completed.returncode, 0, completed.stderr)

    def test_ios_standalone_routes_fresh_link_profile_to_fixtures(self) -> None:
        source = (ROOT / "scripts/ios-simulator-smoke.sh").read_text(encoding="utf-8")
        end = source.index('SIMCTL_CHILD_IRIS_DRIVE_DEBUG_ACTION=link-device')
        start = source.rindex('safe_remove_sim_container "$SIM_APP_BASE_DIR"', 0, end)
        script = self.scripts / "ios-standalone-link-setup.sh"
        self.write_executable(
            script,
            'safe_remove_sim_container() { rm -rf "$1"; }\n'
            'configure_ios_smoke_blossom() { touch "$1/blossom"; }\n'
            'configure_ios_smoke_relay() { touch "$1/relay"; }\n'
            + source[start:end]
            + '[[ -f "$SIM_APP_BASE_DIR/blossom" && -f "$SIM_APP_BASE_DIR/relay" ]]\n',
        )
        completed = subprocess.run(
            [str(script)], capture_output=True, text=True, timeout=WORKFLOW_TIMEOUT_SECONDS,
            env=self.environment(SIM_APP_BASE_DIR=str(self.state / "receiver")),
        )
        self.assertEqual(completed.returncode, 0, completed.stderr)

    def install_parallel_lane(self, name: str, lane: Optional[str] = None, *, barrier: bool = False) -> None:
        barrier_body = ""
        if barrier:
            barrier_body = textwrap.dedent(
                """
                touch "$RELEASE_WORKFLOW_TEST_STATE/started-$lane"
                for ((attempt = 0; attempt < 100; attempt++)); do
                  started=("$RELEASE_WORKFLOW_TEST_STATE"/started-*)
                  [[ "${#started[@]}" -ge 4 ]] && break
                  sleep 0.02
                done
                [[ "${#started[@]}" -ge 4 ]] || exit 91
                """
            )
        self.write_executable(
            self.scripts / name,
            (f"lane={shlex.quote(lane)}\n" if lane is not None else 'lane="$1"\n')
            + 'printf "start %s\\n" "$lane" >>"$RELEASE_WORKFLOW_TEST_STATE/events"\n'
            + barrier_body
            + 'printf "end %s\\n" "$lane" >>"$RELEASE_WORKFLOW_TEST_STATE/events"\n',
        )

    def install_five_platform_fixture(self) -> Path:
        script = self.copy_script("cross-vm-five-platform-e2e.sh")
        self.install_parallel_lane("ios-simulator-smoke.sh", "ios", barrier=True)
        self.install_parallel_lane("ios-gui-linking-smoke.sh", "ios-gui")
        self.install_parallel_lane("ios-device-iris-apps-smoke.sh", "ios-device")
        self.install_parallel_lane("android-gui-linking-smoke.sh", "android", barrier=True)
        self.install_parallel_lane("mobile-android-smoke.sh", "android-provider")
        self.write_executable(
            self.scripts / "mobile-ios-android-linking-e2e.sh",
            'printf "mobile-physical-link\n" >>"$RELEASE_WORKFLOW_TEST_STATE/events"\n'
            'printf "%s\n" "${IRIS_DRIVE_MOBILE_REUSE_ANDROID_ARTIFACTS-unset}" '
            '>"$RELEASE_WORKFLOW_TEST_STATE/mobile-android-artifact-reuse"\n',
        )
        self.write_executable(
            self.scripts / "macos-vm-android-manual-link-e2e.sh",
            'printf "macos-android-manual-link\n" >>"$RELEASE_WORKFLOW_TEST_STATE/events"\n'
            'printf "%s\n" "${IRIS_DRIVE_MACOS_SSH_HOST-unset}" '
            '>"$RELEASE_WORKFLOW_TEST_STATE/macos-android-host"\n'
            'printf "%s\n" "${IRIS_DRIVE_MACOS_SKIP_GIT_SYNC-unset}" '
            '>"$RELEASE_WORKFLOW_TEST_STATE/macos-android-skip-sync"\n'
            'printf "%s\n" "${IRIS_DRIVE_MOBILE_REUSE_ANDROID_ARTIFACTS-unset}" '
            '>"$RELEASE_WORKFLOW_TEST_STATE/macos-android-reuse"\n',
        )
        self.install_parallel_lane("desktop-gui-smoke.sh", barrier=True)
        self.write_executable(
            self.scripts / "cross-vm-e2e.sh",
            'printf "sync\n" >>"$RELEASE_WORKFLOW_TEST_STATE/events"\n'
            'printf "%s\n" "${IRIS_DRIVE_E2E_SIDELOAD_APPKEYS-unset}" '
            '>"$RELEASE_WORKFLOW_TEST_STATE/sideload-appkeys"\n'
            'printf "%s\n" "${IRIS_DRIVE_E2E_DESKTOP_GUI_LINKING-unset}" '
            '>"$RELEASE_WORKFLOW_TEST_STATE/desktop-gui-linking"\n',
        )
        return script

    def install_five_platform_routing_fixture(self) -> Path:
        script = self.install_five_platform_fixture()
        for name in (
            "desktop-gui-smoke.sh",
            "ios-simulator-smoke.sh",
            "ios-gui-linking-smoke.sh",
            "ios-device-iris-apps-smoke.sh",
            "android-gui-linking-smoke.sh",
            "mobile-android-smoke.sh",
            "macos-vm-android-manual-link-e2e.sh",
        ):
            self.write_executable(
                self.scripts / name,
                'touch "$RELEASE_WORKFLOW_TEST_STATE/smoke-ran"\n',
            )
        self.write_executable(self.bin / "ssh", "exit 0\n")
        return script

    def five_platform_environment(self) -> dict[str, str]:
        return self.environment(
            IRIS_DRIVE_E2E_UBUNTU_HOST="local",
            IRIS_DRIVE_E2E_WINDOWS_HOST="local",
            IRIS_DRIVE_E2E_MACOS_HOST="local",
            IRIS_DRIVE_E2E_IOS_HOST="local",
            IRIS_DRIVE_E2E_ANDROID_HOST="local",
        )

    def test_five_platform_smoke_lanes_start_in_parallel(self) -> None:
        script = self.install_five_platform_fixture()
        completed = subprocess.run(
            [str(script)],
            check=False,
            capture_output=True,
            text=True,
            env=self.five_platform_environment(),
            timeout=WORKFLOW_TIMEOUT_SECONDS,
        )

        self.assertEqual(completed.returncode, 0, completed.stderr)
        events = (self.state / "events").read_text(encoding="utf-8").splitlines()
        self.assertIn("sync", events)
        self.assertIn("mobile-physical-link", events)
        self.assertEqual((self.state / "sideload-appkeys").read_text().strip(), "0")
        self.assertEqual((self.state / "desktop-gui-linking").read_text().strip(), "1")
        self.assertEqual(
            (self.state / "mobile-android-artifact-reuse").read_text().strip(), "0"
        )
        first_end = next(index for index, event in enumerate(events) if event.startswith("end "))
        self.assertEqual(set(events[:first_end]), {"start linux", "start windows", "start ios", "start android"})

    def test_five_platform_parallel_failure_preserves_infrastructure_status(self) -> None:
        script = self.install_five_platform_fixture()
        self.write_executable(self.scripts / "desktop-gui-smoke.sh", 'exit 75\n')
        for name in (
            "ios-simulator-smoke.sh",
            "ios-gui-linking-smoke.sh",
            "ios-device-iris-apps-smoke.sh",
            "android-gui-linking-smoke.sh",
            "mobile-android-smoke.sh",
        ):
            self.write_executable(self.scripts / name, ":\n")
        completed = subprocess.run(
            [str(script)],
            check=False,
            capture_output=True,
            text=True,
            env=self.five_platform_environment(),
            timeout=WORKFLOW_TIMEOUT_SECONDS,
        )

        self.assertEqual(completed.returncode, 75, completed.stderr)
        self.assertFalse((self.state / "events").exists() and "sync" in (self.state / "events").read_text())

    def test_five_platform_reuses_local_functional_smokes_only(self) -> None:
        script = self.install_five_platform_fixture()
        for name in ("desktop-gui-smoke.sh", "ios-device-iris-apps-smoke.sh", "mobile-android-smoke.sh"):
            self.write_executable(
                self.scripts / name,
                f'printf "{name}\\n" >>"$RELEASE_WORKFLOW_TEST_STATE/events"\n',
            )
        completed = subprocess.run(
            [str(script)],
            check=False,
            capture_output=True,
            text=True,
            env=self.five_platform_environment()
            | {
                "IRIS_DRIVE_E2E_LOCAL_IOS_FUNCTIONAL_PRECHECKED": "1",
                "IRIS_DRIVE_E2E_LOCAL_ANDROID_FUNCTIONAL_PRECHECKED": "1",
                "IRIS_DRIVE_E2E_SIDELOAD_APPKEYS": "1",
            },
            timeout=WORKFLOW_TIMEOUT_SECONDS,
        )

        self.assertEqual(completed.returncode, 0, completed.stderr)
        events = (self.state / "events").read_text(encoding="utf-8")
        self.assertNotIn("start ios", events)
        self.assertNotIn("start ios-gui", events)
        self.assertNotIn("start android", events)
        self.assertIn("ios-device-iris-apps-smoke.sh", events)
        self.assertIn("mobile-android-smoke.sh", events)
        self.assertIn("mobile-physical-link", events)
        self.assertIn("sync", events)
        self.assertEqual((self.state / "sideload-appkeys").read_text().strip(), "1")
        self.assertEqual((self.state / "desktop-gui-linking").read_text().strip(), "1")
        self.assertEqual(
            (self.state / "mobile-android-artifact-reuse").read_text().strip(), "1"
        )

    def test_five_platform_records_physical_skip_when_device_hosts_differ(self) -> None:
        script = self.install_five_platform_routing_fixture()
        completed = subprocess.run(
            [str(script)],
            check=False,
            capture_output=True,
            text=True,
            env=self.five_platform_environment()
            | {"IRIS_DRIVE_E2E_ANDROID_HOST": "separate-android-host"},
            timeout=WORKFLOW_TIMEOUT_SECONDS,
        )

        self.assertEqual(completed.returncode, 0, completed.stderr)
        self.assertIn("reason=devices_not_colocated", completed.stdout)
        events = (self.state / "events").read_text(encoding="utf-8")
        self.assertNotIn("mobile-physical-link", events)
        self.assertIn("sync", events)

    def test_five_platform_requires_explicit_shared_physical_host(self) -> None:
        script = self.install_five_platform_routing_fixture()
        completed = subprocess.run(
            [str(script)],
            check=False,
            capture_output=True,
            text=True,
            env=self.five_platform_environment()
            | {
                "IRIS_DRIVE_E2E_ANDROID_HOST": "separate-android-host",
                "IRIS_DRIVE_MOBILE_PHYSICAL_LINKING": "required",
            },
            timeout=WORKFLOW_TIMEOUT_SECONDS,
        )

        self.assertEqual(completed.returncode, 1, completed.stderr)
        self.assertIn("IRIS_DRIVE_MOBILE_PHYSICAL_LINK_HOST", completed.stderr)
        self.assertFalse((self.state / "smoke-ran").exists())
        self.assertFalse((self.state / "events").exists())

    def test_five_platform_routes_manual_android_linking_to_macos_vm(self) -> None:
        script = self.install_five_platform_fixture()
        completed = subprocess.run(
            [str(script)],
            check=False,
            capture_output=True,
            text=True,
            env=self.five_platform_environment()
            | {
                "IRIS_DRIVE_MACOS_SSH_HOST": "test-macos-vm",
                "IRIS_DRIVE_MACOS_VM_FUNCTIONAL_PRECHECKED": "1",
            },
            timeout=WORKFLOW_TIMEOUT_SECONDS,
        )

        self.assertEqual(completed.returncode, 0, completed.stderr)
        events = (self.state / "events").read_text(encoding="utf-8")
        self.assertIn("macos-android-manual-link", events)
        self.assertEqual((self.state / "macos-android-host").read_text().strip(), "test-macos-vm")
        self.assertEqual((self.state / "macos-android-skip-sync").read_text().strip(), "1")
        self.assertEqual((self.state / "macos-android-reuse").read_text().strip(), "1")

    def test_android_default_smoke_uses_one_gradle_instrumentation_graph(self) -> None:
        source = (ROOT / "scripts/android-gui-linking-smoke.sh").read_text(encoding="utf-8")
        self.assertIn(":app:assembleDebug :app:connectedUiTestAndroidTest", source)

    def test_android_link_delivery_has_one_strict_clock_and_separate_sync_timeout(self) -> None:
        source = (ROOT / "scripts/android-gui-linking-smoke.sh").read_text(encoding="utf-8")
        self.assertIn(
            'AUTHORIZATION_TIMEOUT_SECS="${IRIS_DRIVE_ANDROID_AUTHORIZATION_TIMEOUT_SECS:-15}"',
            source,
        )
        self.assertIn("wait_for_android_authorized_until", source)
        self.assertIn("PROVIDER_SYNC_TIMEOUT_SECS", source)
        started = source.index('authorization_started_ms="$(monotonic_milliseconds)"')
        approved = source.index('approve "$request_url"', started)
        authorized = source.index("wait_for_android_authorized_until", approved)
        provider = source.index('wait_for_android_provider_entry "android-smoke.txt"', authorized)
        self.assertLess(started, approved)
        self.assertLess(approved, authorized)
        self.assertLess(authorized, provider)

    def test_android_rust_build_declares_gradle_inputs_and_output(self) -> None:
        source = (ROOT / "android/app/build.gradle.kts").read_text(encoding="utf-8")
        self.assertIn("inputs.files(", source)
        self.assertIn("outputs.file(", source)
        self.assertIn("libiris_drive_app_core.so", source)
        self.assertIn('"mergeDebugJniLibFolders"', source)

    def test_parallel_group_restores_existing_exit_cleanup(self) -> None:
        cleanup_marker = self.state / "cleanup"
        completed = subprocess.run(
            [
                "bash",
                "-c",
                textwrap.dedent(
                    f"""
                    set -Eeuo pipefail
                    source {ROOT / 'scripts/lib/parallel-gate.sh'}
                    trap 'touch {cleanup_marker}' EXIT
                    parallel_group_begin test
                    parallel_group_start quick true
                    parallel_group_wait
                    """
                ),
            ],
            check=False,
            capture_output=True,
            text=True,
            timeout=WORKFLOW_TIMEOUT_SECONDS,
        )

        self.assertEqual(completed.returncode, 0, completed.stderr)
        self.assertTrue(cleanup_marker.exists())

    def test_ios_idle_sample_precedes_macos_process_cleanup(self) -> None:
        source = (ROOT / "scripts/release-gate.sh").read_text(encoding="utf-8")
        idle_group = source.split("run_apple_idle_cpu_gates() {", 1)[1].split("\n}", 1)[0]
        self.assertLess(
            idle_group.index("run_ios_idle_cpu_gate"),
            idle_group.index("run run_macos_idle_cpu_gate"),
        )

    def test_release_gate_skips_only_prechecks_proven_by_fast_tier(self) -> None:
        script = self.copy_script("release-gate.sh")
        call_log = self.state / "calls"
        for command in ("cargo", "node", "just"):
            self.write_executable(
                self.bin / command,
                f'printf "{command} %s\\n" "$*" >>"$RELEASE_WORKFLOW_TEST_STATE/calls"\n',
            )
        self.write_executable(self.bin / "git", 'printf "0\n"\n')
        self.write_executable(self.bin / "uname", 'printf "TestOS\n"\n')

        completed = subprocess.run(
            [str(script)],
            check=False,
            capture_output=True,
            text=True,
            env=self.environment(IRIS_DRIVE_RELEASE_GATE_FAST_PRECHECKED="1"),
            timeout=WORKFLOW_TIMEOUT_SECONDS,
        )

        self.assertEqual(completed.returncode, 0, completed.stderr)
        calls = call_log.read_text(encoding="utf-8")
        self.assertNotIn("node --test", calls)
        self.assertNotIn("cargo fmt --check", calls)
        self.assertNotIn("just structure", calls)
        self.assertNotIn("cargo test --workspace --exclude idrive", calls)
        self.assertNotIn("--test link_input_e2e", calls)
        self.assertIn("cargo test -p idrive --bin idrive --test cli_e2e", calls)
        self.assertIn("cargo test -p idrive --test daemon_sync_matrix", calls)
        self.assertIn("cargo build --workspace --release", calls)


if __name__ == "__main__":
    unittest.main()
