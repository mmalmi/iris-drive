"""Require the App Store device SDK in the toolchain and in release products."""

import argparse
from pathlib import Path
import plistlib
import re
import subprocess
from zipfile import BadZipFile, ZipFile


MINIMUM_SDK = 27


def check_sdk(sdk, description):
    version = re.fullmatch(r"iphoneos([0-9]+)(?:\.[0-9]+)+", sdk) if isinstance(sdk, str) else None
    if not version or int(version[1]) < MINIMUM_SDK:
        raise ValueError(f"{description} requires the iOS {MINIMUM_SDK} SDK or later (found {sdk or 'no SDK'}).")
    print(f"{description} SDK: {sdk}")


def only_app(apps):
    if len(apps) != 1:
        raise ValueError("Expected exactly one iOS app in the release product.")
    return apps[0]


def check_archive(archive):
    app = only_app(list((archive / "Products/Applications").glob("*.app")))
    for bundle in [app, *sorted((app / "PlugIns").glob("*.appex"))]:
        info = plistlib.loads((bundle / "Info.plist").read_bytes())
        check_sdk(info.get("DTSDKName"), bundle.name)


def check_ipa(path):
    with ZipFile(path) as ipa:
        names = ipa.namelist()
        app_info = only_app([name for name in names if re.fullmatch(r"Payload/[^/]+\.app/Info\.plist", name)])
        extensions = re.compile(re.escape(app_info.removesuffix("Info.plist")) + r"PlugIns/[^/]+\.appex/Info\.plist")
        for name in [app_info, *sorted(filter(extensions.fullmatch, names))]:
            info = plistlib.loads(ipa.read(name))
            check_sdk(info.get("DTSDKName"), Path(name).parent.name)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    actions = parser.add_subparsers(dest="action", required=True)
    actions.add_parser("selected")
    for action in ["archive", "ipa"]:
        actions.add_parser(action).add_argument("path", type=Path)
    args = parser.parse_args()
    try:
        if args.action == "selected":
            version = subprocess.check_output(["xcrun", "--sdk", "iphoneos", "--show-sdk-version"], text=True).strip()
            check_sdk("iphoneos" + version, "Selected Xcode")
        elif args.action == "archive":
            check_archive(args.path)
        else:
            check_ipa(args.path)
    except (ValueError, OSError, plistlib.InvalidFileException, BadZipFile, subprocess.CalledProcessError) as error:
        parser.exit(1, f"{error}\n")


if __name__ == "__main__":
    main()
