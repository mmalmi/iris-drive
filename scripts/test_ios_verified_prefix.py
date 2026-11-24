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

    def test_a_retried_invocation_counts_only_by_its_last_attempt(self):
        suite, names, boundary = (HELPERS[k] for k in ('FIRST_SUITE', 'PREFIX', 'BOUNDARY'))
        def section(name, ok=True):
            case = 'testOne' if name == suite else name.rsplit('/', 1)[-1]
            result = 'passed' if ok else 'failed'
            return (f'Command line invocation:\n xcodebuild "-only-testing:{name}"\n'
                    f"Test Case '-[Tests {case}]' started.\nTest Case '-[Tests {case}]' {result} (0.1 seconds).\n"
                    f"Executed 1 test, with {0 if ok else 1} failures\n"
                    f"** TEST EXECUTE {'SUCCEEDED' if ok else 'FAILED'} **\n")
        retried = names[2]
        log = ''.join(section(name, ok=False) + section(name) if name == retried else section(name)
                      for name in names) + section(boundary)
        verify = HELPERS['check_log']
        self.assertEqual([row['selection'] for row in verify(log)], names)
        last_failed = log.replace(section(retried), section(retried, ok=False) * 2)
        with self.assertRaises(ValueError):
            verify(last_failed)

    def test_picks_the_evidence_uploaded_by_the_checked_attempt(self):
        jobs = [{'name': 'ios-check / ios', 'id': 22, 'started_at': '2026-11-24T11:37:00Z',
                 'completed_at': '2026-11-24T12:20:00Z'}]
        def artifact(ident, created, name='ios-check-candidate', expired=False):
            return {'id': ident, 'name': name, 'created_at': created, 'expired': expired,
                    'digest': 'sha256:' + str(ident)}
        artifacts = [artifact(1, '2026-11-24T10:15:00Z'), artifact(2, '2026-11-24T11:05:00Z'),
                     artifact(3, '2026-11-24T12:19:58Z'), artifact(4, '2026-11-24T12:00:00Z', 'other')]
        select = HELPERS['attempt_evidence']
        job, chosen = select(jobs, artifacts, 'candidate')
        self.assertEqual((job['id'], chosen['id']), (22, 3))
        for broken in ([artifacts[0], artifacts[1]],
                       [*artifacts, artifact(5, '2026-11-24T12:01:00Z')],
                       [artifact(3, '2026-11-24T12:19:58Z', expired=True)]):
            with self.assertRaises(ValueError):
                select(jobs, broken, 'candidate')

    def test_prefix_run_names_an_optional_attempt(self):
        parse = HELPERS['parse_prefix_run']
        self.assertEqual(parse('45303952692', 3), ('45303952692', 3))
        self.assertEqual(parse('45303952692/2', 3), ('45303952692', 2))
        for invalid in ('', '45303952692/0', '45303952692/4', 'x/1', '1/2/3'):
            with self.assertRaises(ValueError):
                parse(invalid, 3)

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
