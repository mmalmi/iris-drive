import { chmodSync, copyFileSync, existsSync, readdirSync } from 'node:fs'
import { join } from 'node:path'

import { assertNoPrivateBuildMetadata } from './release-build-hygiene.mjs'

function installCli(source, destination) {
  copyFileSync(source, destination)
  chmodSync(destination, 0o755)
}

function regularFiles(root) {
  return readdirSync(root, { withFileTypes: true }).flatMap((entry) => {
    const path = join(root, entry.name)
    return entry.isDirectory() ? regularFiles(path) : entry.isFile() ? [path] : []
  })
}

export function prepareMacosReleaseBinaries({ appPath, cliPath, dryRun, env, repoRoot, run }) {
  const appexPath = join(appPath, 'Contents', 'PlugIns', 'IrisDriveFileProvider.appex')
  const appExecutable = join(appPath, 'Contents', 'MacOS', 'Iris Drive')
  const appexExecutable = join(appexPath, 'Contents', 'MacOS', 'IrisDriveFileProvider')
  const appCliPath = join(appPath, 'Contents', 'MacOS', 'idrive')
  const appexCliPath = join(appexPath, 'Contents', 'MacOS', 'idrive')

  if (!dryRun) {
    for (const path of [appPath, appexPath, appExecutable, appexExecutable, cliPath]) {
      if (!existsSync(path)) throw new Error(`Missing built macOS release component: ${path}`)
    }
  }
  run('strip', ['-S', appExecutable], { dryRun, env })
  run('strip', ['-S', appexExecutable], { dryRun, env })
  if (!dryRun) {
    installCli(cliPath, appCliPath)
    installCli(cliPath, appexCliPath)
    assertNoPrivateBuildMetadata(regularFiles(appPath), env, { repoRoot })
  }
  return { appCliPath, appexCliPath, appexPath }
}
