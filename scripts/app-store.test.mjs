import assert from 'node:assert/strict'
import { spawn } from 'node:child_process'
import { generateKeyPairSync } from 'node:crypto'
import { mkdtempSync, rmSync, writeFileSync } from 'node:fs'
import { createServer } from 'node:http'
import { tmpdir } from 'node:os'
import { join } from 'node:path'
import { fileURLToPath } from 'node:url'
import test from 'node:test'

async function fixture(t, options = {}) {
  const root = mkdtempSync(join(tmpdir(), 'iris-drive-app-store-test-'))
  t.after(() => rmSync(root, { recursive: true, force: true }))
  const keyPath = join(root, 'AuthKey_TEST.p8')
  writeFileSync(keyPath, generateKeyPairSync('ec', { namedCurve: 'P-256' }).privateKey.export({ type: 'pkcs8', format: 'pem' }))
  const notesPath = join(root, 'notes.json')
  writeFileSync(notesPath, JSON.stringify({ 'en-US': 'Improve local peer discovery and idle usage.' }))
  const build = { type: 'builds', id: 'BUILD', attributes: { version: '1038', processingState: 'VALID', buildAudienceType: 'APP_STORE_ELIGIBLE', usesNonExemptEncryption: false, ...options.build }, relationships: { preReleaseVersion: { data: { type: 'preReleaseVersions', id: 'PRE' } } } }
  const live = { type: 'appStoreVersions', id: 'LIVE', attributes: { platform: 'IOS', versionString: '0.1.32', appStoreState: 'READY_FOR_DISTRIBUTION', copyright: 'Existing copyright', releaseType: 'AFTER_APPROVAL' } }
  let version = options.version ? { type: 'appStoreVersions', id: 'VERSION', attributes: { ...live.attributes, versionString: '0.1.37', appStoreState: 'PREPARE_FOR_SUBMISSION', ...options.version } } : null
  let linkedBuild = options.linkedBuild ?? null
  let review = options.review ? { type: 'reviewSubmissions', id: 'REVIEW', attributes: { platform: 'IOS', state: 'READY_FOR_REVIEW', ...options.review } } : null
  let items = options.items ?? []
  let notes = 'Old notes'
  const calls = []
  const server = createServer(async (request, response) => {
    let raw = ''
    for await (const chunk of request) raw += chunk
    const body = raw ? JSON.parse(raw) : undefined
    const url = new URL(request.url, `http://${request.headers.host}`)
    const path = url.pathname.slice(4)
    const method = request.method
    calls.push({ method, path, body })
    assert.match(request.headers.authorization, /^Bearer /)
    const send = (data, status = 200, extra = {}) => { response.writeHead(status, { 'Content-Type': 'application/json' }); response.end(JSON.stringify({ data, ...extra })) }
    if (options.errorPath === path) return send(null, 409, { errors: [{ title: 'Controlled conflict' }] })
    if (method === 'GET') {
      if (path === 'apps') return send([{ type: 'apps', id: 'APP', attributes: { bundleId: options.wrongApp ? 'wrong.bundle' : 'fi.siriusbusiness.drive', primaryLocale: 'en-US' } }])
      if (path === 'builds') return send([build], 200, { included: [{ type: 'preReleaseVersions', id: 'PRE', attributes: { version: options.marketingVersion ?? '0.1.37', platform: options.platform ?? 'IOS' } }] })
      if (path === 'builds/BUILD') return send(build)
      if (path === 'apps/APP/appStoreVersions') return send(version ? [version, live] : [live])
      if (path === 'appStoreVersions/VERSION') return send(version)
      if (path === 'appStoreVersions/VERSION/relationships/build') return send(linkedBuild ? { type: 'builds', id: linkedBuild } : null)
      if (path === 'appStoreVersions/VERSION/appStoreVersionLocalizations') return send([{ type: 'appStoreVersionLocalizations', id: 'LOCALE', attributes: { locale: 'en-US', description: options.missingMetadata ? '' : 'Existing description', supportUrl: 'https://iris.to', whatsNew: notes } }])
      if (path === 'appStoreVersions/VERSION/appStoreReviewDetail') return send({ type: 'appStoreReviewDetails', id: 'DETAIL', attributes: { contactFirstName: 'Reviewer', contactLastName: 'Contact', contactPhone: '+10000000000', contactEmail: 'test@example.invalid', demoAccountRequired: false } })
      if (path === 'apps/APP/reviewSubmissions') return send(review ? [review] : [])
      if (path === 'reviewSubmissions/REVIEW/items') {
        const includeVersion = url.searchParams.get('include')?.split(',').includes('appStoreVersion')
        return send(items.map(item => includeVersion ? item : { ...item, relationships: undefined }))
      }
      if (path === 'reviewSubmissions/REVIEW') return send(review)
    }
    if (method === 'POST' && path === 'appStoreVersions') {
      assert.deepEqual(body.data.relationships, { app: { data: { type: 'apps', id: 'APP' } } })
      version = { type: 'appStoreVersions', id: 'VERSION', attributes: { ...live.attributes, ...body.data.attributes, appStoreState: 'PREPARE_FOR_SUBMISSION' } }
      return send(version, 201)
    }
    if (method === 'PATCH' && path === 'appStoreVersions/VERSION/relationships/build') { linkedBuild = options.ignoreAttach ? null : body.data.id; return send(body.data) }
    if (method === 'PATCH' && path === 'appStoreVersionLocalizations/LOCALE') { notes = body.data.attributes.whatsNew; return send({ type: 'appStoreVersionLocalizations', id: 'LOCALE' }) }
    if (method === 'POST' && path === 'reviewSubmissions') {
      assert.deepEqual(body.data.relationships, { app: { data: { type: 'apps', id: 'APP' } } })
      review = { type: 'reviewSubmissions', id: 'REVIEW', attributes: { platform: 'IOS', state: 'READY_FOR_REVIEW' } }
      return send(review, 201)
    }
    if (method === 'POST' && path === 'reviewSubmissionItems') {
      assert.deepEqual(body.data.relationships.reviewSubmission, { data: { type: 'reviewSubmissions', id: 'REVIEW' } })
      items.push({ type: 'reviewSubmissionItems', id: 'ITEM', attributes: { state: 'READY_FOR_REVIEW' }, relationships: { appStoreVersion: body.data.relationships.appStoreVersion } })
      return send(items.at(-1), 201)
    }
    if (method === 'PATCH' && path === 'reviewSubmissions/REVIEW') {
      assert.equal(body.data.attributes.submitted, true)
      if (!options.ignoreSubmit) review.attributes.state = 'WAITING_FOR_REVIEW'
      return send(review)
    }
    return send(null, 404, { errors: [{ title: `Unexpected ${method} ${path}` }] })
  })
  await new Promise(resolve => server.listen(0, '127.0.0.1', resolve))
  t.after(() => { server.closeAllConnections(); server.close() })
  const run = (action, overrides = {}) => new Promise(resolve => {
    const env = { ...process.env, IRIS_DRIVE_ASC_BASE_URL: `http://127.0.0.1:${server.address().port}/v1/`, IRIS_DRIVE_ASC_AUTH_KEY_PATH: keyPath, IRIS_DRIVE_ASC_AUTH_KEY_ID: 'TEST', IRIS_DRIVE_ASC_AUTH_KEY_ISSUER_ID: '00000000-0000-0000-0000-000000000000', IRIS_DRIVE_IOS_MARKETING_VERSION: '0.1.37', IRIS_DRIVE_IOS_BUILD_NUMBER: '1038', IRIS_DRIVE_APP_STORE_NOTES_PATH: notesPath, ...overrides }
    const child = spawn('bash', ['scripts/app-store', action], { cwd: fileURLToPath(new URL('..', import.meta.url)), env })
    let stdout = '', stderr = ''
    child.stdout.on('data', chunk => { stdout += chunk })
    child.stderr.on('data', chunk => { stderr += chunk })
    child.on('close', status => resolve({ status, stdout, stderr }))
  })
  return { run, calls, writes: () => calls.filter(c => c.method !== 'GET') }
}

