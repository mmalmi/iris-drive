import assert from 'node:assert/strict'
import { readFileSync } from 'node:fs'
import test from 'node:test'

const read = (path) => readFileSync(new URL(`../${path}`, import.meta.url), 'utf8')

function functionBody(source, name, nextName) {
  return source.slice(source.indexOf(`${name}() {`), source.indexOf(`${nextName}() {`))
}

test('Apple main apps embed the project license as a bundle resource', () => {
  for (const platform of ['macos', 'ios']) {
    const spec = read(`${platform}/project.yml`)
    const project = read(
      `${platform}/${platform === 'macos' ? 'IrisDriveMac' : 'IrisDriveIOS'}.xcodeproj/project.pbxproj`,
    )

    assert.match(spec, /- path: \.\.\/LICENSE\n\s+buildPhase: resources/)
    assert.match(project, /LICENSE in Resources/)
    assert.match(project, /path = \.\.;/)
    assert.doesNotMatch(project, /Embed Iris Drive.*license/)
  }
})

test('Android wires every project-license asset consumer to its staging task', () => {
  const gradle = read('android/app/build.gradle.kts')

  assert.match(gradle, /repoRoot\.file\("LICENSE"\)/)
  assert.match(gradle, /generated\/irisDriveLicenseAssets/)
  assert.match(gradle, /assets\.srcDir\(generatedLicenseAssetsDir\)/)

  const consumerTasks = gradle.match(
    /tasks\.matching \{ task ->([\s\S]*?)\}\.configureEach \{\s*dependsOn\(stageProjectLicenseAssets\)\s*\}/,
  )?.[1]
  assert.ok(consumerTasks)
  assert.match(consumerTasks, /startsWith\("merge"\)/)
  assert.match(consumerTasks, /endsWith\("Assets"\)/)
  assert.match(consumerTasks, /contains\("lint", ignoreCase = true\)/)
  assert.match(consumerTasks, /endsWith\("Model"\)/)
  assert.match(consumerTasks, /contains\("Analyze"\)/)
  assert.doesNotMatch(consumerTasks, /generateReleaseLintVitalReportModel/)
})

test('macOS release smoke checks every accepted app license before signature acceptance', () => {
  const smoke = read('scripts/macos-release-smoke.sh')
  const verifier = functionBody(smoke, 'verify_app_bundle', 'verify_dmg')

  assert.match(smoke, /verify_embedded_license\(\)/)
  assert.match(smoke, /Contents\/Resources\/LICENSE/)
  assert.match(smoke, /\/usr\/bin\/cmp -s "\$ROOT\/LICENSE"/)
  assert.ok(verifier.indexOf('verify_embedded_license "$app"') >= 0)
  assert.ok(verifier.indexOf('verify_embedded_license "$app"') < verifier.indexOf('codesign'))
  assert.equal((smoke.match(/verify_app_bundle "\$/g) ?? []).length, 3)
})

test('iOS archive and IPA licenses are accepted before any upload', () => {
  const build = read('scripts/ios-build')
  const archive = functionBody(build, 'run_ios_archive', 'write_export_options')
  const exportIpa = functionBody(build, 'run_ios_export', 'ipa_path')
  const upload = functionBody(build, 'run_ios_upload', 'run_testflight_helper')

  assert.match(build, /verify_ios_archive_license\(\)/)
  assert.match(build, /Products\/Applications/)
  assert.match(build, /verify_ios_export_license\(\)/)
  assert.match(build, /Payload\/\[\^\/\]\+\\\.app\/LICENSE/)
  assert.match(build, /\/usr\/bin\/cmp -s "\$ROOT\/LICENSE"/)
  assert.ok(archive.lastIndexOf('xcodebuild') < archive.lastIndexOf('verify_ios_archive_license'))
  assert.ok(exportIpa.lastIndexOf('xcodebuild') < exportIpa.lastIndexOf('verify_ios_export_license'))
  assert.ok(upload.indexOf('verify_ios_export_license') < upload.indexOf('"$TRANSPORTER"'))
})
