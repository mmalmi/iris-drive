"""Run the provider byte check inside the same physical app group as the UI test."""
from pathlib import Path
import plistlib
import subprocess
import sys
import tempfile


def main():
    run_file, destination, bundle_id, group, filename, digest, log = sys.argv[1:]
    target_name = "IrisDriveIOSShareExtensionTests"
    case = "PhysicalLinkedFileTests/testReadLinkedFileFromDeviceProvider"
    with Path(run_file).open("rb") as stream:
        run = plistlib.load(stream)
    run[target_name].setdefault("EnvironmentVariables", {}).update({
        "IRIS_XCUITEST_ISOLATED_DEVICE": "1",
        "IRIS_XCUITEST_PROVIDER_BUNDLE": bundle_id,
        "IRIS_XCUITEST_PROVIDER_GROUP": group,
        "IRIS_XCUITEST_PEER_FILE": filename,
        "IRIS_XCUITEST_PEER_SHA256": digest,
    })
    # __TESTROOT__ paths in the test description are relative to its containing directory.
    with tempfile.NamedTemporaryFile(suffix=".xctestrun", dir=Path(run_file).parent) as scoped:
        plistlib.dump(run, scoped)
        scoped.flush()
        with Path(log).open("wb") as output:
            subprocess.run([
                "xcodebuild", "-xctestrun", scoped.name, "-destination", f"platform=iOS,id={destination}",
                "-parallel-testing-enabled", "NO", f"-only-testing:{target_name}/{case}", "test-without-building",
            ], stdout=output, stderr=subprocess.STDOUT, check=True, timeout=120)
    passed = f"{case.split('/')[-1]}]' passed"
    if passed not in Path(log).read_text(errors="replace"):
        raise RuntimeError("The physical provider byte check did not run to completion")


if __name__ == "__main__":
    main()
