#!/usr/bin/env node

import { spawnSync } from 'node:child_process'
import {
  chmodSync,
  copyFileSync,
  existsSync,
  lstatSync,
  mkdirSync,
  readFileSync,
  readdirSync,
  rmSync,
  writeFileSync,
} from 'node:fs'
import os from 'node:os'
import { basename, dirname, join, resolve } from 'node:path'
import process from 'node:process'
import { fileURLToPath } from 'node:url'

import {
  buildNumberFromVersion,
  bumpCargoPackageVersion,
  bumpPbxprojReleaseVersions,
  bumpXcodegenProjectVersions,
  buildReleaseManifest,
  buildReleaseManifestFiles,
  buildZapstorePublishPlan,
  canonicalReleaseAssetNames,
  normalizeTag,
  parseNotarytoolSubmitOutput,
  parseEnvFile,
  plannedReleaseAssetNames,
  readWorkspaceVersionTag,
  renderReleaseNotes,
  semverFromTag,
  splitCsv,
  validateCanonicalReleaseAssetSet,
  validateReleaseAssetSet,
} from './local-release-lib.mjs'
import {
  macosRestrictedProfileEntitlementKeys,
  prepareMacosEntitlementsData,
} from './macos-entitlements.mjs'
import { prepareMacosReleaseBinaries } from './macos-release-binaries.mjs'
import { createMacosDmg } from './macos-release-dmg.mjs'
import { assertZipEntriesEqualFiles } from './release-zip.mjs'
import { requireMeshRelease } from './mesh-release-gate.mjs'
import {
  assertNoPrivateBuildMetadata,
  normalizedTarOwnerArgs,
  packageUnixCliTarball,
  releaseRustBuildEnvironment,
} from './release-build-hygiene.mjs'

const __dirname = dirname(fileURLToPath(import.meta.url))
const repoRoot = resolve(__dirname, '..')
const rootCargoToml = join(repoRoot, 'Cargo.toml')
const releasePolicyPath = join(repoRoot, 'release-policy.json')
const distDir = join(repoRoot, 'dist')
const defaultEnvFiles = [
  join(repoRoot, 'dist', 'macos', 'provisioning.env'),
  join(repoRoot, '.env.release.local'),
  join(repoRoot, '.env.zapstore.local'),
]
const defaultBuildSteps = ['platform-versions', 'macos', 'linux', 'windows', 'android', 'ios']
class SkipStepError extends Error {}

function usage() {
  console.log(`Usage: node scripts/local-release.mjs [options]

Build or stage dist artifacts as a hashtree updater release, and optionally
publish the staged release tree.

Options:
  --build                Build selected platform artifacts into dist
  --publish              Publish the staged tree with htree release publish
  --final                Publish as final/latest instead of draft
  --draft                Publish as draft (default when --publish is set)
  --tag <tag>            Release tag (default: workspace version)
  --release-tree <name>  htree release tree name (default: releases/iris-drive)
  --asset-dir <path>     Artifact directory (default: dist)
  --stage-dir <path>     Staging directory
  --env-file <path>      Extra dotenv file to load
  --only <csv>           With --build, limit steps to platform-versions,macos,linux,windows,android,ios
  --skip <csv>           With --build, skip named steps
  --allow-partial        With --build, continue after unavailable platform builders
  --skip-zapstore        With --final, skip publishing the Android APK to Zapstore
  --dry-run              Print actions without copying or publishing
  --help                 Show this help

Final publication requires IRIS_STACK_GATE_RECEIPT for the exact annotated
release commit, produced by the pinned Iris Stack release gate.`)
}

function parseArgs(argv) {
  const options = {
    build: false,
    publish: false,
    draft: true,
    tag: null,
    releaseTree: null,
    assetDir: null,
    stageDir: null,
    envFiles: [],
    only: null,
    skip: new Set(),
    allowPartial: false,
    skipZapstore: false,
    dryRun: false,
  }
  for (let index = 0; index < argv.length; index += 1) {
    const arg = argv[index]
    switch (arg) {
      case '--help':
      case '-h':
        usage()
        process.exit(0)
      case '--build':
        options.build = true
        break
      case '--publish':
        options.publish = true
        break
      case '--final':
        options.publish = true
        options.draft = false
        break
      case '--draft':
        options.publish = true
        options.draft = true
        break
      case '--tag':
        options.tag = normalizeTag(argv[++index] ?? '')
        break
      case '--release-tree':
        options.releaseTree = argv[++index] ?? ''
        break
      case '--asset-dir':
        options.assetDir = resolve(repoRoot, argv[++index] ?? '')
        break
      case '--stage-dir':
        options.stageDir = resolve(repoRoot, argv[++index] ?? '')
        break
      case '--env-file':
        options.envFiles.push(resolve(repoRoot, argv[++index] ?? ''))
        break
      case '--only':
        options.only = new Set(splitCsv(argv[++index] ?? ''))
        break
      case '--skip':
        for (const step of splitCsv(argv[++index] ?? '')) {
          options.skip.add(step)
        }
        break
      case '--allow-partial':
        options.allowPartial = true
        break
      case '--skip-zapstore':
        options.skipZapstore = true
        break
      case '--dry-run':
        options.dryRun = true
        break
      default:
        throw new Error(`Unknown argument: ${arg}`)
    }
  }
  return options
}

function readOptionalEnvFiles(envFiles) {
  const loaded = {}
  for (const envFile of envFiles) {
    if (existsSync(envFile)) {
      Object.assign(loaded, parseEnvFile(readFileSync(envFile, 'utf8')))
    }
  }
  return loaded
}

function envFlag(env, name) {
  return ['1', 'true', 'yes', 'on'].includes(String(env[name] ?? '').trim().toLowerCase())
}

function quote(arg) {
  const value = String(arg)
  return /[^\w./:-]/.test(value) ? JSON.stringify(value) : value
}

function run(command, args, {
  capture = false,
  cwd = repoRoot,
  displayArgs = args,
  dryRun = false,
  env = process.env,
  input = undefined,
} = {}) {
  const rendered = [command, ...displayArgs].map(quote).join(' ')
  console.log(`$ ${rendered}`)
  if (dryRun) {
    return ''
  }
  const result = spawnSync(command, args, {
    cwd,
    env,
    encoding: 'utf8',
    input,
    stdio:
      input !== undefined
        ? ['pipe', capture ? 'pipe' : 'inherit', capture ? 'pipe' : 'inherit']
        : capture
          ? 'pipe'
          : 'inherit',
  })
  if (result.status !== 0) {
    const stderr = capture ? result.stderr.trim() : ''
    throw new Error(stderr || `${command} exited with status ${result.status ?? 'unknown'}`)
  }
  return capture ? result.stdout.trim() : ''
}

