// Fixture checkouts and hosted products for release-route tests.
import { createHash } from 'node:crypto'
import { execFileSync, spawnSync } from 'node:child_process'
import { mkdirSync, mkdtempSync, readFileSync, writeFileSync } from 'node:fs'
import { tmpdir } from 'node:os'
import { dirname, join } from 'node:path'
import { tarGz } from './tar-fixture.mjs'

export const license = readFileSync(new URL('../../LICENSE', import.meta.url))

export function fixtureCheckout(version = '9.9.9') {
  const root = mkdtempSync(join(tmpdir(), 'iris-drive-release-checkout-'))
  const git = (...args) => execFileSync('git', ['-c', 'user.name=Release Test', '-c', 'user.email=release@test.invalid',
    '-c', 'commit.gpgsign=false', ...args], { cwd: root, encoding: 'utf8' }).trim()
  git('init', '-q', '-b', 'master')
  writeFileSync(join(root, 'Cargo.toml'), `[workspace.package]\nversion = "${version}"\n`)
  writeFileSync(join(root, 'LICENSE'), license)
  writeFileSync(join(root, '.gitignore'), 'artifacts/\ndist/\n')
  git('add', '.')
  git('commit', '-q', '-m', 'release fixture')
  return { root, git, commit: git('rev-parse', 'HEAD'), tree: git('rev-parse', 'HEAD^{tree}') }
}

export function writeZip(path, entries) {
  mkdirSync(dirname(path), { recursive: true })
  const result = spawnSync('python3', ['-I', '-c', `
import json, sys, zipfile
with zipfile.ZipFile(sys.argv[1], 'w') as archive:
    for name, data in json.loads(sys.argv[2]).items():
        archive.writestr(name, data.encode('latin-1'))
`, path, JSON.stringify(Object.fromEntries(Object.entries(entries).map(([name, data]) =>
    [name, Buffer.from(data).toString('latin1')])))], { encoding: 'utf8' })
  if (result.status !== 0) throw new Error(result.stderr)
  return readFileSync(path)
}

export function writeProduct(directory, { platform, commit, tree, version = '9.9.9', entries, receipt = {} }) {
  mkdirSync(directory, { recursive: true })
  const archiveName = `${platform}-products.tar.gz`
  const archive = tarGz(entries.map(entry => ({ mode: 0o644, ...entry })))
  writeFileSync(join(directory, archiveName), archive)
  writeFileSync(join(directory, 'build-receipt.json'), `${JSON.stringify({
    schema: 1, commit, tree, platform, version, sourceCleanBeforeBuild: true, signed: false,
    testsExecuted: false, archive: archiveName, bytes: archive.length,
    sha256: createHash('sha256').update(archive).digest('hex'), ...receipt,
  }, null, 2)}\n`)
  return directory
}
