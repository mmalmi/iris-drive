#!/usr/bin/env node
// Run one iOS release phase (export, upload, TestFlight, App Store) for the
// hosted unsigned iOS product of this exact clean checkout.
import { createHash } from 'node:crypto'
import { execFileSync } from 'node:child_process'
import { existsSync, readdirSync, readFileSync, writeFileSync } from 'node:fs'
import { dirname, join, resolve } from 'node:path'
import { fileURLToPath, pathToFileURL } from 'node:url'
import { buildNumberFromVersion, readWorkspaceVersionTag } from './local-release-lib.mjs'
import { loadReleaseEnv, runTool } from './release-env.mjs'

export const iosPhases = {
  'ios-build': ['ios-export', 'ios-upload'],
  'testflight-internal': ['put', 'status'],
  'testflight-public': ['put', 'status'],
  'app-store': ['status', 'submit'],
}
// Uploads and TestFlight changes must use the IPA recorded right after export.
const needsRecordedIpa = new Set(['ios-upload', 'put'])
const iosSettingPrefixes = ['IRIS_DRIVE_ASC_', 'IRIS_DRIVE_TESTFLIGHT_', 'IRIS_DRIVE_ITMS_', 'IRIS_DRIVE_APP_STORE_']

function recordedIpa(stage) {
  const eligible = join(stage, 'eligible')
  const ipas = existsSync(eligible) ? readdirSync(eligible).filter(name => name.endsWith('.ipa')) : []
  if (ipas.length !== 1) throw new Error(`Expected one exported IPA in ${eligible}; run ios-build ios-export first`)
  const data = readFileSync(join(eligible, ipas[0]))
  return { name: ipas[0], sha256: createHash('sha256').update(data).digest('hex') }
}

export function iosReleaseLayout(repoRoot) {
  const version = readWorkspaceVersionTag(readFileSync(join(repoRoot, 'Cargo.toml'), 'utf8')).slice(1)
  const build = String(buildNumberFromVersion(version))
  const commit = execFileSync('git', ['rev-parse', 'HEAD'], { cwd: repoRoot, encoding: 'utf8' }).trim()
  const stage = join(repoRoot, 'dist', `ios-${version}-${build}-${commit.slice(0, 8)}`)
  return { version, build, commit, stage, notesDir: join(repoRoot, 'artifacts', `release-${version}`, 'notes') }
}

export function iosReleasePhase({
  repoRoot, command, action, productDirectory, notesDir, expectedCommit, fileEnv = {}, env = process.env,
  run = runTool,
}) {
  if (!iosPhases[command]?.includes(action)) throw new Error(`Unknown iOS release phase: ${command} ${action}`)
  const layout = iosReleaseLayout(repoRoot)
  if (expectedCommit && expectedCommit !== layout.commit) throw new Error('The release checkout is not at the expected commit')
  if (execFileSync('git', ['status', '--porcelain', '--untracked-files=all'], { cwd: repoRoot, encoding: 'utf8' }).trim()) {
    throw new Error('iOS release phases require a clean release checkout')
  }
  if (command === 'ios-build' && !existsSync(join(productDirectory ?? '', 'build-receipt.json'))) {
    throw new Error('Pass --product with the downloaded hosted ios-release product')
  }
  const record = join(layout.stage, 'eligible.ipa.sha256')
  if (needsRecordedIpa.has(action)) {
    const expected = existsSync(record) ? readFileSync(record, 'utf8').trim() : ''
    const actual = recordedIpa(layout.stage)
    if (expected !== `${actual.sha256}  ${actual.name}`) {
      throw new Error('The exported IPA differs from the one recorded after export; export again')
    }
  }
  const settings = Object.fromEntries(Object.entries(fileEnv).filter(([key]) =>
    iosSettingPrefixes.some(prefix => key.startsWith(prefix))))
  const notes = notesDir ?? layout.notesDir
  const beta = join(notes, 'beta-notes.txt')
  const store = join(notes, 'store-notes.json')
  const phaseEnv = {
    ...settings, ...env,
    IRIS_DRIVE_IOS_DIST_DIR: layout.stage,
    IRIS_DRIVE_IOS_EXPORT_DIR: join(layout.stage, 'eligible'),
    IRIS_DRIVE_IOS_INTERNAL_ONLY: 'false',
    IRIS_DRIVE_IOS_MARKETING_VERSION: layout.version,
    IRIS_DRIVE_IOS_BUILD_NUMBER: layout.build,
    ...(productDirectory ? { IRIS_DRIVE_IOS_HOSTED_PRODUCT_DIR: resolve(productDirectory) } : {}),
    ...(existsSync(beta) ? { IRIS_DRIVE_TESTFLIGHT_WHATS_NEW: readFileSync(beta, 'utf8').trim() } : {}),
    ...(existsSync(store) ? { IRIS_DRIVE_APP_STORE_NOTES_PATH: store } : {}),
  }
  if (command === 'app-store' && action === 'submit' && !phaseEnv.IRIS_DRIVE_APP_STORE_NOTES_PATH) {
    throw new Error(`App Store submission needs release notes at ${store}`)
  }
  console.log(`iOS ${layout.version} (${layout.build}) from ${layout.commit.slice(0, 8)}: ${command} ${action}`)
  process.umask(0o077)
  run(join(repoRoot, 'scripts', command), [action], { env: phaseEnv, cwd: repoRoot })
  if (action === 'ios-export') {
    const exported = recordedIpa(layout.stage)
    writeFileSync(record, `${exported.sha256}  ${exported.name}\n`)
    console.log(`recorded ${exported.name} ${exported.sha256}`)
  }
  return layout
}

if (process.argv[1] && import.meta.url === pathToFileURL(process.argv[1]).href) {
  try {
    const [command, action, ...rest] = process.argv.slice(2)
    const options = { envFiles: [] }
    for (let index = 0; index < rest.length; index += 2) {
      const key = { '--product': 'productDirectory', '--notes': 'notesDir', '--commit': 'expectedCommit',
        '--env-file': 'envFile' }[rest[index]]
      if (!key || rest[index + 1] === undefined) throw new Error(`Unknown argument: ${rest[index]}`)
      const value = rest[index + 1]
      if (key === 'envFile') options.envFiles.push(resolve(value))
      else options[key] = key === 'expectedCommit' ? value : resolve(value)
    }
    const repoRoot = resolve(dirname(fileURLToPath(import.meta.url)), '..')
    const fileEnv = loadReleaseEnv({ repoRoot, envFiles: options.envFiles, env: {} })
    iosReleasePhase({ repoRoot, command, action, ...options, fileEnv, run: (program, args, opts) =>
      execFileSync(program, args, { ...opts, stdio: 'inherit' }) })
  } catch (error) {
    console.error(`error: ${error.message}`)
    process.exitCode = 1
  }
}
