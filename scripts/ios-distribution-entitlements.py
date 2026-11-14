"""Carry the release entitlements through unsigned hosted archives and verify the signed IPA."""
import argparse
import pathlib
import plistlib
import subprocess
import tempfile

TARGETS = (
    ("PlugIns/IrisDriveFileProvider.appex", "FileProvider/FileProvider.entitlements"),
    ("PlugIns/IrisDriveShareExtension.appex", "ShareExtension/ShareExtension.entitlements"),
    (".", "IrisDriveIOS.entitlements"),
)


def only_app(directory):
    apps = list(directory.glob("*.app"))
    if len(apps) != 1:
        raise ValueError("Expected exactly one Iris Drive application.")
    return apps[0]


def targets(app, sources, group):
    extensions = {p.name for p in (app / "PlugIns").glob("*.appex")}
    if extensions != {pathlib.Path(p).name for p, _ in TARGETS if p != "."}:
        raise ValueError("Unexpected application extensions.")
    for path, declaration in TARGETS:
        bundle = app / path
        info = plistlib.loads((bundle / "Info.plist").read_bytes())
        if not group or info.get("IrisDriveAppGroupIdentifier") != group:
            raise ValueError("The application's configured App Group does not match.")
        text = (sources / declaration).read_text().replace("$(IRIS_DRIVE_IOS_APP_GROUP_IDENTIFIER)", group)
        if "$(" in text:
            raise ValueError("Unresolved release entitlement setting.")
        yield bundle, info["CFBundleIdentifier"], plistlib.loads(text.encode())


def check_entitlements(actual, expected, bundle_id):
    if actual.get("get-task-allow") or not actual.get("application-identifier", "").endswith("." + bundle_id):
        raise ValueError("Expected an iOS distribution signature for this bundle.")
    if any(actual.get(key) != value for key, value in expected.items()):
        raise ValueError("Signed bundle is missing its declared release capabilities.")


def prepare(archive, sources, group, scratch):
    # The caller extracted a verified unsigned product into a temporary directory.
    # Xcode takes capabilities from the code signature when exporting. A temporary
    # ad-hoc signature carries them to Xcode's manual App Store signing step.
    for bundle, _, expected in targets(only_app(archive / "Products/Applications"), sources, group):
        declaration = scratch / "release-entitlements.xcent"
        declaration.write_bytes(plistlib.dumps(expected))
        subprocess.run(["codesign", "--force", "--sign", "-", "--generate-entitlement-der",
                        "--entitlements", declaration, bundle], check=True)


def verify(ipa, sources, group, scratch):
    subprocess.run(["/usr/bin/ditto", "-x", "-k", ipa, scratch], check=True)
    app = only_app(scratch / "Payload")
    subprocess.run(["codesign", "--verify", "--deep", "--strict", app], check=True)
    for bundle, bundle_id, expected in targets(app, sources, group):
        data = subprocess.check_output(["codesign", "-d", "--entitlements", ":-", bundle], stderr=subprocess.DEVNULL)
        check_entitlements(plistlib.loads(data), expected, bundle_id)


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("action", choices=["prepare-archive", "verify-ipa"])
    parser.add_argument("product", type=pathlib.Path)
    parser.add_argument("ios_sources", type=pathlib.Path)
    parser.add_argument("app_group")
    args = parser.parse_args()
    with tempfile.TemporaryDirectory(prefix="iris-drive-ios-entitlements-") as work:
        action = prepare if args.action == "prepare-archive" else verify
        action(args.product, args.ios_sources, args.app_group, pathlib.Path(work))
