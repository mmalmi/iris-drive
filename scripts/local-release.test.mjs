import test from 'node:test'
import assert from 'node:assert/strict'
import { spawn, spawnSync } from 'node:child_process'
import { generateKeyPairSync } from 'node:crypto'
import { chmodSync, mkdirSync, mkdtempSync, readFileSync, writeFileSync } from 'node:fs'
import { createServer } from 'node:http'
import { join } from 'node:path'
import { tmpdir } from 'node:os'
import { fileURLToPath } from 'node:url'

import {
  buildReleaseManifest,
  buildReleaseManifestFiles,
  buildZapstorePublishPlan,
  describeAsset,
  plannedReleaseAssetNames,
  readWorkspaceVersionTag,
  validateReleaseAssetSet,
} from './local-release-lib.mjs'

test('readWorkspaceVersionTag reads the workspace package version', () => {
  const tag = readWorkspaceVersionTag(`
[workspace]
members = []

[workspace.package]
version = "0.2.27"
`)

  assert.equal(tag, 'v0.2.27')
})

test('buildReleaseManifest marks idrive archives as binary archives', () => {
  const root = mkdtempSync(join(tmpdir(), 'iris-drive-release-test-'))
  const cli = join(root, 'idrive-v0.2.27-x86_64-unknown-linux-musl.tar.gz')
  writeFileSync(cli, 'archive')

  const manifest = buildReleaseManifest({
    tag: 'v0.2.27',
    commit: 'abc123',
    createdAt: 1774523304,
    assetPaths: [cli],
  })

  assert.equal(manifest.app, 'iris-drive')
  assert.equal(manifest.version, '0.2.27')
  assert.equal(manifest.assets[0].kind, 'binary-archive')
  assert.equal(manifest.assets[0].executable, 'idrive')
  assert.equal(manifest.assets[0].target, 'x86_64-unknown-linux-musl')
})

test('buildReleaseManifest records the Windows idrive executable name', () => {
  const root = mkdtempSync(join(tmpdir(), 'iris-drive-release-test-'))
  const cli = join(root, 'idrive-v0.2.27-x86_64-pc-windows-msvc.zip')
  writeFileSync(cli, 'archive')

  const manifest = buildReleaseManifest({
    tag: 'v0.2.27',
    commit: 'abc123',
    createdAt: 1774523304,
    assetPaths: [cli],
  })

  assert.equal(manifest.assets[0].kind, 'binary-archive')
  assert.equal(manifest.assets[0].executable, 'idrive.exe')
  assert.equal(manifest.assets[0].target, 'x86_64-pc-windows-msvc')
})

test('buildReleaseManifestFiles writes both updater manifest names', () => {
  const files = buildReleaseManifestFiles({
    app: 'iris-drive',
    version: '0.2.27',
    tag: 'v0.2.27',
    assets: [],
  })

  assert.deepEqual(files.map(([name]) => name), ['release.json', 'manifest.json'])
})

test('describeAsset labels idrive release assets', () => {
  assert.equal(
    describeAsset('idrive-v0.2.27-x86_64-pc-windows-msvc.zip'),
    'Windows x64 idrive CLI',
  )
  assert.equal(
    describeAsset('idrive-v0.2.27-x86_64-unknown-linux-gnu.tar.gz'),
    'Linux x64 idrive CLI',
  )
})

test('buildZapstorePublishPlan fails on missing required publish inputs', () => {
  const distRoot = join(tmpdir(), 'iris-drive-dist')
  const base = {
    tag: '0.2.27',
    assetDir: distRoot,
    distDir: distRoot,
    apkExists: true,
    zspAvailable: true,
    signWith: 'nsec1example',
    zapstoreYamlExists: true,
  }

  assert.throws(
    () => buildZapstorePublishPlan({ ...base, apkExists: false }),
    /Missing Android APK/,
  )
  assert.throws(
    () => buildZapstorePublishPlan({ ...base, zspAvailable: false }),
    /Missing zsp/,
  )
  assert.throws(
    () => buildZapstorePublishPlan({ ...base, signWith: '' }),
    /Missing Zapstore signing key/,
  )
})

