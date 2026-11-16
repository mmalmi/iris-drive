import test from 'node:test'
import assert from 'node:assert/strict'
import { createHash } from 'node:crypto'
import { mkdirSync, readFileSync, writeFileSync } from 'node:fs'
import { join } from 'node:path'
import { fixtureCheckout } from './lib/release-fixture.mjs'
import { iosReleaseLayout, iosReleasePhase } from './release-ios-hosted.mjs'

function setup() {
  const checkout = fixtureCheckout()
  const product = join(checkout.root, 'artifacts', 'hosted', 'ios-release')
  mkdirSync(product, { recursive: true })
  writeFileSync(join(product, 'build-receipt.json'), '{}')
  const calls = []
  const run = (program, args, { env }) => {
    calls.push({ program: program.split('/').slice(-2).join('/'), args, env })
    if (args[0] === 'ios-export') {
      mkdirSync(env.IRIS_DRIVE_IOS_EXPORT_DIR, { recursive: true })
      writeFileSync(join(env.IRIS_DRIVE_IOS_EXPORT_DIR, 'Iris Drive.ipa'), 'signed ipa')
    }
  }
  const phase = (command, action, extra = {}) => iosReleasePhase({ repoRoot: checkout.root, command, action,
    productDirectory: product, run, env: {}, ...extra })
  return { checkout, product, calls, phase }
}

test('export runs ios-build with the hosted product and records the exported IPA', () => {
  const { checkout, product, calls, phase } = setup()
  const layout = phase('ios-build', 'ios-export')
  assert.equal(layout.stage, join(checkout.root, 'dist', `ios-9.9.9-9009010-${checkout.commit.slice(0, 8)}`))
  assert.equal(calls[0].program, 'scripts/ios-build')
  assert.deepEqual(calls[0].args, ['ios-export'])
  const { env } = calls[0]
  assert.equal(env.IRIS_DRIVE_IOS_HOSTED_PRODUCT_DIR, product)
  assert.equal(env.IRIS_DRIVE_IOS_INTERNAL_ONLY, 'false')
  assert.equal(env.IRIS_DRIVE_IOS_MARKETING_VERSION, '9.9.9')
  assert.equal(env.IRIS_DRIVE_IOS_BUILD_NUMBER, '9009010')
  assert.equal(env.IRIS_DRIVE_IOS_EXPORT_DIR, join(layout.stage, 'eligible'))
  const sha = createHash('sha256').update('signed ipa').digest('hex')
  assert.equal(readFileSync(join(layout.stage, 'eligible.ipa.sha256'), 'utf8'), `${sha}  Iris Drive.ipa\n`)
})

test('upload and TestFlight use only the IPA recorded after export', () => {
  const { checkout, calls, phase } = setup()
  assert.throws(() => phase('ios-build', 'ios-upload'), /run ios-build ios-export first/)
  phase('ios-build', 'ios-export')
  phase('ios-build', 'ios-upload')
  phase('testflight-internal', 'put', { productDirectory: undefined })
  assert.deepEqual(calls.map(call => `${call.program} ${call.args[0]}`),
    ['scripts/ios-build ios-export', 'scripts/ios-build ios-upload', 'scripts/testflight-internal put'])
  writeFileSync(join(iosReleaseLayout(checkout.root).stage, 'eligible', 'Iris Drive.ipa'), 'changed')
  assert.throws(() => phase('ios-build', 'ios-upload'), /differs from the one recorded after export/)
})

test('only App Store Connect settings from the local env files reach the iOS tools', () => {
  const { calls, phase } = setup()
  phase('testflight-internal', 'status', { fileEnv: { IRIS_DRIVE_ASC_KEY_ID: 'KEY', IRIS_DRIVE_ITMS_TRANSPORTER: '/t',
    ANDROID_KEYSTORE_PASSWORD: 'secret', NOSTR_KEY_PATH: '/key' } })
  const { env } = calls[0]
  assert.equal(env.IRIS_DRIVE_ASC_KEY_ID, 'KEY')
  assert.equal(env.IRIS_DRIVE_ITMS_TRANSPORTER, '/t')
  assert.equal(env.ANDROID_KEYSTORE_PASSWORD, undefined)
  assert.equal(env.NOSTR_KEY_PATH, undefined)
})

test('release notes come from the release evidence folder and are required for submission', () => {
  const { checkout, calls, phase } = setup()
  phase('ios-build', 'ios-export')
  assert.throws(() => phase('app-store', 'submit'), /needs release notes at .*store-notes\.json/)
  const notes = iosReleaseLayout(checkout.root).notesDir
  mkdirSync(notes, { recursive: true })
  writeFileSync(join(notes, 'beta-notes.txt'), 'Faster sync.\n')
  writeFileSync(join(notes, 'store-notes.json'), '{"en-US":"Faster sync."}')
  phase('app-store', 'submit')
  const { env } = calls.at(-1)
  assert.equal(env.IRIS_DRIVE_TESTFLIGHT_WHATS_NEW, 'Faster sync.')
  assert.equal(env.IRIS_DRIVE_APP_STORE_NOTES_PATH, join(notes, 'store-notes.json'))
})

test('unknown phases, another commit, a dirty checkout or a missing product are refused', () => {
  const { checkout, phase } = setup()
  assert.throws(() => phase('ios-build', 'ios-archive'), /Unknown iOS release phase/)
  assert.throws(() => phase('testflight-public', 'put', { expectedCommit: 'c'.repeat(40) }), /expected commit/)
  assert.throws(() => phase('ios-build', 'ios-export', { productDirectory: undefined }), /--product/)
  writeFileSync(join(checkout.root, 'scratch.txt'), 'x')
  assert.throws(() => phase('ios-build', 'ios-export'), /clean release checkout/)
})
