"""Resume the hosted Files scenario only after verifying its canonical prefix."""
import hashlib
import io
import json
import os
import re
import subprocess
import zipfile
from pathlib import Path


UI_SUITE = 'IrisDriveIOSUITests/IrisDriveIOSUITests/'
FIRST_SUITE = 'IrisDriveIOSShareExtensionTests'
BOUNDARY = UI_SUITE + 'testOpenDriveFolderInFilesApp'
PREFIX = [
    FIRST_SUITE,
    *[UI_SUITE + name for name in (
        'testWelcomeRoutesWithoutSetupTitle', 'testCancelSignInRequestRequiresConfirmation',
        'testSettingsLogoutRequiresConfirmation', 'testLinkThisDeviceFromWelcome',
        'testShareSheetImportsFileFromExternalSender', 'testAwaitingApprovalViewVisible',
        'testApprovedLinkedDeviceLeavesWaiting', 'testCreateProfileFromWelcome',
        'testCreateProfileWithUsernameCanSkipProfilePhoto', 'testOpenIrisAppsLoadsBrowserWithoutConnectionError',
        'testMyDriveShowsSyncStatusWithoutMobilePauseControls',
    )],
]


def selection(section):
    selected = re.search(r'-only-testing:([^"\s]+)', section)
    return selected.group(1) if selected else None


def check_log(log):
    sections = []
    for section in log.split('Command line invocation:')[1:]:
        # The smoke script retries a known runner failure once; judge only the last try.
        if sections and selection(sections[-1]) == selection(section):
            sections[-1] = section
        else:
            sections.append(section)
    if len(sections) <= len(PREFIX):
        raise ValueError('Previous gate did not reach the independent Files boundary')
    covered = []
    for name, section in zip([*PREFIX, BOUNDARY], sections):
        if selection(section) != name:
            raise ValueError('Unexpected previous test order or selection')
        if name == BOUNDARY:
            if not re.search(r'^Test Case .*testOpenDriveFolderInFilesApp.* started\.', section, re.MULTILINE):
                raise ValueError('Previous gate did not enter the Files test')
            break
        passed = re.findall(r"^Test Case '([^']+)' passed \(", section, re.MULTILINE)
        if not (passed and '** TEST EXECUTE SUCCEEDED **' in section and '** TEST EXECUTE FAILED **' not in section
                and re.search(r'Executed [1-9][0-9]* tests?, with 0 failures', section)):
            raise ValueError(f'Missing successful execution for {name}')
        if name != FIRST_SUITE and not any(name.split('/')[-1] + ']' in case for case in passed):
            raise ValueError(f'Actual test did not match {name}')
        covered.append({'selection': name, 'casesPassed': len(passed)})
    return covered


def check_inputs(actions, source, products):
    for platform, run_id in products.items():
        artifact = f'drive-{platform}-{source}'
        if not re.search(r'run-id: ' + run_id + r'\s*\n[^\n]*name: ' + re.escape(artifact) + r'\s*\n', actions):
            raise ValueError(f'Previous gate has different inputs for {platform}')
    if 'IOS_CLI_OWNER_AUTHORIZATION_OBSERVED' not in actions:
        raise ValueError('Missing timed CLI approval proof')


def parse_prefix_run(value, latest_attempt):
    """IOS_PREFIX_RUN is `<run>` (its latest attempt) or `<run>/<attempt>`."""
    match = re.fullmatch(r'([0-9]+)(?:/([0-9]+))?', value)
    attempt = int(match.group(2) or latest_attempt) if match else 0
    if not match or not 1 <= attempt <= latest_attempt:
        raise ValueError('Invalid previous run or attempt')
    return match.group(1), attempt