test('buildZapstorePublishPlan resolves the Zapstore release source path', () => {
  const distRoot = join(tmpdir(), 'iris-drive-dist')
  const plan = buildZapstorePublishPlan({
    tag: '0.2.27',
    assetDir: distRoot,
    distDir: distRoot,
    apkExists: true,
    zspAvailable: true,
    signWith: 'nsec1example',
    zapstoreYamlExists: true,
  })

  assert.equal(plan.apkName, 'iris-drive-v0.2.27-android-arm64.apk')
  assert.equal(plan.apkPath, join(distRoot, 'iris-drive-v0.2.27-android-arm64.apk'))
  assert.equal(plan.releaseSourcePath, join(distRoot, 'zapstore-current-android-arm64.apk'))
})

test('validateReleaseAssetSet requires complete public app artifacts for final releases', () => {
  assert.throws(
    () => validateReleaseAssetSet(['idrive-v0.2.27-aarch64-apple-darwin.tar.gz'], {
      requireCompleteAppRelease: true,
    }),
    /macOS DMG.*Linux x64 desktop package.*Windows x64 installer.*signed Android APK/,
  )
})

test('validateReleaseAssetSet rejects unsigned Android public artifacts', () => {
  assert.throws(
    () => validateReleaseAssetSet(['iris-drive-v0.2.27-android-arm64-unsigned.apk']),
    /unsigned Android/,
  )
})

test('plannedReleaseAssetNames names the public release artifacts', () => {
  assert.deepEqual(plannedReleaseAssetNames('0.2.27', ['macos', 'linux', 'windows', 'android']), [
    'idrive-v0.2.27-aarch64-apple-darwin.tar.gz',
    'iris-drive-v0.2.27-macos-arm64.dmg',
    'idrive-v0.2.27-x86_64-unknown-linux-gnu.tar.gz',
    'iris-drive-v0.2.27-linux-x64.deb',
    'idrive-v0.2.27-x86_64-pc-windows-msvc.zip',
    'iris-drive-v0.2.27-windows-x64-setup.exe',
    'iris-drive-v0.2.27-android-arm64.apk',
    'iris-drive-v0.2.27-android-arm64.aab',
  ])
})

test('plannedReleaseAssetNames covers the complete final release validator', () => {
  const names = plannedReleaseAssetNames('v0.2.27', ['macos', 'linux', 'windows', 'android'])
  assert.doesNotThrow(() => validateReleaseAssetSet(names, { requireCompleteAppRelease: true }))
})

test('local-release dry-run validates planned build assets over partial existing dist assets', () => {
  const root = mkdtempSync(join(tmpdir(), 'iris-drive-release-cli-test-'))
  const assetDir = join(root, 'dist')
  const stageDir = join(root, 'stage')
  const keystorePath = join(root, 'upload-keystore.jks')
  mkdirSync(assetDir)
  writeFileSync(join(assetDir, 'iris-drive-v9.9.9-macos-arm64.dmg'), 'partial')
  writeFileSync(keystorePath, 'test keystore placeholder')

  const result = spawnSync(
    process.execPath,
    [
      fileURLToPath(new URL('./local-release.mjs', import.meta.url)),
      '--build',
      '--publish',
      '--final',
      '--dry-run',
      '--skip-zapstore',
      '--tag',
      'v9.9.9',
      '--only',
      'macos,linux,windows,android',
      '--asset-dir',
      assetDir,
      '--stage-dir',
      stageDir,
    ],
    {
      encoding: 'utf8',
      env: {
        ...process.env,
        ANDROID_KEYSTORE_PATH: keystorePath,
        ANDROID_KEYSTORE_PASSWORD: 'password',
        ANDROID_KEY_ALIAS: 'iris',
        ANDROID_KEY_PASSWORD: 'password',
      },
    },
  )

  assert.equal(result.status, 0, result.stderr)
  assert.match(result.stdout, /Would stage 8 planned asset\(s\)/)
})

test('local-release final dry-run rejects a missing Android keystore file', () => {
  const result = spawnSync(
    process.execPath,
    [
      fileURLToPath(new URL('./local-release.mjs', import.meta.url)),
      '--build',
      '--final',
      '--dry-run',
      '--skip-zapstore',
      '--tag',
      'v9.9.9',
      '--only',
      'android',
    ],
    {
      encoding: 'utf8',
      env: {
        ...process.env,
        ANDROID_KEYSTORE_PATH: '/tmp/iris-drive-missing-upload-keystore.jks',
        ANDROID_KEYSTORE_PASSWORD: 'password',
        ANDROID_KEY_ALIAS: 'iris',
        ANDROID_KEY_PASSWORD: 'password',
      },
    },
  )

  assert.equal(result.status, 1)
  assert.match(result.stderr, /Android keystore file not found/)
})

