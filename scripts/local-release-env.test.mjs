import assert from 'node:assert/strict'
import { execFileSync } from 'node:child_process'
import { mkdirSync, mkdtempSync, rmSync, writeFileSync } from 'node:fs'
import { tmpdir } from 'node:os'
import { dirname, join } from 'node:path'
import test from 'node:test'
import { fileURLToPath } from 'node:url'
import { fixtureCheckout } from './lib/release-fixture.mjs'
import { loadReleaseEnv } from './release-env.mjs'

const release = fileURLToPath(new URL('./local-release.mjs', import.meta.url))
const key = 'IRIS_DRIVE_MACOS_NOTARY_KEYCHAIN_PROFILE'

function settings(root, name, value) {
  const file = join(root, name)
  mkdirSync(dirname(file), { recursive: true })
  writeFileSync(file, `${key}=${value}\n`)
  return file
}

function checkNotaryProfile(root, expected, args = [], env = {}) {
  const output = execFileSync(process.execPath, [
    release, '--build', '--dry-run', '--tag', 'v9.9.9', '--only', 'macos', ...args,
  ], { cwd: root, encoding: 'utf8', env: { PATH: process.env.PATH, HOME: root, IRIS_DRIVE_RELEASE_SOURCE_ROOT: root,
    IRIS_DRIVE_MACOS_RELEASE_SMOKE_COMMAND: 'fixture-smoke', ...env } })
  assert.match(output, new RegExp(`notarytool submit .* --keychain-profile ${expected}(?: |$)`, 'm'))
}

test('local macOS releases load primary settings before linked checkout and explicit overrides', t => {
  const { root, git } = fixtureCheckout()
  t.after(() => rmSync(root, { recursive: true, force: true }))
  const linked = join(root, 'linked')
  git('worktree', 'add', '-q', '--detach', linked, 'HEAD')
  writeFileSync(join(linked, 'release-policy.json'), '{"windows":{"signing":"unsigned"}}')
  settings(root, 'dist/macos/provisioning.env', 'primary-build-output')
  checkNotaryProfile(linked, 'iris-drive-notary')
  settings(root, '.env.release.local', 'project')
  checkNotaryProfile(linked, 'project', [], { PATH: '' })
  settings(root, '.env.zapstore.local', 'project-zapstore')
  checkNotaryProfile(linked, 'project-zapstore')

  settings(linked, 'dist/macos/provisioning.env', 'provisioned')
  checkNotaryProfile(linked, 'provisioned')
  assert.equal(loadReleaseEnv({ repoRoot: linked, env: {} })[key], 'project-zapstore')
  settings(linked, '.env.release.local', 'checkout')
  checkNotaryProfile(linked, 'checkout')
  settings(linked, '.env.zapstore.local', 'checkout-zapstore')
  checkNotaryProfile(linked, 'checkout-zapstore')
  settings(linked, 'first.env', 'first')
  settings(linked, 'second.env', 'second')
  const extra = ['--env-file', 'first.env', '--env-file', 'second.env']
  checkNotaryProfile(linked, 'second', extra)
  checkNotaryProfile(linked, 'shell', extra, { [key]: 'shell' })
})

test('local release still loads checkout settings in a non-Git dry run', t => {
  const root = mkdtempSync(join(tmpdir(), 'iris-drive-release-env-'))
  t.after(() => rmSync(root, { recursive: true, force: true }))
  writeFileSync(join(root, 'release-policy.json'), '{"windows":{"signing":"unsigned"}}')
  settings(root, 'dist/macos/provisioning.env', 'generated')
  settings(root, '.env.release.local', 'standalone')
  checkNotaryProfile(root, 'standalone')
})