function runCodesign(args, { dryRun = false, env = process.env } = {}) {
  if (dryRun) {
    run('codesign', args, { dryRun, env })
    return
  }
  const attempts = Number.parseInt(String(env.IRIS_DRIVE_MACOS_CODESIGN_ATTEMPTS ?? '5'), 10)
  const maxAttempts = Number.isFinite(attempts) && attempts > 0 ? attempts : 5
  const delaySeconds = Number.parseInt(
    String(env.IRIS_DRIVE_MACOS_CODESIGN_RETRY_DELAY_SECONDS ?? '30'),
    10,
  )
  const retryDelaySeconds = Number.isFinite(delaySeconds) && delaySeconds >= 0 ? delaySeconds : 30
  for (let attempt = 1; attempt <= maxAttempts; attempt += 1) {
    try {
      run('codesign', args, { env })
      return
    } catch (error) {
      if (attempt === maxAttempts) {
        throw error
      }
      console.error(
        `codesign failed; retrying in ${retryDelaySeconds}s (attempt ${attempt}/${maxAttempts})...`,
      )
      spawnSync('sleep', [String(retryDelaySeconds)], { stdio: 'inherit' })
    }
  }
}

function commandExists(command) {
  const result =
    process.platform === 'win32'
      ? spawnSync('where', [command], { stdio: 'ignore' })
      : spawnSync('sh', ['-lc', `command -v "${command}"`], { stdio: 'ignore' })
  return result.status === 0
}

function cargoTargetDir(env = process.env) {
  const configured = String(env.CARGO_TARGET_DIR ?? '').trim()
  return configured ? resolve(repoRoot, configured) : join(repoRoot, 'target')
}

function findFirstFile(root, matcher) {
  if (!existsSync(root)) {
    return null
  }
  const entry = readdirSync(root).sort().find((candidate) => matcher(candidate))
  return entry ? join(root, entry) : null
}

function releaseVersionInfo(tag) {
  const version = semverFromTag(tag)
  return {
    version,
    build: buildNumberFromVersion(version),
  }
}

function selectedBuildSteps(options) {
  const steps = options.only ? [...options.only] : defaultBuildSteps
  return steps.filter((step) => !options.skip.has(step))
}

function stageLinuxDebCliBinary({ env, targetTriple, dryRun }) {
  const sourcePath = join(cargoTargetDir(env), targetTriple, 'release', 'idrive')
  const releaseDir = join(repoRoot, 'linux', 'target', 'release')
  const destPath = join(releaseDir, 'idrive')
  if (dryRun) {
    console.log(`$ mkdir -p ${quote(releaseDir)}`)
    console.log(`$ cp ${quote(sourcePath)} ${quote(destPath)}`)
    return
  }
  mkdirSync(releaseDir, { recursive: true })
  copyFileSync(sourcePath, destPath)
  chmodSync(destPath, 0o755)
}

function macosDeveloperIdIdentity(env, dryRun) {
  const requested = String(env.IRIS_DRIVE_MACOS_SIGN_IDENTITY ?? '').trim()
  if (requested) {
    return requested
  }
  if (dryRun) {
    return 'Developer ID Application: Example (TEAMID)'
  }
  const identities = run('security', ['find-identity', '-v', '-p', 'codesigning'], {
    capture: true,
  })
  const match = identities.match(/"([^"]*Developer ID Application[^"]*)"/)
  return match?.[1] ?? ''
}

function macosTeamIdentifier(identity, env, dryRun) {
  const direct = String(env.IRIS_DRIVE_MACOS_TEAM_ID ?? '').trim()
  if (direct) {
    return direct
  }
  const parenthesized = identity.match(/\(([A-Z0-9]+)\)\s*$/)
  if (parenthesized) {
    return parenthesized[1]
  }
  if (dryRun) {
    return 'TEAMID'
  }
  const certPem = run('security', ['find-certificate', '-c', identity, '-p'], { capture: true })
  const subject = run('openssl', ['x509', '-noout', '-subject', '-nameopt', 'RFC2253'], {
    capture: true,
    input: certPem,
  })
  const match = subject.match(/(?:^|,)OU=([^,]+)/)
  return match?.[1] ?? ''
}

function macosCodesignTimestampArgs(env) {
  const url = String(env.IRIS_DRIVE_MACOS_TIMESTAMP_URL ?? '').trim()
  return [url ? `--timestamp=${url}` : '--timestamp']
}

function macosHardenedRuntimeArgs() {
  return ['--options', 'runtime']
}

function resolveMacosNotaryAuth(env) {
  const profile = String(
    env.IRIS_DRIVE_MACOS_NOTARY_KEYCHAIN_PROFILE ?? env.IRIS_DRIVE_MACOS_NOTARY_PROFILE ?? '',
  ).trim()
  if (profile) {
    return { mode: 'keychain-profile', profile }
  }
  const ascAuth = resolveIosAscAuth(env)
  const keyPath = String(
    env.IRIS_DRIVE_MACOS_NOTARY_KEY_PATH ??
      env.IRIS_DRIVE_ASC_AUTH_KEY_PATH ??
      env.IRIS_DRIVE_ASC_KEY_PATH ??
      ascAuth.keyPath ??
      '',
  ).trim()
  const keyId = String(
    env.IRIS_DRIVE_MACOS_NOTARY_KEY_ID ??
      env.IRIS_DRIVE_ASC_AUTH_KEY_ID ??
      env.IRIS_DRIVE_ASC_KEY_ID ??
      ascAuth.keyId ??
      '',
  ).trim()
  const issuerId = String(
    env.IRIS_DRIVE_MACOS_NOTARY_ISSUER_ID ??
      env.IRIS_DRIVE_ASC_AUTH_KEY_ISSUER_ID ??
      env.IRIS_DRIVE_ASC_ISSUER_ID ??
      ascAuth.issuerId ??
      '',
  ).trim()
  return {
    mode: 'api-key',
    keyPath: keyPath ? resolve(repoRoot, keyPath) : '',
    keyId,
    issuerId,
  }
}

function macosNotaryAuthArgs(env, dryRun) {
  const auth = resolveMacosNotaryAuth(env)
  if (auth.mode === 'keychain-profile') {
    return {
      args: ['--keychain-profile', auth.profile],
      displayArgs: ['--keychain-profile', auth.profile],
    }
  }
  if (auth.keyPath && auth.keyId && auth.issuerId) {
    return {
      args: ['--key', auth.keyPath, '--key-id', auth.keyId, '--issuer', auth.issuerId],
      displayArgs: ['--key', '<path>', '--key-id', '<key-id>', '--issuer', '<issuer-id>'],
    }
  }
  if (dryRun) {
    return {
      args: ['--keychain-profile', 'iris-drive-notary'],
      displayArgs: ['--keychain-profile', 'iris-drive-notary'],
    }
  }
  throw new Error(
    'Missing macOS notarization inputs. Set IRIS_DRIVE_MACOS_NOTARY_KEYCHAIN_PROFILE, or set IRIS_DRIVE_MACOS_NOTARY_KEY_PATH, IRIS_DRIVE_MACOS_NOTARY_KEY_ID, and IRIS_DRIVE_MACOS_NOTARY_ISSUER_ID.',
  )
}