test('local-release final dry-run rejects missing App Store Connect auth for iOS', () => {
  const root = mkdtempSync(join(tmpdir(), 'iris-drive-asc-preflight-test-'))
  const result = spawnSync(
    process.execPath,
    [
      fileURLToPath(new URL('./local-release.mjs', import.meta.url)),
      '--build',
      '--final',
      '--dry-run',
      '--skip-zapstore',
      '--tag',
      'v9.9.9',
      '--only',
      'ios',
    ],
    {
      encoding: 'utf8',
      env: {
        ...process.env,
        IRIS_DRIVE_ASC_ROOT: root,
        IRIS_DRIVE_ASC_AUTH_KEY_PATH: '',
        IRIS_DRIVE_ASC_KEY_PATH: '',
        IRIS_DRIVE_ASC_AUTH_KEY_ID: '',
        IRIS_DRIVE_ASC_KEY_ID: '',
        IRIS_DRIVE_ASC_AUTH_KEY_ISSUER_ID: '',
        IRIS_DRIVE_ASC_ISSUER_ID: '',
      },
    },
  )

  assert.equal(result.status, 1)
  assert.match(result.stderr, /App Store Connect/)
  assert.doesNotMatch(result.stdout, /htree release publish/)
})

test('local-release final dry-run preflights Zapstore signing before publishing', () => {
  const root = mkdtempSync(join(tmpdir(), 'iris-drive-zapstore-preflight-test-'))
  const keystorePath = join(root, 'upload-keystore.jks')
  writeFileSync(keystorePath, 'test keystore placeholder')

  const result = spawnSync(
    process.execPath,
    [
      fileURLToPath(new URL('./local-release.mjs', import.meta.url)),
      '--build',
      '--final',
      '--dry-run',
      '--tag',
      'v9.9.9',
      '--only',
      'android',
    ],
    {
      encoding: 'utf8',
      env: {
        ...process.env,
        ANDROID_KEYSTORE_PATH: keystorePath,
        ANDROID_KEYSTORE_PASSWORD: 'password',
        ANDROID_KEY_ALIAS: 'iris',
        ANDROID_KEY_PASSWORD: 'password',
        NOSTR_KEY_PATH: '',
        SIGN_WITH: '',
      },
    },
  )

  assert.equal(result.status, 1)
  assert.match(result.stderr, /Missing Zapstore signing key/)
  assert.doesNotMatch(result.stdout, /htree release publish/)
})

test('local-release final dry-run can plan Zapstore publish from signed Android build output', () => {
  const root = mkdtempSync(join(tmpdir(), 'iris-drive-zapstore-plan-test-'))
  const binDir = join(root, 'bin')
  const keystorePath = join(root, 'upload-keystore.jks')
  mkdirSync(binDir)
  writeFileSync(keystorePath, 'test keystore placeholder')
  const zspPath = join(binDir, 'zsp')
  writeFileSync(zspPath, '#!/bin/sh\nexit 0\n')
  chmodSync(zspPath, 0o755)

  const result = spawnSync(
    process.execPath,
    [
      fileURLToPath(new URL('./local-release.mjs', import.meta.url)),
      '--build',
      '--final',
      '--dry-run',
      '--tag',
      'v9.9.9',
      '--only',
      'macos,linux,windows,android',
    ],
    {
      encoding: 'utf8',
      env: {
        ...process.env,
        ANDROID_KEYSTORE_PATH: keystorePath,
        ANDROID_KEYSTORE_PASSWORD: 'password',
        ANDROID_KEY_ALIAS: 'iris',
        ANDROID_KEY_PASSWORD: 'password',
        PATH: `${binDir}:${process.env.PATH}`,
        SIGN_WITH: 'nsec1test',
      },
    },
  )

  assert.equal(result.status, 0, result.stderr)
  assert.match(result.stdout, /Would publish iris-drive-v9\.9\.9-android-arm64\.apk to Zapstore/)
})

test('local-release dry-run builds the Windows installer in dist', () => {
  const result = spawnSync(
    process.execPath,
    [
      fileURLToPath(new URL('./local-release.mjs', import.meta.url)),
      '--build',
      '--dry-run',
      '--tag',
      'v9.9.9',
      '--only',
      'windows',
    ],
    { encoding: 'utf8' },
  )

  assert.equal(result.status, 0, result.stderr)
  assert.match(result.stdout, /scripts\/windows-publish\.ps1/)
  assert.match(result.stdout, /-Installer/)
  assert.match(result.stdout, /-Tag/)
  assert.match(result.stdout, /v9\.9\.9/)
})

