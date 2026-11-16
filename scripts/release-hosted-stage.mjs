#!/usr/bin/env node
// Stage unsigned products from the "Unsigned native products" workflow into
// dist/ after checking their receipts against this exact clean checkout.
import { execFileSync } from 'node:child_process'
import { randomUUID } from 'node:crypto'
import { existsSync, linkSync, mkdirSync, mkdtempSync, readdirSync, readFileSync, unlinkSync, writeFileSync } from 'node:fs'
import { dirname, join, resolve } from 'node:path'
import { fileURLToPath, pathToFileURL } from 'node:url'
import { archiveDigest, verifyProductReceipt } from './hosted-product-receipt.mjs'
import { plannedReleaseAssetNames, readWorkspaceVersionTag } from './local-release-lib.mjs'
import { signHostedAndroid } from './release-android-signing.mjs'
import { gitHubRepository, loadReleaseEnv, mainCheckout, runTool } from './release-env.mjs'
import { stageHostedProduct } from './stage-hosted-product.mjs'

export const hostedWorkflowName = 'Unsigned native products'
export const stagedPlatforms = ['linux', 'windows', 'android']
export const verifiedOnlyPlatforms = ['macos-release', 'ios-release']

export function hostedWorkDir(repoRoot, version, commit) {
  return join(repoRoot, 'artifacts', `release-${version}`, 'hosted', commit)
}

export function downloadHostedProduct({ commit, platform, runId, repository, destination, run = runTool }) {
  if (!/^\d+$/.test(String(runId)) || !/^[\w.-]+\/[\w.-]+$/.test(repository ?? '')) {
    throw new Error('Name the hosted run by its numeric id and owner/name repository')
  }
  const view = JSON.parse(run('gh', ['run', 'view', String(runId), '-R', repository, '--json', 'workflowName,status']))
  if (view.workflowName !== hostedWorkflowName || view.status !== 'completed') {
    throw new Error(`Run ${runId} is not a completed "${hostedWorkflowName}" run`)
  }
  run('gh', ['run', 'download', String(runId), '-R', repository, '-n', `drive-${platform}-${commit}`, '-D', destination])
  return destination
}

async function linkIntoDist(output, files, distDir) {
  mkdirSync(distDir, { recursive: true })
  const outputs = {}
  for (const name of files) {
    const source = join(output, name)
    const dest = join(distDir, name)
    const expected = await archiveDigest(source)
    if (expected.bytes === 0) throw new Error(`${name} is empty`)
    if (existsSync(dest)) {
      const actual = await archiveDigest(dest)
      if (actual.sha256 !== expected.sha256) throw new Error(`dist/${name} differs from the verified hosted product`)
    } else {
      const temp = join(distDir, `.staging-${randomUUID()}`)
      try {
        linkSync(source, temp)
        linkSync(temp, dest)
      } finally {
        if (existsSync(temp)) unlinkSync(temp)
      }
    }
    outputs[name] = expected
  }
  return outputs
}

