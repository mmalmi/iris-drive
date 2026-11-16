"""Resume the hosted Files scenario only after verifying its canonical prefix."""
import hashlib
import json
import os
import re
import subprocess
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


def check_log(log):
    sections = log.split('Command line invocation:')[1:]
    if len(sections) <= len(PREFIX):
        raise ValueError('Previous gate did not reach the independent Files boundary')
    covered = []
    for name, section in zip([*PREFIX, BOUNDARY], sections):
        selected = re.search(r'-only-testing:([^"\s]+)', section)
        if not selected or selected.group(1) != name:
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
    if not previous.isdigit() or not all(run.isdigit() for run in products.values()):
        raise ValueError('Invalid hosted run identity')

    def get(suffix):
        return json.loads(gh('api', f'repos/{repository}/{suffix}'))

    api = f'actions/runs/{previous}'
    run = get(api)
    if (run['status'] != 'completed' or run['event'] != 'workflow_dispatch' or run['head_sha'] != source
            or run['head_repository']['full_name'] != repository or run['path'] != '.github/workflows/release-products.yml'):
        raise ValueError('Previous run is not the canonical workflow at the exact candidate')
    job, = (job for job in get(api + '/jobs?per_page=100')['jobs'] if job['name'] == 'ios-check / ios')
    actions_log = gh('run', 'view', previous, '--repo', repository, '--job', str(job['id']), '--log')
    check_inputs(actions_log, source, products)
    artifact, = (item for item in get(api + '/artifacts?per_page=100')['artifacts']
                 if item['name'] == 'ios-check-' + source and not item['expired'])
    if not (artifact.get('digest') or '').startswith('sha256:'):
        raise ValueError('Previous evidence artifact has no digest')
    prefix_dir = output / 'verified-prefix'
    gh('run', 'download', previous, '--repo', repository, '--name', artifact['name'], '--dir', str(prefix_dir))
    xcode_log = (prefix_dir / 'xcode-tests.log').read_bytes()
    covered = check_log(xcode_log.decode(errors='replace'))

    shell = output / 'ios-gui-linking-smoke.sh'
    shell.write_text(resume_script(shell.read_text()))
    receipt = {
        'commit': source, 'productRuns': products, 'previousRun': previous, 'artifactDigest': artifact['digest'],
        'xcodeLogSha256': hashlib.sha256(xcode_log).hexdigest(), 'completed': covered, 'nextTest': BOUNDARY,
    }
    (output / 'verified-prefix.json').write_text(json.dumps(receipt, indent=2))
    print(f"Verified {len(covered)} previous Xcode invocations and {sum(row['casesPassed'] for row in covered)} passing cases")


if __name__ == '__main__':
    main()