function validateMacosNotaryInputs(env) {
  const auth = resolveMacosNotaryAuth(env)
  if (auth.mode === 'keychain-profile') {
    return []
  }
  const missing = []
  if (!auth.keyPath) {
    missing.push('macOS notarization API key path')
  } else if (!existsSync(auth.keyPath)) {
    missing.push(`macOS notarization API key file not found: ${auth.keyPath}`)
  }
  if (!auth.keyId) {
    missing.push('macOS notarization API key ID')
  }
  if (!auth.issuerId) {
    missing.push('macOS notarization issuer ID')
  }
  return missing
}

function submitMacosNotarization({ artifactPath, env, dryRun }) {
  const auth = macosNotaryAuthArgs(env, dryRun)
  const output = run('xcrun', ['notarytool', 'submit', artifactPath, '--wait', ...auth.args], {
    capture: !dryRun,
    dryRun,
    env,
    displayArgs: ['notarytool', 'submit', artifactPath, '--wait', ...auth.displayArgs],
  })
  if (output) {
    console.log(output)
  }
  if (!dryRun) {
    const result = parseNotarytoolSubmitOutput(output)
    if (result.status !== 'accepted') {
      const suffix = result.id ? ` (submission ${result.id})` : ''
      const status = result.status || 'unknown'
      throw new Error(`Apple notarization failed for ${basename(artifactPath)}: ${status}${suffix}`)
    }
  }
}

function stapleMacosArtifact({ artifactPath, dryRun }) {
  run('xcrun', ['stapler', 'staple', artifactPath], { dryRun })
  run('xcrun', ['stapler', 'validate', artifactPath], { dryRun })
}

function notarizeMacosApp({ appPath, signingDir, env, dryRun }) {
  const appZipPath = join(signingDir, 'IrisDriveMac.notary.zip')
  if (!dryRun) {
    rmSync(appZipPath, { force: true })
  }
  run('ditto', ['-c', '-k', '--keepParent', appPath, appZipPath], { dryRun, env })
  submitMacosNotarization({ artifactPath: appZipPath, env, dryRun })
  stapleMacosArtifact({ artifactPath: appPath, dryRun })
}

function macosProvisionedEntitlementsEnabled(env) {
  return envFlag(env, 'IRIS_DRIVE_MACOS_KEEP_PROVISIONED_ENTITLEMENTS')
}

function macosProvisioningProfilePath(env, name) {
  const value = String(env[name] ?? '').trim()
  return value ? resolve(repoRoot, value) : ''
}

function copyMacosProvisioningProfile({ profilePath, bundlePath, dryRun }) {
  const destination = join(bundlePath, 'Contents', 'embedded.provisionprofile')
  if (dryRun) {
    console.log(`Would embed provisioning profile ${profilePath || '<missing>'} -> ${destination}`)
    return
  }
  if (!profilePath) {
    throw new Error(
      `Missing provisioning profile for ${bundlePath}. Set IRIS_DRIVE_MACOS_APP_PROVISIONING_PROFILE and IRIS_DRIVE_MACOS_FILEPROVIDER_PROVISIONING_PROFILE, or unset IRIS_DRIVE_MACOS_KEEP_PROVISIONED_ENTITLEMENTS.`,
    )
  }
  if (!existsSync(profilePath)) {
    throw new Error(`macOS provisioning profile not found: ${profilePath}`)
  }
  copyFileSync(profilePath, destination)
}

function readPlistFile(path) {
  const script = String.raw`
import json
import plistlib
import sys

with open(sys.argv[1], "rb") as handle:
    print(json.dumps(plistlib.load(handle)))
`
  return JSON.parse(run('python3', ['-c', script, path], { capture: true }))
}

function writePlistFile(path, data) {
  const script = String.raw`
import json
import plistlib
import sys

with open(sys.argv[1], "wb") as handle:
    plistlib.dump(json.loads(sys.stdin.read()), handle, sort_keys=False)
`
  run('python3', ['-c', script, path], { input: JSON.stringify(data) })
}

function readMacosProvisioningProfileEntitlements(profilePath) {
  if (!profilePath) {
    throw new Error(
      'Missing macOS provisioning profile path while profile-filtering release entitlements.',
    )
  }
  if (!existsSync(profilePath)) {
    throw new Error(`macOS provisioning profile not found: ${profilePath}`)
  }
  const profilePlist = run('security', ['cms', '-D', '-i', profilePath], { capture: true })
  const script = String.raw`
import json
import plistlib
import sys

profile = plistlib.loads(sys.stdin.buffer.read())
print(json.dumps(profile.get("Entitlements", {})))
`
  return JSON.parse(run('python3', ['-c', script], { capture: true, input: profilePlist }))
}

function prepareMacosEntitlements(sourcePath, outputPath, teamId, {
  dryRun,
  env,
  profileEntitlements = {},
}) {
  const keepProvisionedEntitlements = macosProvisionedEntitlementsEnabled(env)
  const action = keepProvisionedEntitlements
    ? 'keep provisioned entitlements authorized by embedded profile'
    : `strip ${macosRestrictedProfileEntitlementKeys.join(', ')}`
  if (dryRun) {
    console.log(`Would prepare entitlements ${sourcePath} for team ${teamId} (${action})`)
    return
  }
  rmSync(outputPath, { force: true })
  const { entitlements, dropped } = prepareMacosEntitlementsData({
    sourceEntitlements: readPlistFile(sourcePath),
    teamId,
    keepProvisionedEntitlements,
    profileEntitlements,
  })
  writePlistFile(outputPath, entitlements)
  if (dropped.length > 0) {
    console.warn(
      `Dropped macOS entitlement(s) not launch-authorized by release profile: ${dropped.join(', ')}`,
    )
  }
  if (!existsSync(outputPath)) {
    throw new Error(`Prepared entitlements were not written: ${outputPath}`)
  }
  if (!keepProvisionedEntitlements || dropped.length > 0) {
    const prepared = readFileSync(outputPath, 'utf8')
    const leaked = dropped.filter((key) => prepared.includes(key))
    if (leaked.length > 0) {
      throw new Error(`Prepared entitlements still contain provisioned key(s): ${leaked.join(', ')}`)
    }
  }
}

