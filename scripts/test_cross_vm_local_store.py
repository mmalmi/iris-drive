#!/usr/bin/env python3
"""Exercise the actual local-store helper without starting native daemons."""
import os
from pathlib import Path
import shlex
import subprocess
import tempfile
import unittest

from test_cross_vm_powershell_transport import decoded_script

HELPER = Path(__file__).resolve().parent / 'lib/cross-vm-local-store.sh'


class CrossVmLocalStoreTests(unittest.TestCase):
    def run_case(self, body, labels=('first', 'remote', 'last'), local=('first', 'last')):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            bases = {name: root / ('iris-drive-e2e-fixture-' + name) for name in labels}
            for base in bases.values():
                base.mkdir()
            config = root / 'normal-config'
            config.write_text('unchanged configuration')
            lines = ['set -Eeuo pipefail', 'source ' + shlex.quote(str(HELPER)),
                     'RUN_ID=fixture', 'LABELS=(' + ' '.join(labels) + ')',
                     'host_value() {', 'case "$1:$2" in']
            for name in labels:
                lines += [name + ':kind) echo posix ;;',
                          name + ':ssh) echo ' + ('local' if name in local else 'remote-host') + ' ;;',
                          name + ':base) echo ' + shlex.quote(str(bases[name])) + ' ;;']
            lines += ['*) return 1 ;;', 'esac', '}', body]
            env = dict(os.environ, HTREE_DATA_DIR='/ambient-data', HTREE_CONFIG_DIR=str(config),
                       TEST_LAST_BASE=str(bases[labels[-1]]), TEST_CONFIG=str(config))
            result = subprocess.run(['bash'], input='\n'.join(lines), text=True,
                                    capture_output=True, env=env, timeout=5)
            self.assertEqual(result.returncode, 0, result.stderr)
            self.assertEqual(config.read_text(), 'unchanged configuration')

    def test_two_local_roles_share_last_local_base_and_keep_parent_configuration(self):
        self.run_case(r'''
prepare_local_shared_store
[[ "$E2E_LOCAL_SHARED_DATA_DIR" == "$(host_value last base)/shared-hashtree" ]]
first="$(run_local_e2e_script 'printf "%s\n" "$HTREE_DATA_DIR"')"
last="$(run_local_e2e_script 'printf "%s\n" "$HTREE_DATA_DIR"')"
[[ "$first" == "$last" && "$first" == "$E2E_LOCAL_SHARED_DATA_DIR" ]]
[[ "$HTREE_DATA_DIR" == /ambient-data ]]
[[ "$(run_local_e2e_script 'printf "%s" "$HTREE_CONFIG_DIR"')" == "$TEST_CONFIG" ]]
if assert_local_shared_store_removed; then exit 1; fi
rm -rf "$(host_value last base)"
assert_local_shared_store_removed
''', labels=('first', 'last', 'remote'))

    def test_single_local_role_is_supported(self):
        self.run_case(r'''
prepare_local_shared_store
[[ "$E2E_LOCAL_SHARED_DATA_DIR" == "$(host_value first base)/shared-hashtree" ]]
rm -rf "$(host_value first base)"
assert_local_shared_store_removed
''', labels=('first', 'remote'), local=('first',))

    def test_no_local_roles_preserve_ambient_selection(self):
        self.run_case(r'''
prepare_local_shared_store
[[ -z "$E2E_LOCAL_SHARED_DATA_DIR" ]]
[[ "$(run_local_e2e_script 'printf "%s" "$HTREE_DATA_DIR"')" == /ambient-data ]]
assert_local_shared_store_removed
''', labels=('remote',), local=())

    def test_existing_store_is_not_adopted_or_modified(self):
        self.run_case(r'''
mkdir "$TEST_LAST_BASE/shared-hashtree"
printf preserved >"$TEST_LAST_BASE/shared-hashtree/sentinel"
if prepare_local_shared_store; then exit 1; fi
[[ -z "$E2E_LOCAL_SHARED_DATA_DIR" ]]
[[ "$(cat "$TEST_LAST_BASE/shared-hashtree/sentinel")" == preserved ]]
''', labels=('last',), local=('last',))