const exactItem = { type: 'reviewSubmissionItems', id: 'ITEM', attributes: { state: 'READY_FOR_REVIEW' }, relationships: { appStoreVersion: { data: { type: 'appStoreVersions', id: 'VERSION' } } } }

test('App Store prepare and submit use the real client, exact build, inherited metadata and one resumable review', async t => {
  const f = await fixture(t)
  let result = await f.run('prepare')
  assert.equal(result.status, 0, result.stderr)
  assert.equal(f.writes().filter(c => c.path === 'appStoreVersions').length, 1)
  assert.equal(f.writes().filter(c => c.path === 'reviewSubmissions').length, 0)
  result = await f.run('submit')
  assert.equal(result.status, 0, result.stderr)
  assert.match(result.stdout, /WAITING_FOR_REVIEW/)
  const writes = f.writes().length
  result = await f.run('submit')
  assert.equal(result.status, 0, result.stderr)
  assert.equal(f.writes().length, writes, 'resuming an already submitted exact build must not mutate')
  assert.equal(f.writes().filter(c => c.path === 'reviewSubmissionItems').length, 1)
  assert.equal(f.writes().filter(c => c.path === 'reviewSubmissions').length, 1)
})

test('App Store status is read-only even for an internal build and does not require notes', async t => {
  const f = await fixture(t, { build: { buildAudienceType: 'INTERNAL_ONLY' } })
  const result = await f.run('status', { IRIS_DRIVE_APP_STORE_NOTES_PATH: '' })
  assert.equal(result.status, 0, result.stderr)
  assert.match(result.stdout, /INTERNAL_ONLY/)
  assert.equal(f.writes().length, 0)
})

