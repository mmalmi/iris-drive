#!/usr/bin/env node

import { spawnSync } from 'node:child_process'
import {
  copyFileSync,
  existsSync,
  mkdirSync,
  readFileSync,
  readdirSync,
  rmSync,
  statSync,
  writeFileSync,
} from 'node:fs'
import os from 'node:os'
import { basename, dirname, join, resolve } from 'node:path'
import process from 'node:process'
import { fileURLToPath } from 'node:url'

import {
  buildReleaseManifest,
  buildReleaseManifestFiles,
  buildZapstorePublishPlan,
  normalizeTag,
  parseEnvFile,
  readWorkspaceVersionTag,
  renderReleaseNotes,
  validateReleaseAssetSet,
} from './local-release-lib.mjs'

const __dirname = dirname(fileURLToPath(import.meta.url))
const repoRoot = resolve(__dirname, '..')
const rootCargoToml = join(repoRoot, 'Cargo.toml')
const distDir = join(repoRoot, 'dist')
const defaultEnvFiles = [join(repoRoot, '.env.release.local'), join(repoRoot, '.env.zapstore.local')]

function usage() {
  console.log(`Usage: node scripts/local-release.mjs [options]

Stage existing dist artifacts as a hashtree updater release, and optionally
publish the staged release tree.

Options:
  --publish              Publish the staged tree with htree release publish
  --final                Publish as final/latest instead of draft
  --draft                Publish as draft (default when --publish is set)
  --tag <tag>            Release tag (default: workspace version)
  --release-tree <name>  htree release tree name (default: releases/iris-drive)
  --asset-dir <path>     Artifact directory (default: dist)
  --stage-dir <path>     Staging directory
  --env-file <path>      Extra dotenv file to load
  --skip-zapstore        With --final, skip publishing the Android APK to Zapstore
  --dry-run              Print actions without copying or publishing
  --help                 Show this help`)
}

function parseArgs(argv) {
  const options = {
    publish: false,
    draft: true,
    tag: null,
    releaseTree: null,
    assetDir: null,
    stageDir: null,
    envFiles: [],
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

function quote(arg) {
  const value = String(arg)
  return /[^\w./:-]/.test(value) ? JSON.stringify(value) : value
}

function run(command, args, { capture = false, dryRun = false, env = process.env } = {}) {
  const rendered = [command, ...args].map(quote).join(' ')
  console.log(`$ ${rendered}`)
  if (dryRun) {
    return ''
  }
  const result = spawnSync(command, args, {
    cwd: repoRoot,
    env,
    encoding: 'utf8',
    stdio: capture ? 'pipe' : 'inherit',
  })
  if (result.status !== 0) {
    const stderr = capture ? result.stderr.trim() : ''
    throw new Error(stderr || `${command} exited with status ${result.status ?? 'unknown'}`)
  }
  return capture ? result.stdout.trim() : ''
}

function commandExists(command) {
  const result =
    process.platform === 'win32'
      ? spawnSync('where', [command], { stdio: 'ignore' })
      : spawnSync('sh', ['-lc', `command -v "${command}"`], { stdio: 'ignore' })
  return result.status === 0
}

function collectReleaseAssetPaths(assetDir, tag) {
  if (!existsSync(assetDir)) {
    return []
  }
  return readdirSync(assetDir)
    .sort()
    .filter((entry) => entry.includes(tag))
    .map((entry) => join(assetDir, entry))
    .filter((path) => statSync(path).isFile())
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
  requireCompleteAppRelease = false,
}) {
  const assetPaths = collectReleaseAssetPaths(assetDir, tag)
  validateReleaseAssetSet(assetPaths.map((assetPath) => basename(assetPath)), {
    requireCompleteAppRelease,
  })
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

function publishRelease({ stageDir, releaseTree, tag, draft, dryRun }) {
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
  run('htree', args, { dryRun })
  return cid
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

function publishZapstore({ env, tag, assetDir, dryRun }) {
  const signWith = resolveZapstoreSignWith(env)
  const zapstoreYaml = join(repoRoot, 'zapstore.yaml')
  const normalizedTag = normalizeTag(tag)
  const plan = buildZapstorePublishPlan({
    tag: normalizedTag,
    assetDir,
    distDir,
    apkExists: existsSync(join(assetDir, `iris-drive-${normalizedTag}-android-arm64.apk`)),
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
  const tag = options.tag || readWorkspaceVersionTag(readFileSync(rootCargoToml, 'utf8'))
  const releaseTree = options.releaseTree || env.IRIS_DRIVE_RELEASE_TREE || 'releases/iris-drive'
  const assetDir = options.assetDir || join(repoRoot, 'dist')
  const stageDir =
    options.stageDir || join(os.tmpdir(), `iris-drive-release-${tag.replace(/[^\w.-]/g, '_')}`)
  const commit = resolveReleaseCommit(tag, options.dryRun)

  console.log(`Release tag: ${tag}`)
  console.log(`Release tree: ${releaseTree}`)
  console.log(`Asset dir: ${assetDir}`)
  console.log(`Stage dir: ${stageDir}`)

  stageRelease({
    tag,
    commit,
    assetDir,
    stageDir,
    draft: options.publish ? options.draft : true,
    dryRun: options.dryRun,
    requireCompleteAppRelease: options.publish && !options.draft,
  })

  if (options.publish) {
    if (!commandExists('htree')) {
      throw new Error('Missing htree; cannot publish release')
    }
    const cid = publishRelease({
      stageDir,
      releaseTree,
      tag,
      draft: options.draft,
      dryRun: options.dryRun,
    })
    console.log(`Published ${options.draft ? 'draft ' : ''}${tag} to ${releaseTree} via ${cid}`)
    if (!options.draft && !options.skipZapstore) {
      publishZapstore({ env, tag, assetDir, dryRun: options.dryRun })
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