test('local-release dry-run passes release versions to macOS and Android builders', () => {
  const result = spawnSync(
    process.execPath,
    [
      fileURLToPath(new URL('./local-release.mjs', import.meta.url)),
      '--build',
      '--dry-run',
      '--tag',
      'v9.9.9',
      '--only',
      'macos,android',
    ],
    { encoding: 'utf8' },
  )

  assert.equal(result.status, 0, result.stderr)
  assert.match(result.stdout, /MARKETING_VERSION=9\.9\.9/)
  assert.match(result.stdout, /CURRENT_PROJECT_VERSION=9009009/)
  assert.match(result.stdout, /--timestamp/)
  assert.match(result.stdout, /-PirisDriveVersionName=9\.9\.9/)
  assert.match(result.stdout, /-PirisDriveVersionCode=9009009/)
})

test('local-release build-only mode does not stage existing unsigned artifacts', () => {
  const root = mkdtempSync(join(tmpdir(), 'iris-drive-release-build-only-test-'))
  const assetDir = join(root, 'dist')
  mkdirSync(assetDir)
  writeFileSync(join(assetDir, 'iris-drive-v9.9.9-android-arm64-unsigned.apk'), 'unsigned')

  const result = spawnSync(
    process.execPath,
    [
      fileURLToPath(new URL('./local-release.mjs', import.meta.url)),
      '--build',
      '--tag',
      'v9.9.9',
      '--only',
      '',
      '--asset-dir',
      assetDir,
    ],
    { encoding: 'utf8' },
  )

  assert.equal(result.status, 0, result.stderr)
  assert.match(result.stdout, /Release build steps: \(none\)/)
})

test('local-release dry-run routes iOS builds through the TestFlight script', () => {
  const result = spawnSync(
    process.execPath,
    [
      fileURLToPath(new URL('./local-release.mjs', import.meta.url)),
      '--build',
      '--dry-run',
      '--only',
      'ios',
      '--tag',
      'v9.9.9',
    ],
    { encoding: 'utf8' },
  )

  assert.equal(result.status, 0, result.stderr)
  assert.match(result.stdout, /scripts\/ios-build ios-testflight/)
  assert.doesNotMatch(result.stdout, /Would archive\/export\/upload/)
})

test('local-release dry-run passes release versions to the iOS TestFlight builder', () => {
  const result = spawnSync(
    process.execPath,
    [
      fileURLToPath(new URL('./local-release.mjs', import.meta.url)),
      '--build',
      '--dry-run',
      '--only',
      'ios',
      '--tag',
      'v9.9.9',
    ],
    { encoding: 'utf8' },
  )

  assert.equal(result.status, 0, result.stderr)
  assert.match(result.stdout, /iOS TestFlight version: 9\.9\.9 \(9009009\)/)
})

test('local-release dry-run uses a public-capable iOS upload for internal plus public TestFlight', () => {
  const result = spawnSync(
    process.execPath,
    [
      fileURLToPath(new URL('./local-release.mjs', import.meta.url)),
      '--build',
      '--dry-run',
      '--only',
      'ios',
      '--tag',
      'v9.9.9',
    ],
    {
      encoding: 'utf8',
      env: {
        ...process.env,
        IRIS_DRIVE_IOS_TESTFLIGHT_CHANNELS: 'internal,public',
      },
    },
  )

  assert.equal(result.status, 0, result.stderr)
  assert.match(result.stdout, /scripts\/ios-build ios-testflight-public/)
})

test('TestFlight helper documents iris-drive App Store Connect inputs', () => {
  const result = spawnSync('bash', ['scripts/testflight-internal', '--help'], {
    cwd: fileURLToPath(new URL('..', import.meta.url)),
    encoding: 'utf8',
  })

  assert.equal(result.status, 0, result.stderr)
  assert.match(result.stdout, /ensure-app/)
  assert.match(result.stdout, /IRIS_DRIVE_ASC_AUTH_KEY_PATH/)
  assert.match(result.stdout, /IRIS_DRIVE_TESTFLIGHT_GROUPS/)
})

test('iOS build handles missing App Store Connect auth args under bash nounset', () => {
  const script = readFileSync(
    fileURLToPath(new URL('./ios-build', import.meta.url)),
    'utf8',
  )

  assert.match(script, /\$\{ASC_AUTH_ARGS\[@\]\+"\$\{ASC_AUTH_ARGS\[@\]\}"\}/)
})

