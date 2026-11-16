#!/usr/bin/env python3
"""Select release files from a verified hosted Android or Windows product archive.

Android keeps only the unsigned release APK and AAB. Windows keeps its three
release assets and checks that the app zip holds exactly the hosted publish
directory and that the CLI zip holds the same idrive.exe. Every member is
hashed, so links, duplicates and paths outside the archive are refused.
"""
import hashlib
import pathlib
import sys
import tarfile
import zipfile

ANDROID = {'apk/release/app-release-unsigned.apk', 'bundle/release/app-release.aab'}


def digest(stream, output=None):
    sha = hashlib.sha256()
    for block in iter(lambda: stream.read(1 << 20), b''):
        sha.update(block)
        if output:
            output.write(block)
    return sha.digest()


def zip_digests(path):
    entries = {}
    with zipfile.ZipFile(path) as archive:
        for info in archive.infolist():
            if info.is_dir():
                continue
            name = info.filename.replace('\\', '/')
            if name in entries:
                raise ValueError(f'Duplicate entry in {path.name}')
            with archive.open(info) as member:
                entries[name] = digest(member)
    return entries


def check_windows(target, members, wanted):
    published = {name[len('publish/'):]: value for name, value in members.items()
                 if name.startswith('publish/')}
    if set(members) != wanted | {'publish/' + name for name in published}:
        raise ValueError('Windows product archive has unexpected members')
    app = next(name for name in wanted if name.endswith('-windows-x64.zip'))
    cli = next(name for name in wanted if name.endswith('-windows-msvc.zip'))
    if zip_digests(target / app) != published:
        raise ValueError('Windows app archive differs from the hosted publish directory')
    if zip_digests(target / cli).get('idrive.exe') != published.get('idrive.exe'):
        raise ValueError('Windows CLI archive differs from the hosted idrive.exe')


def select(platform, archive, target, release_names):
    if platform not in ('android', 'windows'):
        raise ValueError('Only Android and Windows products are selected')
    target = pathlib.Path(target)
    if not target.is_dir() or any(target.iterdir()):
        raise ValueError('Selection target must be an empty directory')
    wanted = set(release_names) if platform == 'windows' else ANDROID
    members, written = {}, set()
    with tarfile.open(archive, 'r|gz') as stream:
        for member in stream:
            name = member.name.replace('\\', '/').removeprefix('./')
            path = pathlib.PurePosixPath(name)
            if path.is_absolute() or '..' in path.parts:
                raise ValueError('Unsafe product archive member')
            if member.isdir() or name in ('', '.'):
                continue
            if not member.isfile() or name in members:
                raise ValueError(f'Unexpected product archive member: {name}')
            source = stream.extractfile(member)
            if name in wanted:
                output = target / name
                output.parent.mkdir(parents=True, exist_ok=True)
                with output.open('xb') as handle:
                    members[name] = digest(source, handle)
                output.chmod(member.mode & 0o777)
                written.add(name)
            else:
                members[name] = digest(source)
    if written != wanted:
        raise ValueError('Product archive is missing: ' + ', '.join(sorted(wanted - written)))
    if platform == 'windows':
        check_windows(target, members, wanted)
    return len(members), len(written)


if __name__ == '__main__':
    platform_name, archive_path, target_dir, *names = sys.argv[1:]
    try:
        total, selected = select(platform_name, archive_path, target_dir, names)
    except (ValueError, tarfile.TarError, zipfile.BadZipFile, OSError) as error:
        sys.exit(f'error: {error}')
    print(f'{platform_name} source archive verified; {total} members; {selected} selected release inputs')