for (const [name, linkedBuild, itemVersion, buildMatches, reviewMatches] of [
  ['a different attached build', 'OTHER_BUILD', 'VERSION', false, true],
  ['an unrelated review version', 'BUILD', 'OTHER_VERSION', true, false],
  ['the exact submitted build and version', 'BUILD', 'VERSION', true, true],
]) test(`App Store status identifies ${name} without writes`, async t => {
  const f = await fixture(t, {
    version: { appStoreState: 'WAITING_FOR_REVIEW' }, linkedBuild,
    review: { state: 'WAITING_FOR_REVIEW' },
    items: [{ ...exactItem, relationships: { appStoreVersion: { data: { type: 'appStoreVersions', id: itemVersion } } } }],
  })
  const result = await f.run('status', { IRIS_DRIVE_APP_STORE_NOTES_PATH: '' })
  assert.equal(result.status, 0, result.stderr)
  const status = JSON.parse(result.stdout)
  assert.deepEqual(status.requestedBuild, { version: '1038', id: 'BUILD' })
  assert.equal(status.attachedBuildId, linkedBuild)
  assert.equal(status.buildMatchesRequested, buildMatches)
  assert.deepEqual(status.reviewSubmissions, [{
    id: 'REVIEW', state: 'WAITING_FOR_REVIEW', appStoreVersionIds: [itemVersion],
    matchesRequestedVersion: reviewMatches,
  }])
  assert.equal(f.writes().length, 0)
})

for (const [name, options, error] of [
  ['internal build', { build: { buildAudienceType: 'INTERNAL_ONLY' } }, /INTERNAL_ONLY/],
  ['unconfirmed audience', { build: { buildAudienceType: null } }, /eligibility/],
  ['invalid build', { build: { processingState: 'PROCESSING' } }, /VALID/],
  ['expired build', { build: { expired: true } }, /expired/],
  ['unknown export compliance', { build: { usesNonExemptEncryption: null } }, /compliance/],
  ['wrong marketing version', { marketingVersion: '0.1.36' }, /No .*build/],
  ['wrong platform', { platform: 'MAC_OS' }, /No .*build/],
  ['wrong app', { wrongApp: true }, /No App Store Connect app/],
  ['other build attached', { version: {}, linkedBuild: 'OTHER' }, /different build/],
  ['unrelated review item', { version: {}, linkedBuild: 'BUILD', review: {}, items: [exactItem, { ...exactItem, id: 'OTHER', relationships: { appEvent: { data: { type: 'appEvents', id: 'EVENT' } } } }] }, /unrelated/],
  ['unresolved review', { version: {}, linkedBuild: 'BUILD', review: { state: 'UNRESOLVED_ISSUES' }, items: [exactItem] }, /UNRESOLVED_ISSUES/],
]) test(`App Store refuses ${name} before writes`, async t => {
  const f = await fixture(t, options)
  const result = await f.run('submit')
  assert.equal(result.status, 1, result.stdout)
  assert.match(result.stderr, error)
  assert.equal(f.writes().length, 0)
})

test('App Store validates explicit inputs before contacting Apple', async t => {
  const f = await fixture(t)
  for (const key of ['IRIS_DRIVE_IOS_MARKETING_VERSION', 'IRIS_DRIVE_IOS_BUILD_NUMBER', 'IRIS_DRIVE_APP_STORE_NOTES_PATH']) {
    const result = await f.run('submit', { [key]: '' })
    assert.equal(result.status, 1)
    assert.match(result.stderr, /required|Required|Missing/)
  }
  assert.equal(f.calls.length, 0)
})

for (const [name, options, error] of [
  ['missing inherited metadata', { version: {}, missingMetadata: true }, /description/],
  ['failed attachment readback', { ignoreAttach: true }, /attached/],
  ['Apple conflict', { version: {}, errorPath: 'appStoreVersions/VERSION/relationships/build' }, /409.*|Controlled conflict/],
  ['unconfirmed submission', { ignoreSubmit: true }, /not confirmed/],
]) test(`App Store reports ${name} and does not claim success`, async t => {
  const f = await fixture(t, options)
  const result = await f.run('submit')
  assert.equal(result.status, 1, result.stdout)
  assert.match(result.stderr, error)
  assert.doesNotMatch(result.stdout, /WAITING_FOR_REVIEW/)
})

for (const items of [[], [exactItem]]) test(`App Store resumes a ready review with ${items.length} existing items`, async t => {
  const f = await fixture(t, { version: {}, linkedBuild: 'BUILD', review: {}, items: structuredClone(items) })
  const result = await f.run('submit')
  assert.equal(result.status, 0, result.stderr)
  assert.equal(f.writes().filter(c => c.path === 'reviewSubmissions').length, 0)
  assert.equal(f.writes().filter(c => c.path === 'reviewSubmissionItems').length, 1 - items.length)
})