test('iOS build uses the shared App Store Connect auth defaults', () => {
  const script = readFileSync(
    fileURLToPath(new URL('./ios-build', import.meta.url)),
    'utf8',
  )

  assert.match(script, /IRIS_DRIVE_ASC_ROOT/)
  assert.match(script, /private_keys/)
  assert.match(script, /AuthKey_\*\.p8/)
  assert.match(script, /issuer\.txt/)
})

test('TestFlight helper creates a missing App Store Connect app record', async (t) => {
  const root = mkdtempSync(join(tmpdir(), 'iris-drive-asc-test-'))
  const keyPath = join(root, 'AuthKey_TESTKEY123.p8')
  const { privateKey } = generateKeyPairSync('ec', { namedCurve: 'P-256' })
  writeFileSync(keyPath, privateKey.export({ type: 'pkcs8', format: 'pem' }))

  const requests = []
  const server = createServer(async (request, response) => {
    const url = new URL(request.url, `http://${request.headers.host}`)
    const chunks = []
    for await (const chunk of request) {
      chunks.push(chunk)
    }
    const rawBody = Buffer.concat(chunks).toString('utf8')
    const body = rawBody ? JSON.parse(rawBody) : null
    requests.push({ method: request.method, path: url.pathname, query: url.searchParams, body })

    if (request.method === 'GET' && url.pathname === '/v1/apps') {
      writeJson(response, 200, { data: [] })
      return
    }
    if (request.method === 'GET' && url.pathname === '/v1/bundleIds') {
      writeJson(response, 200, {
        data: [
          {
            type: 'bundleIds',
            id: 'BUNDLE123',
            attributes: { identifier: 'to.iris.drive.ios' },
          },
        ],
      })
      return
    }
    if (request.method === 'POST' && url.pathname === '/v1/apps') {
      writeJson(response, 201, {
        data: {
          type: 'apps',
          id: 'APP123',
          attributes: { name: body.data.attributes.name, bundleId: 'to.iris.drive.ios' },
        },
      })
      return
    }
    writeJson(response, 404, { errors: [{ title: 'unexpected request' }] })
  })
  t.after(() => server.close())
  await listen(server)

  const result = await spawnForTest('bash', ['scripts/testflight-internal', 'ensure-app'], {
    cwd: fileURLToPath(new URL('..', import.meta.url)),
    env: {
      ...process.env,
      IRIS_DRIVE_ASC_BASE_URL: `http://127.0.0.1:${server.address().port}/v1/`,
      IRIS_DRIVE_ASC_AUTH_KEY_PATH: keyPath,
      IRIS_DRIVE_ASC_AUTH_KEY_ID: 'TESTKEY123',
      IRIS_DRIVE_ASC_AUTH_KEY_ISSUER_ID: '00000000-0000-0000-0000-000000000000',
      IRIS_DRIVE_IOS_BUNDLE_ID: 'to.iris.drive.ios',
      IRIS_DRIVE_ASC_APP_NAME: 'Iris Drive',
    },
  })

  assert.equal(result.status, 0, result.stderr)
  assert.match(result.stdout, /Created App Store Connect app: Iris Drive \[APP123\]/)
  const createRequest = requests.find((request) => request.method === 'POST' && request.path === '/v1/apps')
  assert.ok(createRequest)
  assert.deepEqual(createRequest.body, {
    data: {
      type: 'apps',
      attributes: {
        name: 'Iris Drive',
        primaryLocale: 'en-US',
        sku: 'to.iris.drive.ios',
        platform: 'IOS',
      },
      relationships: {
        bundleId: { data: { type: 'bundleIds', id: 'BUNDLE123' } },
      },
    },
  })
})

