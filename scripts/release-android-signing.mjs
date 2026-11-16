// Sign the hosted unsigned Android release with the local upload key and check
// every identity the stores rely on: package, version, ABI, license, 16 KiB
// alignment, signer certificate and an unchanged application payload.
import { createHash } from 'node:crypto'
import { spawnSync } from 'node:child_process'
import { existsSync, linkSync, readdirSync, rmSync } from 'node:fs'
import os from 'node:os'
import { join, resolve } from 'node:path'
import { fileURLToPath } from 'node:url'
import { buildNumberFromVersion } from './local-release-lib.mjs'
import { assertZipEntriesEqualFiles } from './release-zip.mjs'

// Matches the build-tools installed by .github/workflows/release-products.yml.
export const androidBuildToolsVersion = '36.0.0'
const signingKeys = ['ANDROID_KEYSTORE_PATH', 'ANDROID_KEYSTORE_PASSWORD', 'ANDROID_KEY_ALIAS', 'ANDROID_KEY_PASSWORD']

export function androidSigningTools(env, platform = process.platform) {
  let javaHome = env.JAVA_HOME ?? ''
  if (!javaHome && platform === 'darwin') {
    const found = spawnSync('/usr/libexec/java_home', ['-v', '17'], { encoding: 'utf8' })
    javaHome = found.status === 0 ? found.stdout.trim() : ''
  }
  const sdk = env.ANDROID_HOME || env.ANDROID_SDK_ROOT
    || join(os.homedir(), platform === 'darwin' ? 'Library/Android/sdk' : 'Android/Sdk')
  const buildTools = join(sdk, 'build-tools', androidBuildToolsVersion)
  return {
    keytool: join(javaHome, 'bin', 'keytool'),
    jarsigner: join(javaHome, 'bin', 'jarsigner'),
    aapt: join(buildTools, 'aapt'),
    zipalign: join(buildTools, 'zipalign'),
    apksigner: join(buildTools, 'apksigner'),
  }
}

function onlyFile(directory, suffix) {
  const entries = readdirSync(directory).filter(name => name.endsWith(suffix))
  if (entries.length !== 1) throw new Error(`Expected one ${suffix} in the hosted Android product`)
  return join(directory, entries[0])
}

export function signHostedAndroid({
  repoRoot, keyRoot = repoRoot, stage, output, files, version, env, run, tools = androidSigningTools(env),
}) {
  const missing = signingKeys.filter(name => !env[name])
  if (missing.length) throw new Error(`Android release signing needs ${missing.join(', ')}`)
  for (const [name, path] of Object.entries(tools)) {
    if (!existsSync(path)) {
      throw new Error(`Missing ${name}; install JDK 17 and Android build-tools ${androidBuildToolsVersion}`)
    }
  }
  const keystore = resolve(keyRoot, env.ANDROID_KEYSTORE_PATH)
  const alias = env.ANDROID_KEY_ALIAS
  const tool = (name, args, options = {}) => run(tools[name], args, { env, ...options })
  const rawApk = onlyFile(join(stage, 'apk', 'release'), '.apk')
  const rawAab = onlyFile(join(stage, 'bundle', 'release'), '.aab')
  const apk = join(output, files.find(name => name.endsWith('.apk')))
  const aab = join(output, files.find(name => name.endsWith('.aab')))
  const certificate = tool('keytool', ['-exportcert', '-keystore', keystore, '-alias', alias,
    '-storepass:env', 'ANDROID_KEYSTORE_PASSWORD'], { encoding: 'buffer' })
  const fingerprint = createHash('sha256').update(certificate).digest('hex')

  const license = join(repoRoot, 'LICENSE')
  assertZipEntriesEqualFiles([[license, rawApk, 'assets/LICENSE'], [license, rawAab, 'base/assets/LICENSE']],
    'Hosted Android license validation failed.')
  const badging = String(tool('aapt', ['dump', 'badging', rawApk])).split('\n')
  if (!/^package: name='to\.iris\.drive' /.test(badging[0] ?? '')
      || !badging[0].includes(`versionName='${version}'`)
      || !badging[0].includes(`versionCode='${buildNumberFromVersion(version)}'`)) {
    throw new Error('Hosted APK package or version differs from this release')
  }
  if (!badging.includes("native-code: 'arm64-v8a'")) throw new Error('Hosted APK is not the arm64-v8a release')

  const aligned = join(output, 'aligned.apk')
  try {
    tool('zipalign', ['-c', '-P', '16', '4', rawApk])
    linkSync(rawApk, aligned)
  } catch {
    tool('zipalign', ['-P', '16', '4', rawApk, aligned])
  }
  tool('apksigner', ['sign', '--ks', keystore, '--ks-key-alias', alias, '--ks-pass', 'env:ANDROID_KEYSTORE_PASSWORD',
    '--key-pass', 'env:ANDROID_KEY_PASSWORD', '--out', apk, aligned])
  rmSync(aligned, { force: true })
  tool('zipalign', ['-c', '-P', '16', '4', apk])
  const apkSigner = String(tool('apksigner', ['verify', '--verbose', '--print-certs', apk]))
    .match(/Signer #1 certificate SHA-256 digest: ([a-fA-F0-9]+)/)?.[1]?.toLowerCase()
  if (apkSigner !== fingerprint) throw new Error('Signed APK certificate differs from the release keystore')

  tool('jarsigner', ['-keystore', keystore, '-storepass:env', 'ANDROID_KEYSTORE_PASSWORD',
    '-keypass:env', 'ANDROID_KEY_PASSWORD', '-signedjar', aab, rawAab, alias])
  if (!/jar verified/i.test(String(tool('jarsigner', ['-verify', '-keystore', keystore,
    '-storepass:env', 'ANDROID_KEYSTORE_PASSWORD', aab, alias])))) {
    throw new Error('Signed AAB did not verify')
  }
  const aabSigner = String(tool('keytool', ['-printcert', '-jarfile', aab]))
    .match(/SHA256:\s*([\dA-F:]+)/i)?.[1]?.replaceAll(':', '').toLowerCase()
  if (aabSigner !== fingerprint) throw new Error('Signed AAB certificate differs from the release keystore')

  const payloadCheck = fileURLToPath(new URL('./verify_android_test_payload.py', import.meta.url))
  for (const [original, signed] of [[rawApk, apk], [rawAab, aab]]) {
    if (!/^[a-f0-9]{64}$/.test(String(run('python3', [payloadCheck, original, signed], { env })).trim())) {
      throw new Error('Signing changed the Android application payload')
    }
  }
  return { fingerprint }
}
