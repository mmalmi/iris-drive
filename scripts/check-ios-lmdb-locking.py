#!/usr/bin/env python3
"""Reject iOS bundles linked to the unsupported System V LMDB lock backend."""

import argparse
from pathlib import Path
import subprocess


FORBIDDEN = {"_semget", "_semctl", "_semop"}
REQUIRED = {"_sem_open", "_sem_wait", "_sem_post", "_sem_close", "_sem_unlink"}


def locking_errors(symbols, *, extension):
    unsupported = symbols & FORBIDDEN
    # Release extensions can have the entire unused LMDB implementation
    # removed by the linker. A retained backend must still be complete.
    missing = REQUIRED - symbols if not extension or symbols & REQUIRED else set()
    return unsupported, missing


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("app", type=Path)
    app = parser.parse_args().app
    binaries = (
        "Iris Drive",
        "PlugIns/IrisDriveFileProvider.appex/IrisDriveFileProvider",
        "PlugIns/IrisDriveShareExtension.appex/IrisDriveShareExtension",
    )
    failures = []
    for relative in binaries:
        executable = app / relative
        images = [executable, *sorted(executable.parent.glob("*.debug.dylib"))]
        symbols = set()
        for image in images:
            output = subprocess.check_output(
                ["xcrun", "nm", "-u", str(image)], text=True, timeout=30
            )
            symbols.update(line.split()[-1] for line in output.splitlines() if line.split())
        unsupported, missing = locking_errors(symbols, extension=relative.startswith("PlugIns/"))
        if unsupported or missing:
            failures.append(
                f"{relative}: unsupported={sorted(unsupported)}, missing={sorted(missing)}"
            )
        else:
            backend = "POSIX locks present" if symbols & REQUIRED else "unused locks stripped"
            print(f"{relative}: {backend}; System V calls absent")
    if failures:
        raise SystemExit("Unsupported iOS LMDB locking:\n" + "\n".join(failures))


if __name__ == "__main__":
    main()
