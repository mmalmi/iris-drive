"""Verify test re-signing preserves all application code and resources."""
import hashlib
import json
import re
import sys
import zipfile


def payload_manifest(path):
    result = {}
    with zipfile.ZipFile(path) as archive:
        names = archive.namelist()
        if len(names) != len(set(names)):
            raise ValueError('Duplicate APK entry')
        for name in names:
            if re.fullmatch(r'META-INF/(MANIFEST\.MF|[^/]+\.(SF|RSA|DSA|EC))', name, re.I):
                continue
            digest = hashlib.sha256()
            with archive.open(name) as member:
                for block in iter(lambda: member.read(1024 * 1024), b''):
                    digest.update(block)
            result[name] = {'bytes': archive.getinfo(name).file_size, 'sha256': digest.hexdigest()}
    return result


def verify_payload(original, resigned):
    before, after = payload_manifest(original), payload_manifest(resigned)
    if before != after:
        raise ValueError('Test re-signing changed application payload')
    return hashlib.sha256(json.dumps(before, sort_keys=True, separators=(',', ':')).encode()).hexdigest()


if __name__ == '__main__':
    print(verify_payload(*sys.argv[1:]))