function buildMacosArtifacts({ env, tag, dryRun }) {
  if (!dryRun && (process.platform !== 'darwin' || process.arch !== 'arm64')) {
    throw new SkipStepError('macOS release artifacts must be built on Apple Silicon macOS.')
  }
  const deploymentTarget = String(env.MACOSX_DEPLOYMENT_TARGET ?? '14.0').trim()
  if (!/^\d+(?:\.\d+){1,2}$/.test(deploymentTarget)) {
    throw new Error(`Invalid MACOSX_DEPLOYMENT_TARGET: ${deploymentTarget}`)
  }
  env = { ...env, MACOSX_DEPLOYMENT_TARGET: deploymentTarget }
  const rustLibDir = join(cargoTargetDir(env), 'aarch64-apple-darwin', 'release')
  const idrivePath = join(rustLibDir, 'idrive')
  console.log(`macOS deployment target: ${deploymentTarget}`)
  run('cargo', ['build', '--release', '-p', 'idrive', '--target', 'aarch64-apple-darwin'], {
    env,
    dryRun,
  })
  run(
    'cargo',
    ['build', '--release', '-p', 'iris-drive-app-core', '--target', 'aarch64-apple-darwin'],
    {
      env,
      dryRun,
    },
  )
  if (!dryRun) {
    assertNoPrivateBuildMetadata(
      [idrivePath, join(rustLibDir, 'libiris_drive_app_core.a')],
      env,
      { repoRoot },
    )
  }
  packageUnixCliTarball({
    binaryPath: idrivePath,
    distDir,
    env,
    run,
    targetTriple: 'aarch64-apple-darwin',
    tag,
    dryRun,
  })

  if (commandExists('xcodegen')) {
    run('xcodegen', ['generate'], { cwd: join(repoRoot, 'macos'), dryRun, env })
  }
  const derivedData = join(repoRoot, 'macos', '.build', 'ReleaseDerivedData')
  const releaseVersion = releaseVersionInfo(tag)
  run(
    'xcodebuild',
    [
      '-project',
      join(repoRoot, 'macos', 'IrisDriveMac.xcodeproj'),
      '-scheme',
      'IrisDriveMac',
      '-configuration',
      'Release',
      '-derivedDataPath',
      derivedData,
      'CODE_SIGNING_ALLOWED=NO',
      `LIBRARY_SEARCH_PATHS=${rustLibDir}`,
      `OTHER_LDFLAGS=${join(rustLibDir, 'libiris_drive_app_core.a')}`,
      `MARKETING_VERSION=${releaseVersion.version}`,
      `CURRENT_PROJECT_VERSION=${releaseVersion.build}`,
      'build',
    ],
    { dryRun, env },
  )
  const appPath = join(derivedData, 'Build', 'Products', 'Release', 'Iris Drive.app')
  const { appCliPath, appexCliPath, appexPath } = prepareMacosReleaseBinaries({
    appPath,
    cliPath: idrivePath,
    dryRun,
    env,
    repoRoot,
    run,
  })
  const identity = macosDeveloperIdIdentity(env, dryRun)
  if (!identity) {
    throw new Error('Missing Developer ID Application identity for macOS release signing.')
  }
  const teamId = macosTeamIdentifier(identity, env, dryRun)
  if (!teamId) {
    throw new Error(`Could not resolve Team ID for macOS signing identity: ${identity}`)
  }
  const keepProvisionedEntitlements = macosProvisionedEntitlementsEnabled(env)
  const appProvisioningProfile = macosProvisioningProfilePath(
    env,
    'IRIS_DRIVE_MACOS_APP_PROVISIONING_PROFILE',
  )
  const appexProvisioningProfile = macosProvisioningProfilePath(
    env,
    'IRIS_DRIVE_MACOS_FILEPROVIDER_PROVISIONING_PROFILE',
  )
  const signingDir = join(repoRoot, 'macos', '.build', 'ReleaseSigning')
  const appEntitlements = join(signingDir, 'IrisDriveMac.entitlements')
  const appexEntitlements = join(signingDir, 'IrisDriveFileProvider.entitlements')
  const appProfileEntitlements =
    keepProvisionedEntitlements && !dryRun
      ? readMacosProvisioningProfileEntitlements(appProvisioningProfile)
      : {}
  const appexProfileEntitlements =
    keepProvisionedEntitlements && !dryRun && existsSync(appexPath)
      ? readMacosProvisioningProfileEntitlements(appexProvisioningProfile)
      : {}
  if (!dryRun) {
    mkdirSync(signingDir, { recursive: true })
  }
  prepareMacosEntitlements(join(repoRoot, 'macos', 'Release.entitlements'), appEntitlements, teamId, {
    dryRun,
    env,
    profileEntitlements: appProfileEntitlements,
  })
  prepareMacosEntitlements(
    join(repoRoot, 'macos', 'FileProvider', 'Release.entitlements'),
    appexEntitlements,
    teamId,
    { dryRun, env, profileEntitlements: appexProfileEntitlements },
  )
  if (keepProvisionedEntitlements) {
    copyMacosProvisioningProfile({ profilePath: appProvisioningProfile, bundlePath: appPath, dryRun })
    copyMacosProvisioningProfile({
      profilePath: appexProvisioningProfile,
      bundlePath: appexPath,
      dryRun,
    })
  }
  const timestampArgs = macosCodesignTimestampArgs(env)
  const runtimeArgs = macosHardenedRuntimeArgs()
  runCodesign(
    [
      '--force',
      ...timestampArgs,
      ...runtimeArgs,
      '--sign',
      identity,
      appCliPath,
    ],
    { dryRun, env },
  )
  runCodesign(
    ['--force', ...timestampArgs, ...runtimeArgs, '--sign', identity, appexCliPath],
    { dryRun, env },
  )
  runCodesign(
    [
      '--force',
      ...timestampArgs,
      ...runtimeArgs,
      '--sign',
      identity,
      '--entitlements',
      appexEntitlements,
      appexPath,
    ],
    { dryRun, env },
  )
  runCodesign(
    [
      '--force',
      ...timestampArgs,
      ...runtimeArgs,
      '--sign',
      identity,
      '--entitlements',
      appEntitlements,
      appPath,
    ],
    { dryRun, env },
  )
  run('codesign', ['--verify', '--deep', '--strict', appPath], { dryRun })
  notarizeMacosApp({ appPath, signingDir, env, dryRun })

  const dmgPath = join(distDir, `iris-drive-${tag}-macos-arm64.dmg`)
  if (!dryRun) {
    mkdirSync(distDir, { recursive: true })
    rmSync(dmgPath, { force: true })
  }
  createMacosDmg({ appPath, dmgPath, dryRun, env, repoRoot, run })
  runCodesign(['--force', ...timestampArgs, '--sign', identity, dmgPath], { dryRun, env })
  run('codesign', ['--verify', '--strict', dmgPath], { dryRun })
  submitMacosNotarization({ artifactPath: dmgPath, env, dryRun })
  stapleMacosArtifact({ artifactPath: dmgPath, dryRun })
  const updaterArchivePath = join(distDir, `iris-drive-${tag}-macos-arm64.app.tar.gz`)
  if (!dryRun) {
    rmSync(updaterArchivePath, { force: true })
  }
  run(
    'tar',
    [
      ...normalizedTarOwnerArgs(),
      '-czf',
      updaterArchivePath,
      '-C',
      dirname(appPath),
      basename(appPath),
    ],
    { dryRun },
  )
  run(
    join(repoRoot, 'scripts', 'macos-release-smoke.sh'),
    ['--app', appPath, '--archive', updaterArchivePath, '--dmg', dmgPath],
    { dryRun, env },
  )
}

