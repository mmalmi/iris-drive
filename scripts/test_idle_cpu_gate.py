#!/usr/bin/env python3
"""Exercise the desktop samplers with controlled process observations."""

import os
import shutil
import subprocess
import sys
import tempfile
import textwrap
import unittest
from pathlib import Path


SCRIPTS = Path(__file__).resolve().parent
POWERSHELL = shutil.which("pwsh") or shutil.which("powershell")


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
