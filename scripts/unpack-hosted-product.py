#!/usr/bin/env python3
"""Extract a verified product archive without links or paths outside its destination."""
import pathlib
import shutil
import sys
import tarfile


def unpack(archive, destination):
    root = pathlib.Path(destination).resolve()
    if not root.is_dir() or any(root.iterdir()):
        raise ValueError('Product destination must be an empty directory')
    with tarfile.open(archive) as stream:
        members = stream.getmembers()
        for member in members:
            target = (root / member.name).resolve()
            if (target != root and root not in target.parents) or not (
                member.isdir() or member.isfile()
            ):
                raise ValueError('Unsafe product archive member')
        for member in members:
            target = root / member.name
            if member.isdir():
                target.mkdir(parents=True, exist_ok=True)
            else:
                target.parent.mkdir(parents=True, exist_ok=True)
                with stream.extractfile(member) as source, target.open('xb') as output:
                    shutil.copyfileobj(source, output)
                target.chmod(member.mode & 0o777)


if __name__ == '__main__':
    unpack(*sys.argv[1:])