function buildLinuxArtifacts({ env, tag, dryRun }) {
  if (!dryRun && process.platform !== 'linux') {
    throw new SkipStepError('Linux release artifacts must be built on Linux.')
  }
  if (!dryRun && !commandExists('cargo-deb')) {
    throw new Error('Missing cargo-deb; install it before building Linux release artifacts.')
  }
  const targetTriple = 'x86_64-unknown-linux-gnu'
  const cliPath = join(cargoTargetDir(env), targetTriple, 'release', 'idrive')
  run('cargo', ['build', '--release', '--target', targetTriple, '-p', 'idrive'], {
    dryRun,
    env,
  })
  if (!dryRun) {
    assertNoPrivateBuildMetadata([cliPath], env, { repoRoot })
  }
  packageUnixCliTarball({
    binaryPath: cliPath,
    distDir,
    env,
    run,
    targetTriple,
    tag,
    dryRun,
  })
  stageLinuxDebCliBinary({ env, targetTriple, dryRun })
  run('cargo', ['build', '--release', '--manifest-path', join(repoRoot, 'linux', 'Cargo.toml')], {
    dryRun,
    env,
  })
  if (!dryRun) {
    assertNoPrivateBuildMetadata(
      [join(repoRoot, 'linux', 'target', 'release', 'iris-drive')],
      env,
      { repoRoot },
    )
  }
  const debPath = join(repoRoot, 'linux', 'target', 'debian', `iris-drive-${tag}-linux-x64.deb`)
  if (!dryRun) {
    mkdirSync(dirname(debPath), { recursive: true })
  }
  run('cargo', ['deb', '--no-build', '--output', debPath], { cwd: join(repoRoot, 'linux'), dryRun, env })
  if (!dryRun) {
    if (!existsSync(debPath)) {
      throw new Error('Expected Linux .deb output was not produced.')
    }
    run(
      'node',
      [join(repoRoot, 'scripts', 'verify-linux-deb-license.mjs'), debPath, join(repoRoot, 'LICENSE')],
      { env },
    )
    mkdirSync(distDir, { recursive: true })
    copyFileSync(debPath, join(distDir, basename(debPath)))
  }
}

function androidSigningIsComplete(env) {
  return Boolean(
    env.ANDROID_KEYSTORE_PATH &&
      env.ANDROID_KEYSTORE_PASSWORD &&
      env.ANDROID_KEY_ALIAS &&
      env.ANDROID_KEY_PASSWORD,
  )
}

function androidKeystorePath(env) {
  const value = String(env.ANDROID_KEYSTORE_PATH ?? '').trim()
  return value ? resolve(repoRoot, value) : ''
}

function resolveIosAscAuth(env) {
  const ascRoot = String(env.IRIS_DRIVE_ASC_ROOT ?? join(os.homedir(), '.appstoreconnect')).trim()
  let keyPath = String(env.IRIS_DRIVE_ASC_AUTH_KEY_PATH ?? env.IRIS_DRIVE_ASC_KEY_PATH ?? '').trim()
  if (!keyPath) {
    const keyDir = join(ascRoot, 'private_keys')
    if (existsSync(keyDir)) {
      const keyName = readdirSync(keyDir)
        .filter((entry) => /^AuthKey_.*\.p8$/.test(entry))
        .sort()[0]
      if (keyName) {
        keyPath = join(keyDir, keyName)
      }
    }
  }
  let keyId = String(env.IRIS_DRIVE_ASC_AUTH_KEY_ID ?? env.IRIS_DRIVE_ASC_KEY_ID ?? '').trim()
  if (!keyId && keyPath) {
    keyId = basename(keyPath).replace(/^AuthKey_/, '').replace(/\.p8$/, '')
  }
  let issuerId = String(
    env.IRIS_DRIVE_ASC_AUTH_KEY_ISSUER_ID ?? env.IRIS_DRIVE_ASC_ISSUER_ID ?? '',
  ).trim()
  const issuerPath = join(ascRoot, 'issuer.txt')
  if (!issuerId && existsSync(issuerPath)) {
    issuerId = readFileSync(issuerPath, 'utf8').trim()
  }
  return {
    keyPath: keyPath ? resolve(repoRoot, keyPath) : '',
    keyId,
    issuerId,
  }
}

function validateFinalReleaseBuildInputs({ env, steps }) {
  const missing = []
  if (steps.includes('macos')) {
    missing.push(...validateMacosNotaryInputs(env))
  }
  if (steps.includes('android') && !androidSigningIsComplete(env)) {
    missing.push(
      'Android signing inputs: ANDROID_KEYSTORE_PATH, ANDROID_KEYSTORE_PASSWORD, ANDROID_KEY_ALIAS, ANDROID_KEY_PASSWORD',
    )
  }
  if (steps.includes('android') && androidSigningIsComplete(env)) {
    const keystorePath = androidKeystorePath(env)
    if (!existsSync(keystorePath)) {
      missing.push(`Android keystore file not found: ${keystorePath}`)
    }
  }
  if (steps.includes('ios')) {
    const ascAuth = resolveIosAscAuth(env)
    if (!ascAuth.keyPath) {
      missing.push('App Store Connect API key file is required for iOS TestFlight')
    } else if (!existsSync(ascAuth.keyPath)) {
      missing.push(`App Store Connect API key file not found: ${ascAuth.keyPath}`)
    }
    if (!ascAuth.keyId) {
      missing.push('App Store Connect API key ID is required for iOS TestFlight')
    }
    if (!ascAuth.issuerId) {
      missing.push('App Store Connect issuer ID is required for iOS TestFlight')
    }
  }
  if (missing.length > 0) {
    throw new Error(`Missing final release input(s): ${missing.join('; ')}`)
  }
}

function readProjectReleasePolicy() {
  const policy = JSON.parse(readFileSync(releasePolicyPath, 'utf8'))
  if (policy?.windows?.signing !== 'unsigned') {
    throw new Error('release-policy.json must set windows.signing to "unsigned"')
  }
  return policy
}

function validateFinalPublishInputs({ env, skipZapstore }) {
  if (skipZapstore) {
    return
  }
  const missing = []
  if (!resolveZapstoreSignWith(env)) {
    missing.push('Missing Zapstore signing key; set SIGN_WITH or NOSTR_KEY_PATH in .env.zapstore.local')
  }
  if (!existsSync(join(repoRoot, 'zapstore.yaml'))) {
    missing.push('Missing zapstore.yaml; cannot publish Zapstore release')
  }
  if (!commandExists('zsp')) {
    missing.push('Missing zsp; cannot publish Zapstore release')
  }
  if (missing.length > 0) {
    throw new Error(`Missing final publish input(s): ${missing.join('; ')}`)
  }
}

