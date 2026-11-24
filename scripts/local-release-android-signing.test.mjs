import test from 'node:test'
import assert from 'node:assert/strict'
import { createHash } from 'node:crypto'
import { chmodSync, mkdirSync, mkdtempSync, readFileSync, writeFileSync } from 'node:fs'
import { tmpdir } from 'node:os'
import { join } from 'node:path'
import { license, writeZip } from './lib/release-fixture.mjs'
import { androidBuildToolsVersion, androidSigningTools, signHostedAndroid } from './release-android-signing.mjs'
import { runTool } from './release-env.mjs'

const repoRoot = new URL('..', import.meta.url).pathname
const files = ['iris-drive-v9.9.9-android-arm64.apk', 'iris-drive-v9.9.9-android-arm64.aab']
const fingerprint = createHash('sha256').update('CERT').digest('hex')
const colonUpper = hex => hex.toUpperCase().match(/../g).join(':')

// Stand-ins for the JDK and Android build-tools: they copy instead of signing
// and print what the real tools print, so the identity checks run unchanged.
function stubTools(root, { apkSigner = fingerprint, badging, aligned = true } = {}) {
  const jdk = join(root, 'jdk', 'bin')
  const buildTools = join(root, 'sdk', 'build-tools', androidBuildToolsVersion)
  mkdirSync(jdk, { recursive: true })
  mkdirSync(buildTools, { recursive: true })
  const write = (path, body) => { writeFileSync(path, `#!/bin/sh\n${body}\n`); chmodSync(path, 0o755) }
  write(join(jdk, 'keytool'), `case "$1" in -exportcert) printf CERT ;; -printcert) echo "SHA256: ${colonUpper(fingerprint)}" ;; esac`)
  write(join(jdk, 'jarsigner'), `if [ "$1" = -verify ]; then echo "jar verified."; exit 0; fi
while [ "$1" != -signedjar ]; do shift; done; cp "$3" "$2"`)
  write(join(buildTools, 'aapt'), `printf "%s\\n" "${badging ?? "package: name='to.iris.drive' versionCode='9009015' versionName='9.9.9' platformBuildVersionName='16'"}" "native-code: 'arm64-v8a'"`)
  write(join(buildTools, 'zipalign'), `if [ "$1" = -c ]; then exit ${aligned ? 0 : 1}; fi; cp "$4" "$5"`)
  write(join(buildTools, 'apksigner'), `if [ "$1" = verify ]; then echo "Signer #1 certificate SHA-256 digest: ${apkSigner}"; exit 0; fi
while [ "$1" != --out ]; do shift; done; cp "$3" "$2"`)
  return { JAVA_HOME: join(root, 'jdk'), ANDROID_HOME: join(root, 'sdk') }
}

function fixture(options = {}) {
  const root = mkdtempSync(join(tmpdir(), 'iris-drive-android-signing-'))
  const stage = join(root, 'stage')
  writeZip(join(stage, 'apk/release/app-release-unsigned.apk'),
    { 'assets/LICENSE': options.apkLicense ?? license, 'classes.dex': 'dex' })
  writeZip(join(stage, 'bundle/release/app-release.aab'), { 'base/assets/LICENSE': license, 'base/dex/classes.dex': 'dex' })
  const output = join(root, 'output')
  mkdirSync(output)
  const env = { ...process.env, ...stubTools(root, options), ANDROID_KEYSTORE_PATH: 'upload.jks',
    ANDROID_KEYSTORE_PASSWORD: 'test-only', ANDROID_KEY_ALIAS: 'upload', ANDROID_KEY_PASSWORD: 'test-only' }
  const calls = []
  const run = (command, args, opts) => { calls.push([command.split('/').pop(), ...args]); return runTool(command, args, opts) }
  return { root, stage, output, env, calls, sign: () => signHostedAndroid({ repoRoot, keyRoot: root, stage, output,
    files, version: '9.9.9', env, run }) }
}

test('hosted APK and AAB are aligned, signed and checked against the keystore certificate', () => {
  const { output, calls, sign, root } = fixture()
  assert.deepEqual(sign(), { fingerprint })
  for (const name of files) assert.ok(readFileSync(join(output, name)).length > 0)
  const signCall = calls.find(call => call[0] === 'apksigner' && call[1] === 'sign')
  assert.ok(signCall.includes(join(root, 'upload.jks')) && signCall.includes('env:ANDROID_KEYSTORE_PASSWORD'))
  assert.ok(!calls.flat().includes('test-only'), 'passwords stay in the environment, not in arguments')
  assert.ok(calls.some(call => call[0] === 'python3'), 'payload equality is checked')
})

test('an unaligned hosted APK is aligned to 16 KiB pages before signing', () => {
  const { calls, sign } = fixture({ aligned: false })
  assert.throws(sign, /zipalign exited 1/)
  assert.ok(calls.some(call => call[0] === 'zipalign' && call[1] === '-P'))
})

test('a different signer, package version or license is refused', () => {
  for (const [options, pattern] of [
    [{ apkSigner: 'f'.repeat(64) }, /Signed APK certificate differs/],
    [{ badging: "package: name='to.iris.drive' versionCode='1' versionName='9.9.9'" }, /package or version differs/],
    [{ badging: "package: name='to.iris.drive.debug' versionCode='9009015' versionName='9.9.9'" }, /package or version differs/],
    [{ apkLicense: 'other license' }, /Hosted Android license validation failed/],
  ]) {
    assert.throws(fixture(options).sign, pattern)
  }
})

test('missing signing settings or tools are named without guessing', () => {
  const { sign, env } = fixture()
  delete env.ANDROID_KEY_PASSWORD
  assert.throws(sign, /needs ANDROID_KEY_PASSWORD/)
  const tools = androidSigningTools({ JAVA_HOME: '/nonexistent', ANDROID_HOME: '/nonexistent' })
  assert.equal(tools.apksigner, `/nonexistent/build-tools/${androidBuildToolsVersion}/apksigner`)
  const missing = fixture()
  assert.throws(() => signHostedAndroid({ repoRoot, stage: missing.stage, output: missing.output, files, version: '9.9.9',
    env: missing.env, run: runTool, tools }), /Missing keytool; install JDK 17/)
})
