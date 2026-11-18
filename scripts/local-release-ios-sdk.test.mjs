import assert from 'node:assert/strict'
import { spawnSync } from 'node:child_process'
import { mkdtempSync, rmSync } from 'node:fs'
import { tmpdir } from 'node:os'
import { join } from 'node:path'
import { fileURLToPath } from 'node:url'
import test from 'node:test'

const check = fileURLToPath(new URL('./check-ios-release-sdk.py', import.meta.url))

test('release SDK check covers the main app and extensions in both archives and IPAs', t => {
  const work = mkdtempSync(join(tmpdir(), 'drive-ios-sdk-test-'))
  t.after(() => rmSync(work, { recursive: true, force: true }))

  for (const [appSdk, extensionSdk, valid] of [
    ['iphoneos27.2', 'iphoneos27.2', true],
    ['iphoneos28.0', 'iphoneos28.0', true],
    ['iphoneos26.2', 'iphoneos27.2', false],
    ['iphoneos27.2', 'iphoneos26.2', false],
    ['iphoneos27.2', 'iphonesimulator27.2', false],
    ['iphoneos27.2', '', false],
  ]) {
    const archive = join(work, 'Drive.xcarchive')
    const ipa = join(work, 'Drive.ipa')
    const fixture = spawnSync('python3', ['-c', `
import plistlib
from pathlib import Path
import sys
from zipfile import ZipFile

archive, ipa, app_sdk, extension_sdk = sys.argv[1:]
with ZipFile(ipa, "w") as zipped:
    for bundle, sdk in [("Iris Drive.app", app_sdk),
                        ("Iris Drive.app/PlugIns/Share.appex", extension_sdk)]:
        info = plistlib.dumps({"DTSDKName": sdk} if sdk else {}, fmt=plistlib.FMT_BINARY)
        name = bundle + "/Info.plist"
        path = Path(archive) / "Products/Applications" / name
        path.parent.mkdir(parents=True, exist_ok=True)
        path.write_bytes(info)
        zipped.writestr("Payload/" + name, info)
`, archive, ipa, appSdk, extensionSdk], { encoding: 'utf8' })
    assert.equal(fixture.status, 0, fixture.stderr)

    for (const [kind, path] of [['archive', archive], ['ipa', ipa]]) {
      const result = spawnSync('python3', [check, kind, path], { encoding: 'utf8' })
      assert.equal(result.status, valid ? 0 : 1, result.stderr)
      if (!valid) assert.match(result.stderr, /iOS 27 SDK or later/)
    }
  }
})