function buildAndroidArtifacts({ env, tag, dryRun }) {
  if (!dryRun && !commandExists('cargo-ndk')) {
    throw new Error('Missing cargo-ndk; install it before building Android release artifacts.')
  }
  const signed = androidSigningIsComplete(env)
  if (!dryRun && !signed && String(env.IRIS_DRIVE_ALLOW_UNSIGNED_ANDROID ?? '').trim() !== '1') {
    throw new Error(
      'Android release signing is not configured. Set ANDROID_KEYSTORE_PATH, ANDROID_KEYSTORE_PASSWORD, ANDROID_KEY_ALIAS, and ANDROID_KEY_PASSWORD.',
    )
  }
  const releaseVersion = releaseVersionInfo(tag)
  run(
    'bash',
    [
      join(repoRoot, 'tools', 'run-android'),
      `-PirisDriveVersionName=${releaseVersion.version}`,
      `-PirisDriveVersionCode=${releaseVersion.build}`,
      'clean',
      ':app:assembleRelease',
      ':app:bundleRelease',
    ],
    {
      env,
      dryRun,
    },
  )
  if (!dryRun) {
    assertNoPrivateBuildMetadata(
      [join(repoRoot, 'android/app/src/main/jniLibs/arm64-v8a/libiris_drive_app_core.so')],
      env,
      { repoRoot },
    )
  }
  const apkPath = findFirstFile(
    join(repoRoot, 'android', 'app', 'build', 'outputs', 'apk', 'release'),
    (entry) => entry.endsWith('.apk'),
  )
  const aabPath = findFirstFile(
    join(repoRoot, 'android', 'app', 'build', 'outputs', 'bundle', 'release'),
    (entry) => entry.endsWith('.aab'),
  )
  if (!dryRun) {
    if (!apkPath || !aabPath) {
      throw new Error('Expected Android APK/AAB outputs were not produced.')
    }
    assertZipEntriesEqualFiles(
      [
        [join(repoRoot, 'LICENSE'), apkPath, 'assets/LICENSE'],
        [join(repoRoot, 'LICENSE'), aabPath, 'base/assets/LICENSE'],
      ],
      'Android release license validation failed.',
    )
    const suffix = signed ? '' : '-unsigned'
    mkdirSync(distDir, { recursive: true })
    copyFileSync(apkPath, join(distDir, `iris-drive-${tag}-android-arm64${suffix}.apk`))
    copyFileSync(aabPath, join(distDir, `iris-drive-${tag}-android-arm64${suffix}.aab`))
  }
}

function buildWindowsArtifacts({ env, tag, dryRun }) {
  if (!dryRun && process.platform !== 'win32') {
    throw new SkipStepError('Windows release artifacts must be built on Windows.')
  }
  const args = [
    '-NoProfile',
    '-ExecutionPolicy',
    'Bypass',
    '-File',
    join(repoRoot, 'scripts', 'windows-publish.ps1'),
    '-Configuration',
    'Release',
    '-Installer',
    '-Tag',
    tag,
    '-OutputDir',
    distDir,
  ]
  run('powershell.exe', args, { dryRun, env })
  const cliPath = join(repoRoot, 'target', 'release', 'idrive.exe')
  const windowsLicensePath = join(repoRoot, 'LICENSE')
  const cliZipPath = join(distDir, `idrive-${tag}-x86_64-pc-windows-msvc.zip`)
  const expectedInstallerPath = join(distDir, `iris-drive-${tag}-windows-x64-setup.exe`)
  if (!dryRun) {
    if (!existsSync(windowsLicensePath)) {
      throw new Error('Missing Windows CLI license: LICENSE')
    }
    assertNoPrivateBuildMetadata(
      [cliPath, join(repoRoot, 'target', 'release', 'iris_drive_app_core.dll'), windowsLicensePath],
      env,
      { repoRoot },
    )
    assertZipEntriesEqualFiles(
      [[cliPath, cliZipPath, 'idrive.exe'], [windowsLicensePath, cliZipPath, 'LICENSE']],
      'Windows CLI archive validation failed.',
    )
    if (!existsSync(expectedInstallerPath)) {
      throw new Error(`Missing Windows installer: ${expectedInstallerPath}`)
    }
  }
}

function buildIosTestFlight({ env, tag, dryRun }) {
  if (!dryRun && process.platform !== 'darwin') {
    throw new SkipStepError('iOS TestFlight builds must be created on macOS.')
  }
  const channels = resolveIosTestFlightChannels(env)
  const releaseVersion = releaseVersionInfo(tag)
  const publicTestFlight = channels.includes('public')
  const command = publicTestFlight ? 'ios-testflight-public' : 'ios-testflight'
  console.log(`iOS TestFlight version: ${releaseVersion.version} (${releaseVersion.build})`)
  run('bash', [join(repoRoot, 'scripts', 'ios-build'), command], {
    dryRun,
    env: {
      ...env,
      IRIS_DRIVE_IOS_TESTFLIGHT_CHANNELS: channels.join(','),
      IRIS_DRIVE_IOS_MARKETING_VERSION: releaseVersion.version,
      IRIS_DRIVE_IOS_BUILD_NUMBER: releaseVersion.build,
      IRIS_DRIVE_RELEASE_TAG: tag,
    },
  })
}

function resolveIosTestFlightChannels(env) {
  const rawChannels = String(env.IRIS_DRIVE_IOS_TESTFLIGHT_CHANNELS ?? '').trim()
  if (rawChannels) {
    const channels = [...new Set(splitCsv(rawChannels))]
    const unknown = channels.filter((channel) => !['internal', 'public'].includes(channel))
    if (unknown.length > 0) {
      throw new Error(`Unknown iOS TestFlight channel(s): ${unknown.join(', ')}`)
    }
    if (channels.length === 0) {
      throw new Error('IRIS_DRIVE_IOS_TESTFLIGHT_CHANNELS did not name any channels')
    }
    return channels
  }
  const publicTestFlight = String(env.IRIS_DRIVE_IOS_PUBLIC_TESTFLIGHT ?? '').trim() === '1'
  return publicTestFlight ? ['public'] : ['internal']
}

function buildReleaseArtifacts({ env, tag, options }) {
  env = releaseRustBuildEnvironment(env, { repoRoot })
  const steps = selectedBuildSteps(options)
  const signedAndroid = androidSigningIsComplete(env)
  console.log(`Release build steps: ${steps.join(', ') || '(none)'}`)
  console.log(
    `Planned dist artifacts: ${plannedReleaseAssetNames(tag, steps, { signedAndroid }).join(', ') || '(none)'}`,
  )
  const builders = new Map([
    ['platform-versions', () => syncPlatformVersions({ tag, dryRun: options.dryRun })],
    ['macos', () => buildMacosArtifacts({ env, tag, dryRun: options.dryRun })],
    ['linux', () => buildLinuxArtifacts({ env, tag, dryRun: options.dryRun })],
    [
      'windows',
      () =>
        buildWindowsArtifacts({
          env,
          tag,
          dryRun: options.dryRun,
        }),
    ],
    ['android', () => buildAndroidArtifacts({ env, tag, dryRun: options.dryRun })],
    ['ios', () => buildIosTestFlight({ env, tag, dryRun: options.dryRun })],
  ])
  for (const step of steps) {
    const builder = builders.get(step)
    if (!builder) {
      throw new Error(`Unknown release build step: ${step}`)
    }
    try {
      builder()
    } catch (error) {
      if (error instanceof SkipStepError && options.allowPartial) {
        console.warn(`Skipping ${step}: ${error.message}`)
        continue
      }
      throw error
    }
  }
}

