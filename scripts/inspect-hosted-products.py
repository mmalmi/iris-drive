#!/usr/bin/env python3
"""Validate release archives and measure staging without building applications."""
import json
import pathlib
import subprocess
import sys
import tarfile
import tempfile

source, artifacts, output = map(pathlib.Path, sys.argv[1:])
commit = subprocess.check_output(['git', '-C', str(source), 'rev-parse', 'HEAD'], text=True).strip()
tree = subprocess.check_output(['git', '-C', str(source), 'rev-parse', 'HEAD^{tree}'], text=True).strip()
expected = {'macos-release', 'macos-debug', 'ios-release', 'ios-sim-debug',
            'ios-sim-release', 'android', 'linux', 'windows'}
results = []
for receipt_path in sorted(artifacts.glob('*/build-receipt.json')):
    receipt = json.loads(receipt_path.read_text())
    platform = receipt['platform']
    if platform not in expected or platform in {item['platform'] for item in results}:
        raise ValueError('Unexpected or duplicate platform')
    archive = receipt_path.parent / receipt['archive']
    subprocess.run(['node', str(source / 'scripts/hosted-product-receipt.mjs'),
                    str(receipt_path), str(archive), commit, tree, platform], check=True)
    with tarfile.open(archive) as stream:
        members = stream.getmembers()
        declared = sum(member.size for member in members if member.isfile())
        file_count = sum(member.isfile() for member in members)
        if declared > 12 * 1024**3:
            raise ValueError('Archive exceeds inspection staging bound')
    with tempfile.TemporaryDirectory(prefix='drive-product-inspection-') as stage:
        subprocess.run(['python3', str(source / 'scripts/unpack-hosted-product.py'),
                        str(archive), stage], check=True)
        files = [p for p in pathlib.Path(stage).rglob('*') if p.is_file()]
        actual = sum(p.stat().st_size for p in files)
        allocated = sum(p.stat().st_blocks * 512 for p in pathlib.Path(stage).rglob('*'))
        if actual != declared or len(files) != file_count:
            raise ValueError('Extracted payload does not match archive member totals')
    results.append({'platform': platform, 'archiveBytes': archive.stat().st_size,
                    'payloadBytes': actual, 'allocatedBytes': allocated,
                    'files': file_count, 'sha256': receipt['sha256']})
if {item['platform'] for item in results} != expected:
    raise ValueError('Required platform artifacts are missing')
result = {'commit': commit, 'tree': tree, 'platforms': results,
          'totalArchiveBytes': sum(item['archiveBytes'] for item in results),
          'totalPayloadBytes': sum(item['payloadBytes'] for item in results)}
output.write_text(json.dumps(result, indent=2) + '\n')
print(json.dumps(result, indent=2))
