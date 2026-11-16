import test from 'node:test'
import assert from 'node:assert/strict'
import { accessSync, constants, existsSync, readFileSync } from 'node:fs'
import { join } from 'node:path'
import { fileURLToPath } from 'node:url'
import { androidBuildToolsVersion } from './release-android-signing.mjs'
import { hostedWorkflowName, stagedPlatforms, verifiedOnlyPlatforms } from './release-hosted-stage.mjs'
import { iosPhases } from './release-ios-hosted.mjs'

// The hosted release route spans a workflow, several scripts and the README.
// These checks keep the pieces naming the same things.
const root = fileURLToPath(new URL('..', import.meta.url))
const read = path => readFileSync(join(root, path), 'utf8')
const workflow = read('.github/workflows/release-products.yml')
const readme = read('README.md')
const release = readme.slice(readme.indexOf('### Release'), readme.indexOf('### Workspace Layout'))

test('the hosted products workflow still produces what staging downloads and verifies', () => {
  assert.match(workflow, new RegExp(`^name: ${hostedWorkflowName}$`, 'm'))
  assert.match(workflow, /name: drive-\$\{\{ matrix\.platform \}\}-\$\{\{ env\.SOURCE_COMMIT \}\}/)
  for (const platform of [...stagedPlatforms, ...verifiedOnlyPlatforms]) {
    assert.ok(workflow.includes(`"${platform}"`), `${platform} is built by the workflow matrix`)
  }
  assert.ok(workflow.includes(`build-tools;${androidBuildToolsVersion}`), 'signing uses the hosted build-tools version')
  const builder = read('scripts/build-hosted-products.mjs')
  assert.ok(builder.includes('`${platform}-products.tar.gz`') && builder.includes('build-receipt.json'))
  assert.match(read('scripts/ios-build'), /IRIS_DRIVE_IOS_HOSTED_PRODUCT_DIR/)
  for (const command of Object.keys(iosPhases)) accessSync(join(root, 'scripts', command), constants.X_OK)
})

test('every script in the README release route exists', () => {
  const scripts = new Set([...release.matchAll(/(?:^|[\s`])(scripts\/[\w./-]*\w)/gm)].map(match => match[1]))
  assert.ok(scripts.size >= 8, [...scripts].join(', '))
  for (const script of scripts) assert.ok(existsSync(join(root, script)), `${script} exists`)
  for (const step of ['release-products.yml', 'product-lab.yml', 'release-hosted-stage.mjs', 'release-ios-hosted.mjs',
    'release-gate-env.mjs', 'release-tools.mjs install', 'IRIS_STACK_GATE_RECEIPT', '--final', 'zapstore-readback.mjs']) {
    assert.ok(release.includes(step), `README release route mentions ${step}`)
  }
})

test('the hosted web check uses the pinned Git helper independently of the journey storage server', () => {
  const workflow = read('.github/workflows/release-web-check.yml')
  const check = read('scripts/check-hosted-web.mjs')
  assert.match(workflow, /node tooling\/scripts\/release-tools\.mjs install/)
  assert.doesNotMatch(workflow, /--bin git-remote-htree/)
  assert.match(workflow, /--version '=0\.2\.142'.*--bin htree/)
  assert.match(check, /run\('git', \['clone', publicWeb, web\].*env: releaseToolsEnv\(/)
})

test('the release route keeps local machine details out of the repository', () => {
  const files = ['README.md', 'scripts/release-gate-env.mjs', 'scripts/release-hosted-stage.mjs',
    'scripts/release-ios-hosted.mjs', 'scripts/release-tools.mjs', 'scripts/macos-vm-release-smoke.sh',
    'scripts/release-android-signing.mjs', 'scripts/zapstore-readback.mjs']
  for (const file of files) {
    const text = read(file)
    assert.doesNotMatch(text, /\/Users\/|\/private\/tmp|\.codex_all|~\/src\/iris-drive\b/, `${file} has no local paths`)
    assert.doesNotMatch(text, /[0-9A-F]{8}-[0-9A-F]{4}-[0-9A-F]{4}-[0-9A-F]{4}-[0-9A-F]{12}/, `${file} has no device UDIDs`)
    assert.doesNotMatch(text, /Developer ID Application: [^E"]/, `${file} has no signing identity`)
  }
  assert.doesNotMatch(release, /macos-utm|ssh:\/\/|@[\w-]+\.(?:lan|home|internal)\b/)
})
