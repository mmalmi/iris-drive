import test from 'node:test'
import assert from 'node:assert/strict'
import { mkdtempSync, rmSync, symlinkSync, truncateSync, writeFileSync } from 'node:fs'
import { tmpdir } from 'node:os'
import { join } from 'node:path'
import { macosDmgSizeMiB } from './macos-release-dmg.mjs'

test('DMG sizing counts sparse logical data and leaves filesystem reserve', (t) => {
  const root = mkdtempSync(join(tmpdir(), 'iris-drive-dmg-size-'))
  t.after(() => rmSync(root, { recursive: true, force: true }))
  const payload = join(root, 'idrive')
  writeFileSync(payload, '')
  truncateSync(payload, 768 * 1024 * 1024)

  // The large file occupies almost no source blocks, but must fit after copying.
  assert.ok(macosDmgSizeMiB(root) >= 845)
  assert.ok(macosDmgSizeMiB(root) <= 846)
  truncateSync(payload, 1)
  assert.equal(macosDmgSizeMiB(root), 65)
})

test('DMG sizing does not traverse Applications, cyclic, or dangling symlinks', (t) => {
  const root = mkdtempSync(join(tmpdir(), 'iris-drive-dmg-links-'))
  t.after(() => rmSync(root, { recursive: true, force: true }))
  symlinkSync('/Applications', join(root, 'Applications'))
  symlinkSync(root, join(root, 'cycle'))
  symlinkSync(join(root, 'missing'), join(root, 'dangling'))
  assert.equal(macosDmgSizeMiB(root), 65)
  assert.throws(() => macosDmgSizeMiB(join(root, 'missing')), { code: 'ENOENT' })
})
