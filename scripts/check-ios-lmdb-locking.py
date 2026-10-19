#!/usr/bin/env python3
"""Reject iOS bundles linked to the unsupported System V LMDB lock backend."""

import argparse
from pathlib import Path
import subprocess


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("app", type=Path)
    app = parser.parse_args().app
    forbidden = {"_semget", "_semctl", "_semop"}
    required = {"_sem_open", "_sem_wait", "_sem_post", "_sem_close", "_sem_unlink"}
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
        unsupported = symbols & forbidden
        missing = required - symbols
        if unsupported or missing:
            failures.append(
                f"{relative}: unsupported={sorted(unsupported)}, missing={sorted(missing)}"
            )
        else:
            print(f"{relative}: POSIX reader/writer locks present; System V calls absent")
    if failures:
        raise SystemExit("Unsupported iOS LMDB locking:\n" + "\n".join(failures))


if __name__ == "__main__":
    main()
