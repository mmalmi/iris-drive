import runpy
import unittest
from pathlib import Path


HELPERS = runpy.run_path(str(Path(__file__).with_name('ios-verified-prefix.py')))


class VerifiedIOSPrefixTests(unittest.TestCase):
    def test_requires_actual_passing_cases_and_entry_to_independent_boundary(self):
        suite, names, boundary = (HELPERS[k] for k in ('FIRST_SUITE', 'PREFIX', 'BOUNDARY'))
        def section(name):
            case = 'testOne' if name == suite else name.rsplit('/', 1)[-1]
            return (f'Command line invocation:\n xcodebuild "-only-testing:{name}"\n'
                    f"Test Case '-[Tests {case}]' started.\nTest Case '-[Tests {case}]' passed (0.1 seconds).\n"
                    'Executed 1 test, with 0 failures\n** TEST EXECUTE SUCCEEDED **\n')
        prefix = ''.join(section(name) for name in names)
        after = section(boundary)
        verify = HELPERS['check_log']
        self.assertEqual([row['selection'] for row in verify(prefix + after)], names)
        for invalid in (prefix, prefix.replace('SUCCEEDED', 'FAILED', 1) + after,
                        prefix.replace('Executed 1', 'Executed 0', 1) + after,
                        prefix.replace('testWelcomeRoutesWithoutSetupTitle]','testUnrelated]') + after,
                        prefix + after.replace('started.', 'never started')):
            with self.assertRaises(ValueError):
                verify(invalid)

    def test_binds_each_hosted_product_to_its_own_run(self):
        runs = {'ios-sim-debug':'101', 'ios-sim-release':'102', 'macos-release':'103'}
        log = ''.join(f'run-id: {run}\nname: drive-{key}-candidate\n' for key, run in runs.items())
        log += 'IOS_CLI_OWNER_AUTHORIZATION_OBSERVED'
        verify = HELPERS['check_inputs']
        verify(log, 'candidate', runs)
        for platform in runs:
            with self.assertRaises(ValueError):
                verify(log, 'candidate', {**runs, platform:'104'})

    def test_resumes_the_canonical_tail_verbatim(self):
        canonical = Path(__file__).with_name('ios-gui-linking-smoke.sh').read_text()
        resumed = HELPERS['resume_script'](canonical)
        marker = f'run_ui_test "{HELPERS["BOUNDARY"]}"\n'
        self.assertEqual(resumed.split(marker)[1], canonical.split(marker)[1])
        self.assertIn('start_local_relay\nstart_ios_smoke_blossom\nreset_sim_app_state\n' + marker, resumed)
        self.assertNotIn('run_ui_test "IrisDriveIOSShareExtensionTests"', resumed)
