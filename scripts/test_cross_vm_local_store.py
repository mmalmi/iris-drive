#!/usr/bin/env python3
"""Exercise the actual local-store helper without starting native daemons."""
import os
from pathlib import Path
import shlex
import subprocess
import tempfile
import unittest

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


if __name__ == '__main__':
    unittest.main()
