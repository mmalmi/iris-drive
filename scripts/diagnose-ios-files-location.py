"""One Files-opening XCTest against a receipt-verified simulator binary."""
from pathlib import Path
import collections, json, os, plistlib, re, subprocess as sp, sys

root = Path(os.environ['SOURCE_ROOT'])
out = Path(os.environ['CHECK_OUTPUT'])
product = Path(sys.argv[1])
tools = Path(__file__).parent
identifier = 'fi.siriusbusiness.drive'
cli = lambda *a: sp.check_output(a, text=True).strip()
device = next(d['udid'] for rt, ds in json.loads(cli('xcrun','simctl','list','devices','available','--json'))['devices'].items()
              if 'iOS-26-2' in rt for d in ds if 'iPhone' in d['name'])
derived=out/'debug'
derived.mkdir()
product.rename(derived/'Build')
products=derived/'Build'/'Products'
app=products/'Debug-iphonesimulator'/'Iris Drive.app'
testfile,=products.glob('*.xctestrun')
sp.run(['xcrun','simctl','boot',device],stdout=sp.DEVNULL,stderr=sp.DEVNULL)
sp.run(['xcrun','simctl','bootstatus',device,'-b'],stdout=sp.DEVNULL,check=True)
sp.run(['xcrun','simctl','install',device,str(app)],check=True)
app_group=Path(cli('xcrun','simctl','get_app_container',device,identifier,'group.fi.siriusbusiness.drive'))
trace=out/'files-lookup-diagnostic.dylib'
sp.run(['xcrun','--sdk','iphonesimulator','clang','-arch','arm64','-mios-simulator-version-min=17.0','-dynamiclib','-fobjc-arc',
        str(tools/'diagnose-ios-files-location.m'),'-framework','Foundation','-framework','FileProvider','-o',str(trace)],check=True)
sp.run(['codesign','--force','--sign','-',str(trace)],stdout=sp.DEVNULL,check=True)
with testfile.open('rb') as h: config=plistlib.load(h)
for target in config.values():
    if isinstance(target,dict) and target.get('IsUITestBundle'):
        for k in ['EnvironmentVariables','TestingEnvironmentVariables','UITargetAppEnvironmentVariables']:
            target.setdefault(k,{})['IRIS_DRIVE_UI_TEST_BASE_DIR']=str(app_group/'IrisDrive')
        target['UITargetAppEnvironmentVariables']['DYLD_INSERT_LIBRARIES']=str(trace)
with testfile.open('wb') as h: plistlib.dump(config,h)
test='IrisDriveIOSUITests/IrisDriveIOSUITests/testOpenDriveFolderInFilesApp'
with (out/'private-xcode.log').open('wb') as h:
    rc=sp.run(['xcodebuild','-xctestrun',str(testfile),'-destination','platform=iOS Simulator,id='+device,
               '-only-testing:'+test,'test-without-building'],stdout=h,stderr=sp.STDOUT,timeout=480).returncode
raw=(out/'private-xcode.log').read_text(errors='replace')
app_data=Path(cli('xcrun','simctl','get_app_container',device,identifier,'data'))
record=app_data/'Documents'/'iris-files-diagnostic.jsonl'
rows=[]
if record.exists():
    for line in record.read_text().splitlines():
        row=json.loads(line)
        for field in ['errorDomain','underlyingDomain']:
            if not re.fullmatch(r'[A-Za-z0-9._-]{0,100}',row[field]):row[field]='unrecognized-domain'
        if row not in rows:rows.append(row)
facts={'commit':os.environ['SOURCE_COMMIT'],'xcodeExit':rc,'testStarted':bool(re.search(r"^Test Case .*testOpenDriveFolderInFilesApp.* started\.",raw,re.M)),
       'lookupFailedMessage':'Open in Files failed: Files could not check the Iris Drive location.' in raw,'lookups':rows}
(out/'receipt.json').write_text(json.dumps(facts,indent=2))
print('FILES_LOCATION_RESULT',json.dumps(facts))
if not rows or not facts['testStarted']:raise SystemExit('No captured domain callbacks for the required test')