class CrossVmRemoteStoreTests(unittest.TestCase):
    source = (HELPER.parent.parent / 'cross-vm-e2e.sh').read_text()

    def run_command(self, kind, mode, base):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            executable = root / 'idrive'
            executable.write_text('#!/bin/sh\nprintf "%s\\n" "$HTREE_DATA_DIR" "$HTREE_CONFIG_DIR"\n')
            executable.chmod(0o755)
            sections = [('sh_quote() {', 'windows_guest_host_for() {'),
                        ('run_remote_exec() {', '\nremote_exec() {'),
                        ('setup_host() {', '\nidrive_cmd() {'),
                        ('idrive_cmd() {', '\nowner_profile_roster_ops_b64() {'),
                        ('start_daemon() {', '\nstop_daemon() {')]
            bodies = '\n'.join(self.source[self.source.index(start):self.source.index(end)]
                               for start, end in sections)
            script = 'set -Eeuo pipefail\nsource ' + shlex.quote(str(HELPER)) + '\n' + bodies + r'''
host_value() {
  case $2 in
    kind) printf %s "$TEST_KIND" ;;
    ssh) printf %s fixture-remote ;;
    base) [[ "$TEST_MODE" == setup ]] || printf %s "$TEST_BASE" ;;
    config) printf %s "$TEST_BASE/config" ;;
    idrive) printf %s "$TEST_IDRIVE" ;;
    daemon_ssh_pid) printf %s "$TEST_DAEMON_SSH_PID" ;;
    *) printf %s '' ;;
  esac
}
set_host_value() { [[ "$2" != daemon_ssh_pid ]] || TEST_DAEMON_SSH_PID="$3"; }
windows_powershell_command_for() { printf %s powershell; }
host_idrive_override() { printf %s "$TEST_IDRIVE"; }
daemon_relay_args_windows() { :; }
daemon_relay_args_posix() { :; }
# Replace transport, not production command generation. Only the CLI case
# executes, and its executable is our two-line environment observer.
ssh() {
  cat > "$TEST_CAPTURE"
  if [[ "$TEST_KIND:$TEST_MODE" == posix:cli ]]; then bash -se < "$TEST_CAPTURE"; fi
}
remote_exec() { run_remote_exec "$@"; }
remote_exec_with_timeout() { printf %s "$2" > "$TEST_CAPTURE"; }
# The daemon branch's startup liveness probes concern the mock transport only.
sleep() { :; }
kill() { [[ "$1" == -0 ]]; }
TEST_DAEMON_SSH_PID=''
RUN_ID=fixture
E2E_PROFILE=debug
REBUILD_IDRIVE=0
SETUP_REMOTE_TIMEOUT_SECS=1
MOUNT_LABELS=''
case "$TEST_MODE" in
  cli) idrive_cmd fixture status ;;
  daemon) start_daemon fixture; [[ -z "$TEST_DAEMON_SSH_PID" ]] || wait "$TEST_DAEMON_SSH_PID" ;;
  setup) setup_host fixture ;;
esac
'''
            result = subprocess.run(['bash'], input=script, text=True, capture_output=True,
                                    env=dict(os.environ, TEST_KIND=kind, TEST_MODE=mode,
                                             TEST_BASE=base, TEST_IDRIVE=str(executable),
                                             TEST_CAPTURE=str(root / 'command'),
                                             HTREE_DATA_DIR='/ambient-data',
                                             HTREE_CONFIG_DIR='/preserved-config'), timeout=5)
            self.assertEqual(result.returncode, 0, result.stdout + result.stderr)
            command = (root / 'command').read_text()
            if kind == 'windows' and mode != 'setup':
                command = decoded_script(command)
            return result.stdout, command

    def test_remote_posix_cli_and_daemon_share_owned_store(self):
        base = "/tmp/iris-drive-e2e-fixture-owner's data"
        output, cli = self.run_command('posix', 'cli', base)
        self.assertEqual(output.splitlines(), [base + '/shared-hashtree', '/preserved-config'])
        _, daemon = self.run_command('posix', 'daemon', base)
        assignment = cli.splitlines()[0]
        self.assertTrue(assignment.startswith('export HTREE_DATA_DIR='))
        self.assertTrue(daemon.startswith(assignment + '\n'))

    def test_remote_windows_cli_and_special_daemon_share_owned_store(self):
        base = r"C:\owned temp\iris-drive-e2e-fixture-owner's data"
        _, cli = self.run_command('windows', 'cli', base)
        _, daemon = self.run_command('windows', 'daemon', base)
        assignment = "$env:HTREE_DATA_DIR = '" + (base + r'\shared-hashtree').replace("'", "''") + "'"
        for command in (cli, daemon):
            self.assertTrue(command.startswith(assignment + '\n'), command[:200])
            self.assertLess(command.index(assignment), command.index('$idrive ='))
        self.assertIn('& $idrive @daemonArgs', daemon)

    def test_setup_scopes_help_probe_after_creating_owned_base(self):
        for kind in ('posix', 'windows'):
            with self.subTest(kind=kind):
                _, command = self.run_command(kind, 'setup', '')
                marker = ('$env:HTREE_DATA_DIR = Join-Path $base \'shared-hashtree\''
                          if kind == 'windows' else 'export HTREE_DATA_DIR="$base/shared-hashtree"')
                self.assertIn(marker, command)
                create = 'New-Item -ItemType Directory -Path $base' if kind == 'windows' else 'mkdir "$base"'
                self.assertLess(command.index(create), command.index(marker))
                self.assertLess(command.index(marker), command.index('build-idrive-for-e2e'))


if __name__ == '__main__':
    unittest.main()
