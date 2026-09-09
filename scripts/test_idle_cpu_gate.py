#!/usr/bin/env python3
"""Exercise the desktop samplers with controlled process observations."""

import base64
import json
import os
import re
import shutil
import subprocess
import sys
import tempfile
import textwrap
import unittest
from pathlib import Path


SCRIPTS = Path(__file__).resolve().parent
POWERSHELL = shutil.which("pwsh") or shutil.which("powershell")


class CrossVmIdleGateTests(unittest.TestCase):
    source = (SCRIPTS / "cross-vm-e2e.sh").read_text()

    def emitted_command(self, kind, config):
        quotes = self.source[self.source.index("sh_quote() {"):
                             self.source.index("windows_guest_host_for() {")]
        gate = self.source[self.source.index("idle_cpu_remote_timeout_secs() {"):
                           self.source.index("write_initial_seed_files() {")]
        script = ("set -euo pipefail\n" + quotes + gate + "\n"
                  + 'host_value() { case $2 in kind) printf %s "$KIND";; '
                  + 'config) printf %s "$CONFIG";; ssh) printf %s fixture;; esac; }\n'
                  + "remote_exec_with_timeout() { printf %s \"$2\"; }\n"
                  + "idle_cpu_gate_label fixture\n")
        result = subprocess.run(["bash", "-c", script], text=True, capture_output=True,
                                env=dict(os.environ, ROOT=str(SCRIPTS.parent),
                                         KIND=kind, CONFIG=config), timeout=5)
        self.assertEqual(result.returncode, 0, result.stderr)
        return result.stdout

    def test_posix_invokes_exact_checkout_sampler_in_owned_config(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            config = root / "owned config's data"
            config.mkdir()
            command = self.emitted_command("posix", str(config))
            # Execute the generated transfer and environment setup. Replace only
            # the sampler invocation, so no real processes or clocks are sampled.
            wrapper = root / "bash"
            wrapper.write_text(f"#!{sys.executable}\n" + textwrap.dedent("""\
                import json, os, sys
                print(json.dumps({'path': sys.argv[1], 'args': sys.argv[2:],
                    'match': os.environ['IRIS_DRIVE_IDLE_CPU_COMMAND_MATCH'],
                    'roles': os.environ['IRIS_DRIVE_IDLE_CPU_REQUIRED_ROLES']}))
                """))
            wrapper.chmod(0o755)
            result = subprocess.run(["/bin/bash", "-c", command], text=True, capture_output=True,
                                    env=dict(os.environ, PATH=f"{root}{os.pathsep}{os.environ['PATH']}"),
                                    timeout=5)
            self.assertEqual(result.returncode, 0, result.stderr)
            invocation = json.loads(result.stdout)
            sampler = Path(invocation["path"])
            self.assertEqual(sampler.parent, config)
            self.assertEqual(sampler.read_bytes(), (SCRIPTS / "idle-cpu-gate.sh").read_bytes())
            self.assertEqual(invocation["match"], str(config))
            self.assertEqual(invocation["roles"], "daemon")
            self.assertEqual(invocation["args"], ["--platform", "auto"])

    def test_windows_transfers_exact_sampler_and_uses_owned_config_filter(self):
        config = r"C:\fixtures\owned config's data"
        command = self.emitted_command("windows", config)
        payload = re.search(r"FromBase64String\('([A-Za-z0-9+/=]+)'\)", command)
        self.assertIsNotNone(payload, "Sampler must come from the invoking checkout")
        self.assertEqual(base64.b64decode(payload.group(1)),
                         (SCRIPTS / "idle-cpu-gate-windows.ps1").read_bytes())
        quoted_config = "'" + config.replace("'", "''") + "'"
        self.assertIn("Join-Path " + quoted_config, command)
        self.assertIn("IRIS_DRIVE_IDLE_CPU_COMMAND_MATCH = " + quoted_config, command)
        self.assertNotIn("$repo", command)

    def test_final_idle_samples_run_serially(self):
        gate = self.source[self.source.rindex("if idle_cpu_gate_enabled; then"):
                           self.source.rindex('\necho\necho "cross-vm e2e passed')]
        parallel = self.source[self.source.index("run_for_all_labels_parallel() {"):
                               self.source.index("idle_cpu_gate_enabled() {")]
        script = ("set -euo pipefail\n" + parallel
                  + "\nLABELS=(one two); calls=0\n"
                  + "idle_cpu_gate_enabled() { return 0; }\n"
                  + "idle_cpu_gate_label() { calls=$((calls + 1)); }\n"
                  + "run_step() { shift; \"$@\"; }\n" + gate
                  + '\n[[ "$calls" == 2 ]]\n')
        result = subprocess.run(["bash", "-c", script], text=True, capture_output=True, timeout=5)
        self.assertEqual(result.returncode, 0, "Idle samples escaped into parallel subshells")


class DesktopIdleCpuTests(unittest.TestCase):
    def run_posix(self, platform, scenario):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            # Only the sampler's clock is virtual; its shell entry point and
            # process parsing/aggregation are the production implementations.
            wrapper = root / "python3"
            wrapper.write_text(f"#!{sys.executable}\n" + textwrap.dedent("""\
                import sys, time
                clock = [0.0]
                time.monotonic = lambda: clock[0]
                def sleep(seconds):
                    clock[0] += seconds
                time.sleep = sleep
                sys.argv = sys.argv[1:]
                exec(compile(sys.stdin.read(), '<idle-cpu-gate>', 'exec'))
                """))
            process_reader = root / "ps"
            process_reader.write_text(f"#!{sys.executable}\n" + textwrap.dedent("""\
                import os
                from pathlib import Path
                state = Path(os.environ['IDLE_TEST_STATE'])
                sample = int(state.read_text()) if state.exists() else 0
                state.write_text(str(sample + 1))
                scenario = os.environ['IDLE_TEST_SCENARIO']
                if scenario != 'disappear' or sample < 2:
                    pid = 102 if scenario == 'restart' and sample >= 2 else 101
                    cpu = sample if scenario == 'busy' else 1.0
                    if scenario == 'counter-reset' and sample >= 2:
                        cpu = 0.0
                    print(f'{pid} 1 {cpu:.2f} /fixture/idrive daemon')
                """))
            wrapper.chmod(0o755)
            process_reader.chmod(0o755)
            environment = {key: value for key, value in os.environ.items()
                           if not key.startswith('IRIS_DRIVE_IDLE_CPU_')}
            environment.update(
                PATH=f"{root}{os.pathsep}{os.environ['PATH']}",
                IDLE_TEST_STATE=str(root / "samples"),
                IDLE_TEST_SCENARIO=scenario,
                IRIS_DRIVE_IDLE_CPU_WARMUP_SECS="0",
                IRIS_DRIVE_IDLE_CPU_DURATION_SECS="20",
                IRIS_DRIVE_IDLE_CPU_INTERVAL_SECS="5",
                IRIS_DRIVE_IDLE_CPU_REQUIRED_ROLES="daemon",
            )
            return subprocess.run(
                ["bash", str(SCRIPTS / "idle-cpu-gate.sh"), "--platform", platform],
                env=environment, capture_output=True, text=True, timeout=10,
            )

    @unittest.skipIf(os.name == "nt", "POSIX shell sampler")
    def test_posix_required_process_survives_entire_sample(self):
        for platform in ("linux", "macos"):
            for scenario in ("stable", "busy", "disappear", "restart", "counter-reset"):
                with self.subTest(platform=platform, scenario=scenario):
                    result = self.run_posix(platform, scenario)
                    self.assertEqual(result.returncode == 0, scenario == "stable",
                                     result.stdout + result.stderr)
                    if scenario in ("disappear", "restart", "counter-reset"):
                        self.assertIn("during idle sample", result.stderr)

    @unittest.skipUnless(POWERSHELL, "PowerShell is required for the Windows sampler")
    def test_windows_required_process_survives_entire_sample(self):
        for scenario in ("stable", "busy", "disappear", "restart", "missing-counter", "null-counter"):
            with self.subTest(scenario=scenario), tempfile.TemporaryDirectory() as directory:
                fixture = Path(directory) / "sample.ps1"
                fixture.write_text(textwrap.dedent("""\
                    param([string]$Gate, [string]$Scenario)
                    $script:Tick = 0
                    $script:Sample = 0
                    function Get-Date {
                        $script:Tick += 1
                        return [datetime]::new(2026, 1, 1).AddSeconds($script:Tick)
                    }
                    function Start-Sleep { param($Seconds) }
                    function Get-CimInstance {
                        param([string]$ClassName)
                        if ($ClassName -eq 'Win32_Process') { $script:Sample += 1 }
                        if ($Scenario -eq 'disappear' -and $script:Sample -ge 2) { return }
                        $processId = if ($Scenario -eq 'restart' -and $script:Sample -ge 2) { 102 } else { 101 }
                        if ($ClassName -eq 'Win32_Process') {
                            [pscustomobject]@{ ProcessId = $processId; Name = 'idrive.exe'; CommandLine = 'idrive.exe daemon' }
                        } elseif ($Scenario -ne 'missing-counter' -or $script:Sample -lt 2) {
                            $cpu = if ($Scenario -eq 'busy') { 50 } else { 0 }
                            if ($Scenario -eq 'null-counter' -and $script:Sample -ge 2) { $cpu = $null }
                            [pscustomobject]@{ IDProcess = $processId; PercentProcessorTime = $cpu }
                        }
                    }
                    $env:IRIS_DRIVE_IDLE_CPU_REQUIRED_ROLES = 'daemon'
                    $env:IRIS_DRIVE_IDLE_CPU_DAEMON_MAX = '10'
                    $env:IRIS_DRIVE_IDLE_CPU_COMMAND_MATCH = ''
                    & $Gate -WarmupSecs 0 -DurationSecs 3 -IntervalSecs 1
                    exit $LASTEXITCODE
                    """))
                result = subprocess.run(
                    [POWERSHELL, "-NoProfile", "-NonInteractive", "-ExecutionPolicy", "Bypass",
                     "-File", str(fixture), str(SCRIPTS / "idle-cpu-gate-windows.ps1"), scenario],
                    capture_output=True, text=True, timeout=15,
                )
                self.assertEqual(result.returncode == 0, scenario == "stable",
                                 result.stdout + result.stderr)
                if scenario in ("disappear", "restart", "missing-counter", "null-counter"):
                    self.assertIn("during idle sample", result.stderr)


if __name__ == "__main__":
    unittest.main()
