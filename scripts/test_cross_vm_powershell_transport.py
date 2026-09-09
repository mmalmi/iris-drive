#!/usr/bin/env python3
"""Check whole-script PowerShell input and preservation through an SSH jump."""
import base64
import os
from pathlib import Path
import shlex
import shutil
import subprocess
import tempfile
import unittest

ROOT = Path(__file__).resolve().parents[1]
POWERSHELL = shutil.which('powershell.exe') or shutil.which('pwsh') or shutil.which('powershell')


def helpers():
    source = (ROOT / 'scripts/cross-vm-e2e.sh').read_text()
    return source[source.index('sh_quote() {'):source.index('\nhost_idrive_override() {')]


def command_for(guest=''):
    result = subprocess.run(['bash', '-c', helpers() + '\nwindows_powershell_command_for fixture'],
                            env=dict(os.environ, IRIS_DRIVE_E2E_WINDOWS_GUEST_HOST=guest),
                            text=True, capture_output=True, timeout=5, check=True)
    return result.stdout


def script_stdin(script):
    return subprocess.run(['bash', '-c', helpers() + '\nwindows_script_stdin'], input=script,
                          text=True, encoding='utf-8', capture_output=True,
                          timeout=5, check=True).stdout


def decoded_script(wire):
    encoded = wire.split('FromBase64String("', 1)[1].split('"', 1)[0]
    return base64.b64decode(encoded, validate=True).decode('utf-8')


def cases():
    return [
        ('compound', "$ErrorActionPreference='Stop'\n$values=@()\nif ($true) {\n foreach ($value in @(1,2,3)) {\n  $values += $value * $value\n }\n}\n$values -join ','\n", 0, '1,4,9'),
        ('finally', "$result=@()\ntry {\n $result += 'try'\n} finally {\n $result += 'finally'\n}\n$result -join ','\n", 0, 'try,finally'),
        ('exit', "Write-Output 'before-exit'\nexit 37\nWrite-Output 'after-exit'\n", 37, 'before-exit'),
        ('error', "$ErrorActionPreference='Stop'\nthrow 'expected fixture failure'\n", 1, ''),
        ('error-stops-script', "$ErrorActionPreference='Stop'\nthrow 'expected fixture failure'\nWrite-Output 'MUST_NOT_RUN'\n", 1, ''),
        ('parse', "if ($true) {\n", 1, ''),
        ('error-after-success', "$LASTEXITCODE=0\n$ErrorActionPreference='Stop'\nthrow 'expected fixture failure'\n", 1, ''),
        ('success-after-native', "& (Get-Process -Id $PID).Path -NoProfile -Command 'exit 37'\nWrite-Output 'RECOVERED'\n", 0, 'RECOVERED'),
        ('unicode', "$value='雪☃'\nif ($value.Length -ne 2 -or [int]$value[0] -ne 38634) { throw 'UTF8 input changed' }\nWrite-Output 'UNICODE_OK'\n", 0, 'UNICODE_OK'),
        ('large', "$value=@'\n" + 'x' * (128 * 1024) + "\n'@\nif ($value.Length -ne 131072) { throw 'payload changed' }\nWrite-Output 'LARGE_OK'\n", 0, 'LARGE_OK'),
    ]


class WindowsPowerShellTransportTests(unittest.TestCase):
    def test_jump_preserves_the_fixed_inner_command(self):
        direct = command_for()
        jump = command_for("guest's fixture")
        # Run the actual emitted POSIX jump command, replacing only SSH.
        mock = 'ssh() { printf "%s\\n" "$1"; shift; printf "%s\\n" "$*"; }; '
        result = subprocess.run(['bash', '-c', mock + jump], text=True, capture_output=True,
                                timeout=5, check=True)
        self.assertEqual(result.stdout.splitlines(), ["guest's fixture", direct])

    def test_payload_remains_stdin_instead_of_command_line(self):
        command = command_for()
        self.assertTrue(command.endswith('-Command -'))
        self.assertNotIn('-EncodedCommand', command)
        self.assertLess(len(command), 128)
        for name, script, _, _ in cases():
            with self.subTest(case=name):
                wire = script_stdin(script)
                self.assertTrue(wire.isascii())
                self.assertEqual(wire.count('\n'), 3)
                self.assertEqual(decoded_script(wire), script)

    def test_address_probe_uses_the_same_script_transport(self):
        source = (ROOT / 'scripts/cross-vm-e2e.sh').read_text()
        body = source[source.index('detect_host_fips_addr() {'):source.index('\nsetup_host() {')]
        with tempfile.TemporaryDirectory() as directory:
            capture = Path(directory) / 'wire'
            fixture = helpers() + '\n' + body + r'''
host_value() { case "$2" in kind) echo windows ;; ssh) echo fixture ;; esac; }
ssh() { cat > "$TEST_WIRE"; echo 192.0.2.1; }
detect_host_fips_addr fixture
'''
            result = subprocess.run(['bash', '-c', fixture], text=True, capture_output=True,
                                    env=dict(os.environ, TEST_WIRE=str(capture)), timeout=5)
            self.assertEqual(result.returncode, 0, result.stderr)
            self.assertEqual(result.stdout.strip(), '192.0.2.1')
            self.assertIn('Get-NetIPAddress', decoded_script(capture.read_text()))

    @unittest.skipUnless(POWERSHELL, 'PowerShell is required for actual script execution')
    def test_compound_exit_unicode_and_large_stdin(self):
        arguments = shlex.split(command_for())
        arguments[0] = POWERSHELL
        for name, script, expected_exit, expected_stdout in cases():
            with self.subTest(case=name):
                result = subprocess.run(arguments, input=script_stdin(script), text=True, encoding='utf-8',
                                        capture_output=True, timeout=10)
                self.assertEqual(result.returncode == 0, expected_exit == 0, result.stdout + result.stderr)
                self.assertEqual(result.stdout.strip(), expected_stdout, result.stderr)
                if expected_exit == 0:
                    self.assertEqual(result.stderr, '')


if __name__ == '__main__':
    unittest.main()
