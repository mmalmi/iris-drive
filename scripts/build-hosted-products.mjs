#!/usr/bin/env node
import { spawnSync, execFileSync } from 'node:child_process'
import { cpSync, mkdirSync, mkdtempSync, readFileSync, readdirSync, rmSync, writeFileSync } from 'node:fs'
import { join, resolve } from 'node:path'
import { homedir } from 'node:os'
import { pathToFileURL } from 'node:url'
import { archiveDigest } from './hosted-product-receipt.mjs'

const root = resolve(process.env.SOURCE_ROOT)
const platform = process.env.PRODUCT_PLATFORM
const commit = process.env.SOURCE_COMMIT
const allowed = ['macos-release', 'macos-debug', 'ios-release', 'ios-sim-debug',
  'ios-sim-release', 'android', 'linux']
if (!allowed.includes(platform) || !/^[0-9a-f]{40}$/.test(commit)) throw new Error('Invalid build input')
const git = (...args) => execFileSync('git', args, { cwd: root, encoding: 'utf8' }).trim()
if (git('rev-parse', 'HEAD') !== commit || git('status', '--porcelain', '--untracked-files=all')) {
  throw new Error('Build requires the exact clean source checkout')
}
const tree = git('rev-parse', 'HEAD^{tree}')
const helper = await import(pathToFileURL(join(root, 'scripts/release-build-hygiene.mjs')))
const release = await import(pathToFileURL(join(root, 'scripts/local-release-lib.mjs')))
const version = release.readWorkspaceVersionTag(readFileSync(join(root, 'Cargo.toml'), 'utf8')).slice(1)
const output = resolve(process.env.PRODUCT_OUTPUT)
const payload = join(output, 'payload')
mkdirSync(payload, { recursive: true })
let env = helper.releaseRustBuildEnvironment({ ...process.env,
  CARGO_BUILD_JOBS: '2', CARGO_INCREMENTAL: '0', CARGO_PROFILE_DEV_DEBUG: '0',
  CARGO_PROFILE_TEST_DEBUG: '0', CARGO_PROFILE_RELEASE_DEBUG: '0',
  MACOSX_DEPLOYMENT_TARGET: '14.0', IPHONEOS_DEPLOYMENT_TARGET: '17.0',
}, { repoRoot: root })
function run(command, args, options = {}) {
  console.log(`Running ${command} ${args.join(' ')}`)
  const result = spawnSync(command, args, { cwd: root, stdio: 'inherit', env, ...options })
  if (result.status !== 0) throw new Error(`${command} failed: ${result.status ?? result.error}`)
}
function files(path) {
  return readdirSync(path, { withFileTypes: true }).flatMap(entry => {
    const item = join(path, entry.name)
    return entry.isDirectory() ? files(item) : entry.isFile() ? [item] : []
  })
}
if (platform.startsWith('macos') || platform.startsWith('ios')) {
  const mac = platform.startsWith('macos')
  const simulator = platform.startsWith('ios-sim')
  const debug = platform.endsWith('debug')
  const configuration = debug ? 'Debug' : 'Release'
  const target = mac ? 'aarch64-apple-darwin' : simulator ? 'aarch64-apple-ios-sim' : 'aarch64-apple-ios'
  env = { ...env, CARGO_TARGET_DIR: join(root, 'target') }
  run('rustup', ['target', 'add', target])
  const args = ['build', '--locked', '--target', target, '-p', 'iris-drive-app-core']
  if (mac) args.push('-p', 'idrive')
  if (!debug) args.push('--release')
  run('cargo', args)
  const rust = join(env.CARGO_TARGET_DIR, target, debug ? 'debug' : 'release')
  const library = join(rust, 'libiris_drive_app_core.a')
  helper.assertNoPrivateBuildMetadata([library], env, { repoRoot: root })
  const project = mac ? 'macos' : 'ios'
  const name = mac ? 'IrisDriveMac' : 'IrisDriveIOS'
  run('xcodegen', ['generate'], { cwd: join(root, project) })
  // dsymutil records its input binary path independently of compiler prefix maps.
  // Match the established iOS archive build's anonymous temporary directory.
  const derived = !mac && !simulator
    ? mkdtempSync('/tmp/iris-drive-ios-build-') : join(output, 'DerivedData')
  const swift = `$(inherited) -debug-prefix-map "${root}=/usr/src/iris-drive" -debug-prefix-map "${homedir()}=/usr/src/home"`
  const clang = `$(inherited) "-ffile-prefix-map=${root}=/usr/src/iris-drive" "-fdebug-prefix-map=${root}=/usr/src/iris-drive" "-ffile-prefix-map=${homedir()}=/usr/src/home" "-fdebug-prefix-map=${homedir()}=/usr/src/home"`
  const xcode = ['-project', join(root, project, `${name}.xcodeproj`), '-scheme', name,
    '-configuration', configuration, '-derivedDataPath', derived, '-jobs', '2',
    '-destination', mac ? 'generic/platform=macOS' : simulator ? 'generic/platform=iOS Simulator' : 'generic/platform=iOS',
    'ARCHS=arm64', 'ONLY_ACTIVE_ARCH=YES', 'CODE_SIGNING_ALLOWED=NO', 'CODE_SIGNING_REQUIRED=NO',
    'CODE_SIGN_IDENTITY=', 'PROVISIONING_PROFILE_SPECIFIER=',
    `LIBRARY_SEARCH_PATHS=${rust}`, `OTHER_LDFLAGS=${library}`,
    `MARKETING_VERSION=${version}`, `CURRENT_PROJECT_VERSION=${release.buildNumberFromVersion(version)}`,
    `OTHER_SWIFT_FLAGS=${swift}`, `OTHER_CFLAGS=${clang}`, `OTHER_CPLUSPLUSFLAGS=${clang}`,
    'SWIFT_SERIALIZE_DEBUGGING_OPTIONS=NO']
  if (!mac && !simulator) {
    const archive = join(payload, 'IrisDriveIOS.xcarchive')
    run('xcodebuild', [...xcode, '-archivePath', archive, 'archive'])
    helper.assertNoPrivateBuildMetadata(files(archive), env, { repoRoot: root })
    rmSync(derived, { recursive: true })
  } else {
    run('xcodebuild', [...xcode, simulator && debug ? 'build-for-testing' : 'build'])
    const products = join(derived, 'Build/Products')
    if (mac) {
      const appPath = join(products, configuration, 'Iris Drive.app')
      const cliPath = join(rust, 'idrive')
      if (!debug) {
        const { prepareMacosReleaseBinaries } = await import(pathToFileURL(join(root, 'scripts/macos-release-binaries.mjs')))
        prepareMacosReleaseBinaries({ appPath, cliPath, dryRun: false, env, repoRoot: root, run })
      } else {
        cpSync(cliPath, join(appPath, 'Contents/MacOS/idrive'))
        cpSync(cliPath, join(appPath, 'Contents/PlugIns/IrisDriveFileProvider.appex/Contents/MacOS/idrive'))
      }
      cpSync(appPath, join(payload, 'Iris Drive.app'), { recursive: true })
      cpSync(cliPath, join(payload, 'idrive'))
    } else {
      cpSync(products, join(payload, 'Products'), { recursive: true })
    }
  }
} else if (platform === 'linux') {
  delete env.CARGO_TARGET_DIR
  run('node', [join(root, 'scripts/local-release.mjs'), '--build', '--only', 'linux', '--tag', `v${version}`])
  for (const name of release.plannedReleaseAssetNames(`v${version}`, ['linux'])) {
    cpSync(join(root, 'dist', name), join(payload, name))
  }
} else {
  env = { ...env, CARGO_TARGET_DIR: join(root, 'target') }
  run('rustup', ['target', 'add', 'aarch64-linux-android'])
  run('gradle', ['-p', join(root, 'android'), '--no-daemon', '--max-workers=2',
    `-PirisDriveVersionName=${version}`, `-PirisDriveVersionCode=${release.buildNumberFromVersion(version)}`,
    ':app:assembleRelease', ':app:bundleRelease', ':app:assembleUiTest', ':app:assembleUiTestAndroidTest'])
  helper.assertNoPrivateBuildMetadata([join(root, 'android/app/src/main/jniLibs/arm64-v8a/libiris_drive_app_core.so')], env, { repoRoot: root })
  cpSync(join(root, 'android/app/build/outputs/apk'), join(payload, 'apk'), { recursive: true })
  cpSync(join(root, 'android/app/build/outputs/bundle'), join(payload, 'bundle'), { recursive: true })
}
run('git', ['diff', '--exit-code', '--', 'Cargo.lock', 'linux/Cargo.lock'])
const archive = join(output, `${platform}-products.tar.gz`)
run('tar', [...helper.normalizedTarOwnerArgs(), '-czf', archive, '-C', payload, '.'])
const receipt = { schema: 1, commit, tree, platform, version,
  sourceCleanBeforeBuild: true, signed: false, testsExecuted: false,
  archive: `${platform}-products.tar.gz`, ...await archiveDigest(archive) }
writeFileSync(join(output, 'build-receipt.json'), `${JSON.stringify(receipt, null, 2)}\n`)
