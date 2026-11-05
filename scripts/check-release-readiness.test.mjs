import assert from 'node:assert/strict';
import { mkdtempSync, readFileSync, rmSync, writeFileSync } from 'node:fs';
import { tmpdir } from 'node:os';
import path from 'node:path';
import { spawnSync } from 'node:child_process';
import { test } from 'node:test';

const source = readFileSync(new URL('./check-release-readiness.sh', import.meta.url), 'utf8');
const start = source.indexOf('require_registry_package() {');
const end = source.indexOf('\n}\n', start) + 3;
assert.ok(start >= 0 && end > start);
const guard = source.slice(start, end);
const checksum = 'a'.repeat(64);
const entry = (version = '0.2.0', hash = checksum, origin = 'registry+https://github.com/rust-lang/crates.io-index') =>
  `[[package]]\nname = "nostr-social-graph"\nversion = "${version}"\nsource = "${origin}"\nchecksum = "${hash}"\n`;

for (const [name, lock, accepted] of [
  ['accepts the exact single registry package', entry(), true],
  ['rejects a different checksum', entry('0.2.0', 'b'.repeat(64)), false],
  ['rejects a second graph family even when one entry matches', entry() + entry('0.1.4'), false],
  ['rejects a non-registry source', entry('0.2.0', checksum, 'git+https://example.invalid/graph'), false],
]) {
  test(name, () => {
    const root = mkdtempSync(path.join(tmpdir(), 'drive-registry-guard-'));
    try {
      writeFileSync(path.join(root, 'Cargo.lock'), lock);
      const result = spawnSync('bash', ['-c',
        `set -euo pipefail\nROOT="$1"\n${guard}\nrequire_registry_package Cargo.lock nostr-social-graph 0.2.0 "$2"`,
        'registry-guard', root, checksum], { encoding: 'utf8' });
      assert.equal(result.status === 0, accepted, result.stderr);
    } finally {
      rmSync(root, { recursive: true });
    }
  });
}
