"""Verify exact hosted inputs and the completed independent prefix before resuming."""
import hashlib, json, os, re, subprocess as sp
from pathlib import Path

expected = [
    'IrisDriveIOSShareExtensionTests',
    *['IrisDriveIOSUITests/IrisDriveIOSUITests/'+m for m in [
      'testWelcomeRoutesWithoutSetupTitle','testCancelSignInRequestRequiresConfirmation','testSettingsLogoutRequiresConfirmation',
      'testLinkThisDeviceFromWelcome','testShareSheetImportsFileFromExternalSender','testAwaitingApprovalViewVisible',
      'testApprovedLinkedDeviceLeavesWaiting','testCreateProfileFromWelcome','testCreateProfileWithUsernameCanSkipProfilePhoto',
      'testOpenIrisAppsLoadsBrowserWithoutConnectionError','testMyDriveShowsSyncStatusWithoutMobilePauseControls']]
]
boundary = 'IrisDriveIOSUITests/IrisDriveIOSUITests/testOpenDriveFolderInFilesApp'

def check_log(log):
    blocks = log.split('Command line invocation:')[1:]
    if len(blocks) < len(expected)+1: raise ValueError('Previous gate did not reach the independent Files boundary')
    covered=[]
    for name,section in zip(expected+[boundary],blocks):
        selected=re.search(r'-only-testing:([^"\s]+)',section)
        if not selected or selected.group(1)!=name: raise ValueError('Unexpected previous test order or selection')
        if name==boundary:
            if not re.search(r"^Test Case .*testOpenDriveFolderInFilesApp.* started\.",section,re.M):
                raise ValueError('Previous gate did not reach the Files test')
            break
        passed=re.findall(r"^Test Case '([^']+)' passed \(",section,re.M)
        if ('** TEST EXECUTE SUCCEEDED **' not in section or '** TEST EXECUTE FAILED **' in section or not passed
                or not re.search(r'Executed [1-9][0-9]* tests?, with 0 failures',section)):
            raise ValueError('Missing successful execution for '+name)
        if name!=expected[0] and not any(name.split('/')[-1]+']' in value for value in passed):
            raise ValueError('Actual test did not match selection')
        covered.append({'selection':name,'casesPassed':len(passed)})
    return covered

def main():
    out=Path(os.environ['CHECK_OUTPUT']); source=os.environ['SOURCE_COMMIT']; repository=os.environ['GITHUB_REPOSITORY']
    previous=os.environ['IOS_PREFIX_RUN']; products=os.environ['IOS_PREFIX_PRODUCTS_RUN']
    if not previous.isdigit() or not products.isdigit(): raise ValueError('Invalid hosted run identity')
    gh=lambda *args: sp.check_output(['gh',*args],text=True)
    get=lambda suffix: json.loads(gh('api','repos/'+repository+'/'+suffix))
    run=get('actions/runs/'+previous)
    if (run['status']!='completed' or run['event']!='workflow_dispatch' or run['head_sha']!=source
            or run['head_repository']['full_name']!=repository or run['path']!='.github/workflows/release-products.yml'):
        raise ValueError('Previous run is not the canonical workflow at the exact candidate')
    jobs=get('actions/runs/'+previous+'/jobs?per_page=100')['jobs']
    [job]=[j for j in jobs if j['name']=='ios-check / ios']
    actions=gh('run','view',previous,'--repo',repository,'--job',str(job['id']),'--log')
    for platform in ['ios-sim-debug','ios-sim-release','macos-release']:
        item='drive-'+platform+'-'+source
        if not re.search(r'run-id: '+products+r'\s*\n[^\n]*name: '+re.escape(item)+r'\s*\n',actions):
            raise ValueError('Previous gate has different inputs: '+platform)
    if 'IOS_CLI_OWNER_AUTHORIZATION_OBSERVED' not in actions: raise ValueError('Missing arrival through the timed CLI approval')
    [artifact]=[a for a in get('actions/runs/'+previous+'/artifacts?per_page=100')['artifacts']
                    if a['name']=='ios-check-'+source and not a['expired']]
    prev=out/'verified-prefix'
    gh('run','download',previous,'--repo',repository,'--name',artifact['name'],'--dir',str(prev))
    log=(prev/'xcode-tests.log').read_bytes()
    covered=check_log(log.decode(errors='replace'))
    shell=out/'ios-gui-linking-smoke.sh'; text=shell.read_text()
    first='run_ui_test "IrisDriveIOSShareExtensionTests"\n'
    last='run_ui_test "'+boundary+'"\n'
    if text.count(first)!=1 or text.count(last)!=1: raise ValueError('Canonical scenario boundary changed')
    # All removed checks executed successfully before the prior gate began Files.
    # Files seeds its own owner. Give it a fresh app plus the standard relay/blob fixtures.
    text=text[:text.index(first)]+'start_local_relay\nstart_ios_smoke_blossom\nreset_sim_app_state\n'+text[text.index(last):]
    shell.write_text(text)
    result={'commit':source,'productRun':products,'previousRun':previous,'artifactDigest':artifact.get('digest'),
            'xcodeLogSha256':hashlib.sha256(log).hexdigest(),'completed':covered,'nextTest':boundary}
    if not str(result['artifactDigest']).startswith('sha256:'): raise ValueError('Previous evidence artifact has no digest')
    (out/'verified-prefix.json').write_text(json.dumps(result,indent=2))
    print('Verified',len(covered),'previous Xcode invocations and',sum(x['casesPassed'] for x in covered),'passing cases; resuming Files on clean simulator')
if __name__=='__main__':main()