test('TestFlight helper explains App Store Connect app creation permission failures', async (t) => {
  const root = mkdtempSync(join(tmpdir(), 'iris-drive-asc-test-'))
  const keyPath = join(root, 'AuthKey_TESTKEY123.p8')
  const { privateKey } = generateKeyPairSync('ec', { namedCurve: 'P-256' })
  writeFileSync(keyPath, privateKey.export({ type: 'pkcs8', format: 'pem' }))

  const server = createServer(async (request, response) => {
    const url = new URL(request.url, `http://${request.headers.host}`)
    const chunks = []
    for await (const chunk of request) {
      chunks.push(chunk)
    }

    if (request.method === 'GET' && url.pathname === '/v1/apps') {
      writeJson(response, 200, { data: [] })
      return
    }
    if (request.method === 'GET' && url.pathname === '/v1/bundleIds') {
      writeJson(response, 200, {
        data: [
          {
            type: 'bundleIds',
            id: 'BUNDLE123',
            attributes: { identifier: 'to.iris.drive.ios' },
          },
        ],
      })
      return
    }
    if (request.method === 'POST' && url.pathname === '/v1/apps') {
      writeJson(response, 403, {
        errors: [
          {
            status: '403',
            code: 'FORBIDDEN_ERROR',
            detail: "The resource 'apps' does not allow 'CREATE'. Allowed operations are: GET_COLLECTION, GET_INSTANCE, UPDATE",
          },
        ],
      })
      return
    }
    writeJson(response, 404, { errors: [{ title: 'unexpected request' }] })
  })
  t.after(() => server.close())
  await listen(server)

  const result = await spawnForTest('bash', ['scripts/testflight-internal', 'ensure-app'], {
    cwd: fileURLToPath(new URL('..', import.meta.url)),
    env: {
      ...process.env,
      IRIS_DRIVE_ASC_BASE_URL: `http://127.0.0.1:${server.address().port}/v1/`,
      IRIS_DRIVE_ASC_AUTH_KEY_PATH: keyPath,
      IRIS_DRIVE_ASC_AUTH_KEY_ID: 'TESTKEY123',
      IRIS_DRIVE_ASC_AUTH_KEY_ISSUER_ID: '00000000-0000-0000-0000-000000000000',
      IRIS_DRIVE_IOS_BUNDLE_ID: 'to.iris.drive.ios',
      IRIS_DRIVE_ASC_APP_NAME: 'Iris Drive',
    },
  })

  assert.equal(result.status, 1)
  assert.match(result.stderr, /cannot create App Store Connect app records/)
  assert.match(result.stderr, /Name: Iris Drive/)
  assert.match(result.stderr, /Bundle ID: to\.iris\.drive\.ios/)
  assert.match(result.stderr, /SKU: to\.iris\.drive\.ios/)
})

test('iOS FileProvider declares the required App Store document group', () => {
  const plist = readFileSync(
    fileURLToPath(new URL('../ios/FileProvider/Info.plist', import.meta.url)),
    'utf8',
  )

  assert.match(
    plist,
    /<key>NSExtensionFileProviderDocumentGroup<\/key>\s*<string>group\.to\.iris\.drive<\/string>/,
  )
})

test('iOS app icons have no alpha channel', () => {
  const appIconSet = fileURLToPath(
    new URL('../ios/Resources/Assets.xcassets/AppIcon.appiconset/', import.meta.url),
  )
  const contents = JSON.parse(readFileSync(join(appIconSet, 'Contents.json'), 'utf8'))
  const icons = contents.images.filter((image) => image.filename)

  assert.ok(
    icons.some((image) => image.size === '1024x1024'),
    'AppIcon.appiconset must declare a 1024x1024 marketing icon',
  )
  for (const icon of icons) {
    assert.equal(pngHasAlpha(join(appIconSet, icon.filename)), false, `${icon.filename} must be opaque`)
  }
})

function writeJson(response, status, body) {
  response.writeHead(status, { 'content-type': 'application/json' })
  response.end(JSON.stringify(body))
}

function listen(server) {
  return new Promise((resolve, reject) => {
    server.once('error', reject)
    server.listen(0, '127.0.0.1', resolve)
  })
}

function spawnForTest(command, args, options) {
  return new Promise((resolve) => {
    const child = spawn(command, args, { ...options, encoding: 'utf8' })
    let stdout = ''
    let stderr = ''
    child.stdout.on('data', (chunk) => {
      stdout += chunk
    })
    child.stderr.on('data', (chunk) => {
      stderr += chunk
    })
    child.on('close', (status) => resolve({ status, stdout, stderr }))
  })
}

function pngHasAlpha(path) {
  const bytes = readFileSync(path)
  assert.equal(bytes.subarray(0, 8).toString('hex'), '89504e470d0a1a0a', `${path} is not a PNG`)
  const colorType = bytes[25]
  if (colorType === 4 || colorType === 6) {
    return true
  }
  let offset = 8
  while (offset + 12 <= bytes.length) {
    const length = bytes.readUInt32BE(offset)
    const type = bytes.subarray(offset + 4, offset + 8).toString('ascii')
    if (type === 'tRNS') {
      return true
    }
    offset += 12 + length
  }
  return false
}
