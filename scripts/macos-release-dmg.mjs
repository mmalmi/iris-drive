import { lstatSync, mkdirSync, readdirSync, rmSync, symlinkSync } from 'node:fs'
import { basename, join } from 'node:path'

export function macosDmgSizeMiB(root) {
  const pending = [root]
  let payload = 0
  while (pending.length) {
    const path = pending.pop()
    const entry = lstatSync(path)
    // Count logical bytes, including sparse files, and never follow Applications
    // or bundle symlinks outside the staged tree.
    payload += Math.max(4096, Math.ceil(entry.size / 4096) * 4096)
    if (entry.isDirectory()) {
      pending.push(...readdirSync(path).map((name) => join(path, name)))
    }
  }
  // Leave space for filesystem metadata and copy transactions instead of relying
  // on hdiutil's automatic estimate, which can run out of space during the copy.
  const mib = 1024 * 1024
  return Math.ceil((payload + Math.max(64 * mib, payload * 0.1)) / mib)
}

export function createMacosDmg({ appPath, dmgPath, dryRun, env, repoRoot, run }) {
  const dmgRoot = join(repoRoot, 'macos', '.build', 'ReleaseDmgRoot')
  const applicationsLink = join(dmgRoot, 'Applications')
  if (!dryRun) {
    rmSync(dmgRoot, { recursive: true, force: true })
    mkdirSync(dmgRoot, { recursive: true })
  }
  run('ditto', [appPath, join(dmgRoot, basename(appPath))], { dryRun, env })
  if (dryRun) {
    console.log(`Would link /Applications -> ${applicationsLink}`)
  } else {
    symlinkSync('/Applications', applicationsLink)
  }
  const sizeMiB = dryRun ? '<calculated from staged files>' : String(macosDmgSizeMiB(dmgRoot))
  run('hdiutil', [
    'create', '-volname', 'Iris Drive', '-srcfolder', dmgRoot,
    '-megabytes', sizeMiB, '-ov', '-format', 'UDZO', dmgPath,
  ], { dryRun, env })
}