function syncPlatformVersions({ tag, dryRun }) {
  const targets = [
    { path: rootCargoToml, bump: bumpWorkspaceVersion },
    { path: join(repoRoot, 'linux', 'Cargo.toml'), bump: bumpCargoPackageVersion },
    { path: join(repoRoot, 'macos', 'project.yml'), bump: bumpXcodegenProjectVersions },
    { path: join(repoRoot, 'ios', 'project.yml'), bump: bumpXcodegenProjectVersions },
    { path: join(repoRoot, 'macos', 'IrisDriveMac.xcodeproj', 'project.pbxproj'), bump: bumpPbxprojReleaseVersions },
    { path: join(repoRoot, 'ios', 'IrisDriveIOS.xcodeproj', 'project.pbxproj'), bump: bumpPbxprojReleaseVersions },
  ]
  const updated = []
  for (const { path, bump } of targets) {
    if (!existsSync(path)) {
      continue
    }
    const original = readFileSync(path, 'utf8')
    const next = bump(original, tag)
    if (next === original) {
      continue
    }
    if (!dryRun) {
      writeFileSync(path, next)
    }
    updated.push(path.replace(`${repoRoot}/`, ''))
  }
  if (updated.length > 0) {
    console.log(`Synced platform versions to ${tag}: ${updated.join(', ')}`)
  } else {
    console.log(`Platform versions already at ${tag}`)
  }
}

