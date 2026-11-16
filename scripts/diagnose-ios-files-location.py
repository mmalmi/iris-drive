"""Augment the unchanged, prepared smoke with one callback-only diagnostic."""
from pathlib import Path
import json, os, plistlib, re, subprocess as sp, sys

tools=Path(__file__).parent
output=Path(os.environ['CHECK_OUTPUT'])
test='IrisDriveIOSUITests/IrisDriveIOSUITests/testOpenDriveFolderInFilesApp'
if sys.argv[1]=='prepare':
    trace=output/'files-lookup-diagnostic.dylib'
    sp.run(['xcrun','--sdk','iphonesimulator','clang','-arch','arm64','-mios-simulator-version-min=17.0','-dynamiclib','-fobjc-arc',
            str(tools/'diagnose-ios-files-location.m'),'-framework','Foundation','-framework','FileProvider','-o',str(trace)],check=True)
    sp.run(['codesign','--force','--sign','-',str(trace)],stdout=sp.DEVNULL,check=True)
    script=output/'ios-gui-linking-smoke.sh'
    text=script.read_text()
    old='  python3 - "${python_args[@]}"'
    assert text.count(old)==1
    text=text.replace(old,'  ONLY_DIAGNOSTIC_TEST="$only_testing" python3 - "${python_args[@]}"')
    old='    for env_key in (\n'
    assert text.count(old)==1
    text=text.replace(old,'    import os\n    if os.environ["ONLY_DIAGNOSTIC_TEST"] == '+repr(test)+':\n'
                      +'        target.setdefault("UITargetAppEnvironmentVariables", {})["DYLD_INSERT_LIBRARIES"] = '+repr(str(trace))+'\n'+old)
    old='  rm -f "$run_file"\n  return "$status"'
    assert text.count(old)==1
    text=text.replace(old,'  if [[ "$only_testing" == "'+test+'" ]]; then\n'
                      +'    python3 "'+str(Path(__file__).resolve())+'" collect "$DEVICE_UDID" "$status"\n  fi\n'+old)
    script.write_text(text)
elif sys.argv[1]=='collect':
    device, status=sys.argv[2:]
    data=sp.check_output(['xcrun','simctl','get_app_container',device,'fi.siriusbusiness.drive','data'],text=True).strip()
    path=Path(data)/'Documents/iris-files-diagnostic.jsonl'
    records=[]
    if path.exists():
        for line in path.read_text().splitlines():
            rec=json.loads(line)
            for key in ['errorDomain','underlyingDomain']:
                if not re.fullmatch(r'[A-Za-z0-9._-]{0,100}',rec[key]):rec[key]='unrecognized-domain'
            if rec not in records:records.append(rec)
    summary={'commit':os.environ['SOURCE_COMMIT'],'test':test,'xcodeExit':int(status),'lookups':records}
    (output/'files-lookup.json').write_text(json.dumps(summary,indent=2))
    print('FILES_LOCATION_RESULT',json.dumps(summary),flush=True)
