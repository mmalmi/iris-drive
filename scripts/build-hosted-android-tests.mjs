#!/usr/bin/env node
import { execFileSync } from 'node:child_process'
import { cpSync, mkdirSync, readFileSync, writeFileSync } from 'node:fs'
import { dirname, join, resolve } from 'node:path'
import { fileURLToPath } from 'node:url'
import { archiveDigest } from './hosted-product-receipt.mjs'
import { stageHostedProduct } from './stage-hosted-product.mjs'

const scripts = dirname(fileURLToPath(import.meta.url))
const tooling = dirname(scripts)
const source = resolve(process.env.SOURCE_ROOT)
const output = resolve(process.env.PRODUCT_OUTPUT)
const inputs = resolve(process.env.PRODUCT_INPUTS)
const env = { ...process.env }
const git = (root, ...args) => execFileSync('git', args, { cwd: root, encoding: 'utf8' }).trim()
const commit = git(source, 'rev-parse', 'HEAD')
if (commit !== env.SOURCE_COMMIT || git(source, 'status', '--porcelain')) throw new Error('Require exact clean native source')
if (git(tooling, 'status', '--porcelain')) throw new Error('Require clean test tooling')
const testCommit = git(tooling, 'rev-parse', 'HEAD')
mkdirSync(output, { recursive: true })
function run(command, args, options = {}) {
  return execFileSync(command, args, { cwd: source, env, stdio: 'inherit', ...options })
}
const prepared = stageHostedProduct({ repoRoot: source, productDirectory: inputs,
  platform: 'android', run, env, destinationParent: output })
const original = join(prepared, 'apk/uiTest/app-uiTest.apk')
const testPath = 'android/app/src/androidTest/java/to/iris/drive/app/MainActivityApprovalDeepLinkTest.kt'
cpSync(join(tooling, testPath), join(source, testPath))
if (git(source, 'diff', '--name-only') !== testPath) throw new Error('Only the declared instrumentation test may differ')
run('gradle', ['-p', join(source, 'android'), '--no-daemon', '--max-workers=2',
  '-Dorg.gradle.jvmargs=-Xmx2g', '-Pkotlin.compiler.execution.strategy=in-process',
  '-PirisDriveVersionName=0.1.38', '-PirisDriveVersionCode=1039',
  ':app:assembleUiTestAndroidTest', '-x', ':app:buildRustArm64'])
const payload = join(output, 'payload')
mkdirSync(payload)
const app = join(payload, 'app-uiTest.apk')
const tests = join(payload, 'app-uiTest-androidTest.apk')
cpSync(original, app)
cpSync(join(source, 'android/app/build/outputs/apk/androidTest/uiTest/app-uiTest-androidTest.apk'), tests)
const signer = join(env.ANDROID_HOME, 'build-tools/36.0.0/apksigner')
// Instrumentation-only Gradle builds need not create the conventional debug key.
// Use one explicit disposable key for both installed test packages.
const key = join(output, 'instrumentation-only.jks')
run('keytool', ['-genkeypair', '-keystore', key, '-storepass', 'android',
  '-keypass', 'android', '-alias', 'androiddebugkey', '-keyalg', 'RSA',
  '-keysize', '2048', '-validity', '30', '-dname', 'CN=Android Test Fixture', '-noprompt'])
for (const apk of [app, tests]) {
  run(signer, ['sign', '--ks', key, '--ks-pass', 'pass:android', '--key-pass', 'pass:android', apk])
}
const certificate = path => {
  const result = run(signer, ['verify', '--print-certs', path], { encoding: 'utf8', stdio: ['ignore', 'pipe', 'inherit'] })
  const value = result.match(/Signer #1 certificate SHA-256 digest:\s*([a-fA-F0-9]{64})/)?.[1]?.toLowerCase()
  if (!value) throw new Error('Missing verified test certificate')
  return value
}
const appCertificate = certificate(app)
if (appCertificate !== certificate(tests)) throw new Error('Instrumentation and application test certificates differ')
const payloadSha256 = run('python3', [join(scripts, 'verify_android_test_payload.py'), original, app],
  { encoding: 'utf8', stdio: ['ignore', 'pipe', 'inherit'] }).trim()
if (!/^[a-f0-9]{64}$/.test(payloadSha256)) throw new Error('Missing unchanged application payload proof')
const files = { app: await archiveDigest(app), tests: await archiveDigest(tests) }
const archive = join(output, 'android-tests-products.tar.gz')
run('tar', ['-czf', archive, '-C', payload, '.'])
writeFileSync(join(output, 'test-build-receipt.json'), JSON.stringify({ schema: 1,
  nativeCommit: commit, nativeTree: git(source, 'rev-parse', 'HEAD^{tree}'), testCommit,
  testPath, testOnly: true, testsExecuted: false, nativeCompilation: false,
  productionPayloadUnchanged: true, payloadSha256, appCertificate,
  originalApp: await archiveDigest(original), files,
  product: JSON.parse(readFileSync(join(inputs, 'build-receipt.json'), 'utf8')),
  archive: 'android-tests-products.tar.gz', ...await archiveDigest(archive) }, null, 2))