export async function stageHostedRelease({
  repoRoot, commit, platform, productDirectory, work, distDir = join(repoRoot, 'dist'),
  env = process.env, run = runTool, sign = signHostedAndroid, log = console.log,
}) {
  const git = (...args) => execFileSync('git', args, { cwd: repoRoot, encoding: 'utf8' }).trim()
  if (git('rev-parse', 'HEAD') !== commit) throw new Error('Stage hosted products from a checkout at the release commit')
  if (git('status', '--porcelain', '--untracked-files=all')) throw new Error('Prepared products require a clean source checkout')
  const tag = readWorkspaceVersionTag(readFileSync(join(repoRoot, 'Cargo.toml'), 'utf8'))
  const version = tag.slice(1)
  const receipt = JSON.parse(readFileSync(join(productDirectory, 'build-receipt.json'), 'utf8'))
  if (receipt.version !== version) throw new Error(`Hosted product is ${receipt.version}; this checkout releases ${version}`)
  const tree = git('rev-parse', 'HEAD^{tree}')
  const archive = join(productDirectory, `${platform}-products.tar.gz`)
  if (![...stagedPlatforms, ...verifiedOnlyPlatforms].includes(platform)) {
    throw new Error(`Unknown hosted platform: ${platform}`)
  }
  await verifyProductReceipt(receipt, archive, { commit, tree, platform })
  if (verifiedOnlyPlatforms.includes(platform)) return { productDirectory }
  const files = plannedReleaseAssetNames(tag, [platform])
  mkdirSync(work, { recursive: true })
  let stage
  if (platform === 'linux') {
    stage = stageHostedProduct({ repoRoot, productDirectory, platform, run, env, destinationParent: work })
  } else {
    stage = mkdtempSync(join(work, `prepared-${platform}-`))
    const select = fileURLToPath(new URL('./select-hosted-release-products.py', import.meta.url))
    log(String(run('python3', [select, platform, archive, stage, ...files], { env })).trim())
  }
  const output = mkdtempSync(join(work, `${platform}-verified-`))
  if (platform === 'android') {
    sign({ repoRoot, keyRoot: mainCheckout(repoRoot), stage, output, files, version, env, run })
  } else {
    const staged = readdirSync(stage).sort()
    if (JSON.stringify(staged) !== JSON.stringify([...files].sort())) {
      throw new Error(`Hosted ${platform} product does not hold exactly ${files.join(', ')}`)
    }
    for (const name of files) linkSync(join(stage, name), join(output, name))
  }
  const outputs = await linkIntoDist(output, files, distDir)
  writeFileSync(join(work, `${platform}-staged.json`), `${JSON.stringify({
    commit, tree, version, platform,
    input: { archive: receipt.archive, sha256: receipt.sha256, bytes: receipt.bytes }, outputs,
  }, null, 2)}\n`)
  log(`${platform}: ${files.length} verified artifacts staged in dist for ${tag}`)
  return { outputs }
}

function parseArgs(argv) {
  const options = { envFiles: [] }
  for (let index = 0; index < argv.length; index += 1) {
    const value = argv[index + 1]
    const key = { '--platform': 'platform', '--run': 'runId', '--repo': 'repository', '--product': 'product',
      '--commit': 'commit', '--env-file': 'envFile' }[argv[index]]
    if (!key || value === undefined) throw new Error(`Unknown argument: ${argv[index]}`)
    if (key === 'envFile') options.envFiles.push(resolve(value))
    else options[key] = value
    index += 1
  }
  if (!options.platform || !options.runId === !options.product) {
    throw new Error('Usage: node scripts/release-hosted-stage.mjs --platform <linux|windows|android|macos-release|ios-release> (--run <id> [--repo owner/name] | --product <dir>) [--commit <sha>] [--env-file <path>]')
  }
  return options
}

if (process.argv[1] && import.meta.url === pathToFileURL(process.argv[1]).href) {
  try {
    const options = parseArgs(process.argv.slice(2))
    const repoRoot = resolve(dirname(fileURLToPath(import.meta.url)), '..')
    const commit = options.commit ?? execFileSync('git', ['rev-parse', 'HEAD'], { cwd: repoRoot, encoding: 'utf8' }).trim()
    const version = readWorkspaceVersionTag(readFileSync(join(repoRoot, 'Cargo.toml'), 'utf8')).slice(1)
    const work = hostedWorkDir(repoRoot, version, commit)
    let productDirectory = options.product && resolve(options.product)
    if (!productDirectory) {
      productDirectory = join(work, `${options.platform}-run-${options.runId}`)
      if (!existsSync(join(productDirectory, 'build-receipt.json'))) {
        downloadHostedProduct({ commit, platform: options.platform, runId: options.runId,
          repository: options.repository ?? gitHubRepository(repoRoot), destination: productDirectory })
      }
    }
    const env = loadReleaseEnv({ repoRoot, envFiles: options.envFiles })
    await stageHostedRelease({ repoRoot, commit, platform: options.platform, productDirectory, work, env })
    if (verifiedOnlyPlatforms.includes(options.platform)) console.log(`verified hosted ${options.platform} product: ${productDirectory}`)
  } catch (error) {
    console.error(`error: ${error.message}`)
    process.exitCode = 1
  }
}
