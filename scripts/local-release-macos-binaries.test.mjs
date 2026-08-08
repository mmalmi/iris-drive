import test from 'node:test'
import assert from 'node:assert/strict'
import {
  existsSync,
  mkdirSync,
  mkdtempSync,
  readFileSync,
  writeFileSync,
} from 'node:fs'
import { join } from 'node:path'
import { tmpdir } from 'node:os'

import { prepareMacosReleaseBinaries } from './macos-release-binaries.mjs'

function fixture({ extension = true, leakedResource = false } = {}) {
  const root = mkdtempSync(join(tmpdir(), 'iris-drive-macos-binaries-'))
  const appPath = join(root, 'Iris Drive.app')
  const appMacos = join(appPath, 'Contents', 'MacOS')
  const appexMacos = join(
    appPath,
    'Contents',
    'PlugIns',
    'IrisDriveFileProvider.appex',
    'Contents',
    'MacOS',
  )
  mkdirSync(appMacos, { recursive: true })
  writeFileSync(join(appMacos, 'Iris Drive'), 'app')
  if (extension) {
    mkdirSync(appexMacos, { recursive: true })
    writeFileSync(join(appexMacos, 'IrisDriveFileProvider'), 'extension')
  }
  if (leakedResource) {
    const resources = join(appPath, 'Contents', 'Resources')
    mkdirSync(resources, { recursive: true })
    writeFileSync(join(resources, 'helper.sh'), `#!/bin/sh\n# ${root}/source.swift\n`)
  }
  const cliPath = join(root, 'idrive')
  writeFileSync(cliPath, 'cli')
  return { appPath, cliPath, root }
}

function prepare(paths, { dryRun = false } = {}) {
  const calls = []
  const result = prepareMacosReleaseBinaries({
    ...paths,
    dryRun,
    env: {},
    repoRoot: paths.root,
    run: (command, args) => calls.push([command, ...args]),
  })
  return { calls, result }
}

test('macOS release strips, installs, and audits the complete app bundle', () => {
  const paths = fixture()
  const { calls, result } = prepare(paths)

  assert.deepEqual(calls, [
    ['strip', '-S', join(paths.appPath, 'Contents', 'MacOS', 'Iris Drive')],
    ['strip', '-S', join(result.appexPath, 'Contents', 'MacOS', 'IrisDriveFileProvider')],
  ])
  assert.equal(readFileSync(result.appCliPath, 'utf8'), 'cli')
  assert.equal(readFileSync(result.appexCliPath, 'utf8'), 'cli')
})

test('macOS release rejects a private path in any bundled file', () => {
  const paths = fixture({ leakedResource: true })
  assert.throws(() => prepare(paths), /private build metadata/)
})

test('macOS release requires the File Provider before stripping', () => {
  const paths = fixture({ extension: false })
  const calls = []
  assert.throws(
    () =>
      prepareMacosReleaseBinaries({
        ...paths,
        dryRun: false,
        env: {},
        repoRoot: paths.root,
        run: (...args) => calls.push(args),
      }),
    /Missing built macOS release component/,
  )
  assert.deepEqual(calls, [])
})

test('macOS release dry-run plans both strips without touching disk', () => {
  const root = join(mkdtempSync(join(tmpdir(), 'iris-drive-macos-dry-run-')), 'missing')
  const appPath = join(root, 'Iris Drive.app')
  const { calls } = prepare({ appPath, cliPath: '', root }, { dryRun: true })

  assert.equal(calls.length, 2)
  assert.equal(existsSync(appPath), false)
})