def attempt_evidence(jobs, artifacts, source):
    """Each attempt uploads its own evidence under the same name; take the one this job uploaded."""
    job, = (job for job in jobs if job['name'] == 'ios-check / ios')
    if not job.get('started_at') or not job.get('completed_at'):
        raise ValueError('Previous attempt has not completed')
    uploaded = [item for item in artifacts if item['name'] == 'ios-check-' + source and not item['expired']
                and job['started_at'] <= item['created_at'] <= job['completed_at']]
    if len(uploaded) != 1:
        raise ValueError('Previous attempt has no single evidence artifact')
    return job, uploaded[0]


def resume_script(text):
    first = f'run_ui_test "{FIRST_SUITE}"\n'
    last = f'run_ui_test "{BOUNDARY}"\n'
    if text.count(first) != 1 or text.count(last) != 1:
        raise ValueError('Canonical scenario boundaries changed')
    # Files resets and seeds its own owner. Recreate only its relay, blob and
    # simulator setup before running the original tail on a clean runner.
    setup = 'start_local_relay\nstart_ios_smoke_blossom\nreset_sim_app_state\n'
    return text[:text.index(first)] + setup + text[text.index(last):]


def gh(*args):
    return subprocess.check_output(['gh', *args], text=True)


def main():
    output = Path(os.environ['CHECK_OUTPUT'])
    source, repository = os.environ['SOURCE_COMMIT'], os.environ['GITHUB_REPOSITORY']
    previous = os.environ['IOS_PREFIX_RUN']
    products = {
        'ios-sim-debug': os.environ['IOS_DEBUG_RUN'],
        'ios-sim-release': os.environ['IOS_RELEASE_RUN'],
        'macos-release': os.environ['IOS_MAC_RUN'],
    }
    if not all(run.isdigit() for run in products.values()):
        raise ValueError('Invalid hosted run identity')

    def get(suffix):
        return json.loads(gh('api', f'repos/{repository}/{suffix}'))

    run_id = previous.split('/', 1)[0]
    if not run_id.isdigit():
        raise ValueError('Invalid hosted run identity')
    api = f'actions/runs/{run_id}'
    run = get(api)
    run_id, attempt = parse_prefix_run(previous, run['run_attempt'])
    if (run['status'] != 'completed' or run['event'] != 'workflow_dispatch' or run['head_sha'] != source
            or run['head_repository']['full_name'] != repository or run['path'] != '.github/workflows/release-products.yml'):
        raise ValueError('Previous run is not the canonical workflow at the exact candidate')
    job, artifact = attempt_evidence(get(f'{api}/attempts/{attempt}/jobs?per_page=100')['jobs'],
                                     get(api + '/artifacts?per_page=100')['artifacts'], source)
    actions_log = gh('run', 'view', run_id, '--repo', repository, '--job', str(job['id']), '--log')
    check_inputs(actions_log, source, products)
    if not (artifact.get('digest') or '').startswith('sha256:'):
        raise ValueError('Previous evidence artifact has no digest')
    archive = subprocess.check_output(['gh', 'api', f'repos/{repository}/actions/artifacts/{artifact["id"]}/zip'])
    if 'sha256:' + hashlib.sha256(archive).hexdigest() != artifact['digest']:
        raise ValueError('Previous evidence artifact does not match its digest')
    prefix_dir = output / 'verified-prefix'
    zipfile.ZipFile(io.BytesIO(archive)).extractall(prefix_dir)
    xcode_log = (prefix_dir / 'xcode-tests.log').read_bytes()
    covered = check_log(xcode_log.decode(errors='replace'))

    shell = output / 'ios-gui-linking-smoke.sh'
    shell.write_text(resume_script(shell.read_text()))
    receipt = {
        'commit': source, 'productRuns': products, 'previousRun': run_id, 'previousAttempt': attempt,
        'artifactDigest': artifact['digest'],
        'xcodeLogSha256': hashlib.sha256(xcode_log).hexdigest(), 'completed': covered, 'nextTest': BOUNDARY,
    }
    (output / 'verified-prefix.json').write_text(json.dumps(receipt, indent=2))
    print(f"Verified {len(covered)} previous Xcode invocations and {sum(row['casesPassed'] for row in covered)} passing cases")


if __name__ == '__main__':
    main()