function bumpWorkspaceVersion(cargoTomlText, version) {
  const semver = semverFromTag(version)
  return cargoTomlText
    .replace(
      /^(\[workspace\.package\][\s\S]*?^version\s*=\s*")[^"\n]+(")/m,
      `$1${semver}$2`,
    )
    .replace(
      /(iris-drive-core\s*=\s*\{\s*version\s*=\s*")[^"\n]+(")/g,
      `$1${semver}$2`,
    )
    .replace(
      /(iris-drive-app-core\s*=\s*\{\s*version\s*=\s*")[^"\n]+(")/g,
      `$1${semver}$2`,
    )
}

function collectReleaseAssetPaths(assetDir, tag) {
  if (!existsSync(assetDir)) {
    return []
  }
  const allowed = new Set(canonicalReleaseAssetNames(tag))
  return readdirSync(assetDir)
    .sort()
    .filter((entry) => allowed.has(entry))
    .map((entry) => join(assetDir, entry))
    .filter((path) => lstatSync(path).isFile())
}

function resolveReleaseCommit(tag, dryRun) {
  if (dryRun) {
    return tag
  }
  return run('git', ['rev-parse', 'HEAD'], { capture: true, dryRun }) || 'HEAD'
}

function stageRelease({
  tag,
  commit,
  assetDir,
  stageDir,
  draft,
  dryRun,
  plannedAssetNames = [],
  requireCompleteAppRelease = false,
}) {
  const assetDirNames = existsSync(assetDir) ? readdirSync(assetDir) : []
  const assetPaths = collectReleaseAssetPaths(assetDir, tag)
  const assetNames = assetPaths.map((assetPath) => basename(assetPath))
  const hasPlannedDryRunAssets = dryRun && plannedAssetNames.length > 0
  if (requireCompleteAppRelease && !hasPlannedDryRunAssets) {
    validateCanonicalReleaseAssetSet(tag, assetNames, assetDirNames)
  }
  const validationNames = hasPlannedDryRunAssets
    ? plannedAssetNames
    : assetNames.length > 0
      ? assetNames
      : plannedAssetNames
  validateReleaseAssetSet(validationNames, {
    requireCompleteAppRelease,
  })
  if (hasPlannedDryRunAssets) {
    console.log(`Would stage ${plannedAssetNames.length} planned asset(s) into ${stageDir}`)
    return
  }
  if (assetPaths.length === 0) {
    throw new Error(`No dist assets found for ${tag} in ${assetDir}`)
  }
  if (dryRun) {
    console.log(`Would stage ${assetPaths.length} asset(s) into ${stageDir}`)
    return
  }

  rmSync(stageDir, { recursive: true, force: true })
  mkdirSync(join(stageDir, 'assets'), { recursive: true })
  const stagedAssetPaths = []
  for (const assetPath of assetPaths) {
    const stagedPath = join(stageDir, 'assets', basename(assetPath))
    copyFileSync(assetPath, stagedPath)
    stagedAssetPaths.push(stagedPath)
  }
  const createdAt = Math.floor(Date.now() / 1000)
  const manifest = buildReleaseManifest({
    tag,
    commit,
    createdAt,
    assetPaths: stagedAssetPaths,
    draft,
  })
  for (const [fileName, text] of buildReleaseManifestFiles(manifest)) {
    writeFileSync(join(stageDir, fileName), text)
  }
  writeFileSync(
    join(stageDir, 'notes.md'),
    renderReleaseNotes({
      tag,
      commit,
      assetNames: stagedAssetPaths.map((assetPath) => basename(assetPath)),
    }),
  )
}

function publishRelease({ stageDir, releaseTree, tag, draft, dryRun, env = process.env }) {
  const addOutput = run('htree', ['add', stageDir], { capture: true, dryRun })
  const match = addOutput.match(/^\s*url:\s*(\S+)/m)
  if (!dryRun && !match) {
    throw new Error('Could not parse htree add output for release CID')
  }
  const cid = dryRun ? 'dry-run' : match[1]
  const args = ['release', 'publish', releaseTree, tag, cid]
  if (draft) {
    args.push('--draft')
  }
  const publishOutput = run('htree', args, { capture: !dryRun, dryRun })
  if (publishOutput) {
    console.log(publishOutput)
  }
  const npubMatch = publishOutput.match(/Published release:\s*htree:\/\/([^/\s]+)\//)
  return {
    cid,
    npub: dryRun
      ? String(env.IRIS_DRIVE_RELEASE_NPUB ?? 'npub1example').trim()
      : (npubMatch?.[1] ?? ''),
  }
}

function releaseResolverRefreshBaseUrls(env) {
  const configured = String(env.IRIS_DRIVE_RELEASE_RESOLVER_REFRESH_BASE_URLS ?? '').trim()
  if (/^(0|false|no|off|none)$/i.test(configured)) {
    return []
  }
  return splitCsv(configured || 'https://cdn.iris.to')
}

function publicReleaseManifestUrl(baseUrl, npub, releaseTree) {
  return `${baseUrl.replace(/\/+$/, '')}/${npub}/${encodeURIComponent(releaseTree)}/latest/release.json`
}

function refreshPublicReleaseResolvers({ env, npub, releaseTree, tag, dryRun }) {
  const baseUrls = releaseResolverRefreshBaseUrls(env)
  if (baseUrls.length === 0) {
    return
  }
  if (!npub) {
    throw new Error('Could not determine release npub for public resolver refresh.')
  }
  const encodedTree = encodeURIComponent(releaseTree)
  for (const rawBaseUrl of baseUrls) {
    const baseUrl = rawBaseUrl.replace(/\/+$/, '')
    const refreshUrl = `${baseUrl}/api/resolve/${npub}/${encodedTree}?refresh=1`
    const manifestUrl = publicReleaseManifestUrl(baseUrl, npub, releaseTree)
    const refreshBody = run(
      'curl',
      ['--fail', '--location', '--silent', '--show-error', '--max-time', '30', refreshUrl],
      { capture: !dryRun, dryRun },
    )
    if (dryRun) {
      console.log(`Would verify public release manifest ${manifestUrl}`)
      continue
    }
    const refreshed = JSON.parse(refreshBody)
    console.log(
      `Refreshed public release resolver ${baseUrl}: ${refreshed.hash ?? refreshed.cid ?? 'ok'}`,
    )
    const manifestBody = run(
      'curl',
      ['--fail', '--location', '--silent', '--show-error', '--max-time', '30', manifestUrl],
      { capture: true },
    )
    const manifest = JSON.parse(manifestBody)
    const manifestTag = normalizeTag(String(manifest.tag ?? manifest.version ?? ''))
    if (manifestTag !== normalizeTag(tag)) {
      throw new Error(
        `Public release manifest at ${manifestUrl} is stale: expected ${tag}, got ${manifestTag}`,
      )
    }
    console.log(`Verified public release manifest ${manifestUrl} -> ${manifestTag}`)
  }
}

function resolveZapstoreSignWith(env) {
  const direct = String(env.SIGN_WITH ?? '').trim()
  if (direct) {
    return direct
  }
  const keyPath = String(env.NOSTR_KEY_PATH ?? '').trim()
  if (!keyPath || !existsSync(keyPath)) {
    return ''
  }
  return readFileSync(keyPath, 'utf8').trim()
}

function publishZapstore({ env, tag, assetDir, dryRun, plannedAssetNames = [] }) {
  const signWith = resolveZapstoreSignWith(env)
  const zapstoreYaml = join(repoRoot, 'zapstore.yaml')
  const normalizedTag = normalizeTag(tag)
  const apkName = `iris-drive-${normalizedTag}-android-arm64.apk`
  const plan = buildZapstorePublishPlan({
    tag: normalizedTag,
    assetDir,
    distDir,
    apkExists: existsSync(join(assetDir, apkName)) || (dryRun && plannedAssetNames.includes(apkName)),
    zspAvailable: commandExists('zsp'),
    signWith,
    zapstoreYamlExists: existsSync(zapstoreYaml),
  })
  if (dryRun) {
    console.log(`Would publish ${plan.apkName} to Zapstore`)
    return
  }
  mkdirSync(distDir, { recursive: true })
  copyFileSync(plan.apkPath, plan.releaseSourcePath)
  run(
    'zsp',
    ['publish', '--quiet', '--skip-preview', '--overwrite-release', zapstoreYaml],
    {
      dryRun,
      env: { ...env, SIGN_WITH: plan.signWith },
    },
  )
}

function main() {
  const options = parseArgs(process.argv.slice(2))
  const env = { ...readOptionalEnvFiles([...defaultEnvFiles, ...options.envFiles]), ...process.env }
  const releasePolicy = readProjectReleasePolicy()
  const tag = options.tag || readWorkspaceVersionTag(readFileSync(rootCargoToml, 'utf8'))
  const releaseTree = options.releaseTree || env.IRIS_DRIVE_RELEASE_TREE || 'releases/iris-drive'
  const assetDir = options.assetDir || join(repoRoot, 'dist')
  const stageDir =
    options.stageDir || join(os.tmpdir(), `iris-drive-release-${tag.replace(/[^\w.-]/g, '_')}`)
  const commit = resolveReleaseCommit(tag, options.dryRun)
  const meshGate = { repoRoot, tag, commit, receiptPath: env.IRIS_STACK_GATE_RECEIPT }
  const buildSteps = selectedBuildSteps(options)
  const signedAndroid = androidSigningIsComplete(env)
  const plannedAssetNames = options.build
    ? plannedReleaseAssetNames(tag, buildSteps, { signedAndroid })
    : []

  console.log(`Release tag: ${tag}`)
  console.log(`Release tree: ${releaseTree}`)
  console.log(`Asset dir: ${assetDir}`)
  console.log(`Stage dir: ${stageDir}`)
  console.log(`Windows release signing: ${releasePolicy.windows.signing}`)

  if (options.build && options.publish && !options.draft) {
    validateFinalReleaseBuildInputs({ env, steps: buildSteps })
  }
  if (options.publish && !options.draft) {
    validateFinalPublishInputs({ env, skipZapstore: options.skipZapstore })
    if (!options.dryRun) requireMeshRelease(meshGate)
  }

  if (options.build) {
    buildReleaseArtifacts({ env, tag, options })
    if (!options.publish) {
      return
    }
  }

  stageRelease({
    tag,
    commit,
    assetDir,
    stageDir,
    draft: options.publish ? options.draft : true,
    dryRun: options.dryRun,
    plannedAssetNames,
    requireCompleteAppRelease: options.publish && !options.draft,
  })

  if (options.publish) {
    if (!commandExists('htree')) {
      throw new Error('Missing htree; cannot publish release')
    }
    if (!options.draft && !options.dryRun) requireMeshRelease(meshGate)
    const published = publishRelease({
      stageDir,
      releaseTree,
      tag,
      draft: options.draft,
      dryRun: options.dryRun,
      env,
    })
    console.log(
      `Published ${options.draft ? 'draft ' : ''}${tag} to ${releaseTree} via ${published.cid}`,
    )
    if (!options.draft) {
      refreshPublicReleaseResolvers({
        env,
        npub: published.npub,
        releaseTree,
        tag,
        dryRun: options.dryRun,
      })
    }
    if (!options.draft && !options.skipZapstore) {
      publishZapstore({ env, tag, assetDir, dryRun: options.dryRun, plannedAssetNames })
    }
  } else if (!options.dryRun) {
    console.log(`Staged ${tag} at ${stageDir}`)
  }
}

try {
  main()
} catch (error) {
  console.error(`error: ${error.message}`)
  process.exit(1)
}
