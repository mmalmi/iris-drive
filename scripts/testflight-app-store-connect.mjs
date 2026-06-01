#!/usr/bin/env node

import { createPrivateKey, sign as cryptoSign } from 'node:crypto'
import { existsSync, readFileSync, readdirSync } from 'node:fs'
import os from 'node:os'
import { basename, dirname, join, resolve } from 'node:path'
import process from 'node:process'
import { fileURLToPath } from 'node:url'

const __dirname = dirname(fileURLToPath(import.meta.url))
const repoRoot = resolve(__dirname, '..')
loadEnvFileDefaults(join(repoRoot, '.env.release.local'))
const rawBaseUrl = process.env.IRIS_DRIVE_ASC_BASE_URL || 'https://api.appstoreconnect.apple.com/v1/'
const baseUrl = rawBaseUrl.endsWith('/') ? rawBaseUrl : `${rawBaseUrl}/`
const mode = process.argv[2] || ''
const action = process.argv[3] || 'put'
const publicActions = new Set(['put', 'submit', 'attach', 'status', 'groups'])
const internalActions = new Set(['ensure-app', 'put', 'attach', 'wait', 'status', 'groups', 'compliance'])

function usage() {
  console.log(`usage: scripts/testflight-internal <ensure-app|put|attach|wait|status|groups|compliance>
       scripts/testflight-public <put|submit|attach|status|groups>

Commands:
  ensure-app create the App Store Connect app record if it is missing
  put        wait for the uploaded build and publish it to the selected channel
  attach     attach an already-valid build to the selected TestFlight group(s)
  submit     public only: update metadata and submit Beta App Review
  wait       internal only: wait until the uploaded build is valid
  status     print build and group status
  groups     list TestFlight groups for the app
  compliance set export-compliance metadata on the uploaded build

Environment:
  IRIS_DRIVE_ASC_AUTH_KEY_PATH or IRIS_DRIVE_ASC_KEY_PATH
  IRIS_DRIVE_ASC_AUTH_KEY_ID or IRIS_DRIVE_ASC_KEY_ID
  IRIS_DRIVE_ASC_AUTH_KEY_ISSUER_ID or IRIS_DRIVE_ASC_ISSUER_ID
  IRIS_DRIVE_ASC_APP_NAME
  IRIS_DRIVE_ASC_APP_SKU
  IRIS_DRIVE_ASC_APP_PRIMARY_LOCALE
  IRIS_DRIVE_IOS_BUNDLE_ID
  IRIS_DRIVE_IOS_MARKETING_VERSION
  IRIS_DRIVE_IOS_BUILD_NUMBER
  IRIS_DRIVE_TESTFLIGHT_GROUPS
  IRIS_DRIVE_TESTFLIGHT_PUBLIC_GROUPS
  IRIS_DRIVE_TESTFLIGHT_PRIVACY_POLICY_URL
  IRIS_DRIVE_TESTFLIGHT_FEEDBACK_EMAIL
  IRIS_DRIVE_TESTFLIGHT_CONTACT_FIRST_NAME
  IRIS_DRIVE_TESTFLIGHT_CONTACT_LAST_NAME
  IRIS_DRIVE_TESTFLIGHT_CONTACT_PHONE
  IRIS_DRIVE_TESTFLIGHT_CONTACT_EMAIL`)
}

if (['-h', '--help', 'help'].includes(mode) || ['-h', '--help', 'help'].includes(action)) {
  usage()
  process.exit(0)
}

if (!['internal', 'public'].includes(mode)) {
  usage()
  process.exit(2)
}

if (mode === 'internal' && !internalActions.has(action)) {
  fail(`Unknown internal TestFlight action: ${action}`, 2)
}
if (mode === 'public' && !publicActions.has(action)) {
  fail(`Unknown public TestFlight action: ${action}`, 2)
}

const appName = envValue(['IRIS_DRIVE_ASC_APP_NAME'], 'Iris Drive')
const bundleId = envValue(['IRIS_DRIVE_IOS_BUNDLE_ID'], 'to.iris.drive.ios')
const appSku = envValue(['IRIS_DRIVE_ASC_APP_SKU'], bundleId)
const appPrimaryLocale = envValue(['IRIS_DRIVE_ASC_APP_PRIMARY_LOCALE'], 'en-US')
const versionName = envValue(['IRIS_DRIVE_IOS_MARKETING_VERSION'], workspaceVersion())
const buildNumber = envValue(['IRIS_DRIVE_IOS_BUILD_NUMBER'], semanticVersionCode(versionName))
const waitAttempts = intEnv('IRIS_DRIVE_TESTFLIGHT_WAIT_ATTEMPTS', 40)
const waitSeconds = intEnv('IRIS_DRIVE_TESTFLIGHT_WAIT_SECONDS', 30)
const usesNonExemptEncryption = boolEnv('IRIS_DRIVE_TESTFLIGHT_USES_NONEXEMPT_ENCRYPTION', false)
const encryptionDeclarationId = envValue(['IRIS_DRIVE_TESTFLIGHT_APP_ENCRYPTION_DECLARATION_ID'])
const defaultPublicGroupName = 'Public Beta'
const defaultPublicLinkLimit = 9999
const defaultWhatsNew =
  'Please test device linking, file sync, provider browsing, uploads, downloads, and feedback or crash reporting.'
const defaultBetaDescription =
  'Iris Drive is a private file sync app that links the user-owned devices they choose and syncs files over the Iris Drive daemon.'
const defaultReviewNotes =
  'No demo account is required. To review, create or link a device in Iris Drive, then test file browsing and sync through the app provider surface. The app only syncs user-selected Iris Drive data between user-owned devices.'

let authToken = ''

try {
  await main()
} catch (error) {
  fail(error instanceof Error ? error.message : String(error))
}

async function main() {
  authToken = createToken(resolveAscAuth())
  if (action === 'ensure-app') {
    await ensureApp()
    return
  }
  const app = await findApp()
  if (!app) {
    fail(`No App Store Connect app found for bundle ${bundleId}. Run ensure-app first.`)
  }
  const appId = app.id

  if (action === 'groups') {
    await printGroups(appId)
    return
  }

  const build = ['put', 'wait'].includes(action)
    ? await waitForValidBuild(appId)
    : await requireVisibleBuild(appId)

  if (mode === 'internal') {
    await runInternal(appId, build)
  } else {
    await runPublic(appId, build)
  }
}

async function runInternal(appId, build) {
  if (action === 'wait') {
    await summarizeInternalBuild(appId, build)
    return
  }
  if (['put', 'attach', 'compliance'].includes(action)) {
    build = await ensureExportCompliance(build)
  }
  if (action === 'compliance') {
    await summarizeInternalBuild(appId, build)
    return
  }
  if (action === 'status') {
    await summarizeInternalBuild(appId, build)
    return
  }
  build = await waitForInternalTestableBuild(build)
  const selectedGroups = await selectInternalGroups(appId)
  await attachBuild(build, selectedGroups, 'internal')
  await summarizeInternalBuild(appId, build, selectedGroups)
}

async function runPublic(appId, build) {
  if (action === 'status') {
    const selectedGroups = await selectExternalGroups(appId, { createMissing: false })
    await summarizePublicBuild(build, selectedGroups)
    return
  }
  build = await ensureExportCompliance(build)
  build = await ensurePublicBuild(build)
  const selectedGroups = await selectExternalGroups(appId, {
    createMissing: action === 'put' || action === 'attach',
  })
  if (action === 'put' || action === 'submit') {
    await ensurePublicMetadata(appId)
    await upsertWhatToTest(build)
    await submitBetaReview(build)
  }
  if (action === 'put' || action === 'attach') {
    const groups = await ensurePublicGroupSettings(selectedGroups)
    await attachBuild(build, groups, 'external')
    await summarizePublicBuild(build, groups)
    return
  }
  await summarizePublicBuild(build, selectedGroups)
}

function loadEnvFileDefaults(path) {
  if (!existsSync(path)) {
    return
  }
  for (const line of readFileSync(path, 'utf8').split(/\r?\n/)) {
    const trimmed = line.trim()
    if (!trimmed || trimmed.startsWith('#') || !trimmed.includes('=')) {
      continue
    }
    const key = trimmed.slice(0, trimmed.indexOf('=')).trim()
    if (!/^[A-Za-z_][A-Za-z0-9_]*$/.test(key) || process.env[key]) {
      continue
    }
    let value = trimmed.slice(trimmed.indexOf('=') + 1).trim()
    if (
      (value.startsWith('"') && value.endsWith('"')) ||
      (value.startsWith("'") && value.endsWith("'"))
    ) {
      value = value.slice(1, -1)
    }
    process.env[key] = value
  }
}

function envValue(names, fallback = '') {
  for (const name of names) {
    const value = String(process.env[name] ?? '').trim()
    if (value) {
      return value
    }
  }
  return fallback
}

function boolEnv(name, fallback = false) {
  const value = String(process.env[name] ?? '').trim().toLowerCase()
  if (!value) {
    return fallback
  }
  return ['1', 'true', 'yes', 'on'].includes(value)
}

function intEnv(name, fallback) {
  const value = String(process.env[name] ?? '').trim()
  if (!value) {
    return fallback
  }
  const parsed = Number.parseInt(value, 10)
  if (!Number.isFinite(parsed) || parsed < 1) {
    fail(`${name} must be a positive integer`)
  }
  return parsed
}

function workspaceVersion() {
  const cargoToml = readFileSync(join(repoRoot, 'Cargo.toml'), 'utf8')
  const workspace = cargoToml.match(/\[workspace\.package\][\s\S]*?version\s*=\s*"([^"]+)"/)
  return workspace?.[1] ?? '0.1.0'
}

function semanticVersionCode(version) {
  const match = version.match(/^(\d+)\.(\d+)\.(\d+)/)
  if (!match) {
    fail(`Version is not semver-shaped: ${version}`)
  }
  const [, major, minor, patch] = match
  return String(Number(major) * 1_000_000 + Number(minor) * 1_000 + Number(patch))
}

function resolveAscAuth() {
  const ascRoot = envValue(['IRIS_DRIVE_ASC_ROOT'], join(os.homedir(), '.appstoreconnect'))
  let keyPath = envValue(['IRIS_DRIVE_ASC_AUTH_KEY_PATH', 'IRIS_DRIVE_ASC_KEY_PATH'])
  if (!keyPath) {
    const keyDir = join(ascRoot, 'private_keys')
    if (existsSync(keyDir)) {
      keyPath = readdirSync(keyDir)
        .filter((entry) => /^AuthKey_.*\.p8$/.test(entry))
        .sort()
        .map((entry) => join(keyDir, entry))[0] ?? ''
    }
  }
  let keyId = envValue(['IRIS_DRIVE_ASC_AUTH_KEY_ID', 'IRIS_DRIVE_ASC_KEY_ID'])
  if (!keyId && keyPath) {
    keyId = basename(keyPath).replace(/^AuthKey_/, '').replace(/\.p8$/, '')
  }
  let issuerId = envValue([
    'IRIS_DRIVE_ASC_AUTH_KEY_ISSUER_ID',
    'IRIS_DRIVE_ASC_ISSUER_ID',
  ])
  const issuerPath = join(ascRoot, 'issuer.txt')
  if (!issuerId && existsSync(issuerPath)) {
    issuerId = readFileSync(issuerPath, 'utf8').trim()
  }
  if (!keyPath) {
    fail('IRIS_DRIVE_ASC_AUTH_KEY_PATH is required for App Store Connect API calls', 2)
  }
  if (!keyId) {
    fail('IRIS_DRIVE_ASC_AUTH_KEY_ID is required for App Store Connect API calls', 2)
  }
  if (!issuerId) {
    fail('IRIS_DRIVE_ASC_AUTH_KEY_ISSUER_ID is required for App Store Connect API calls', 2)
  }
  if (!existsSync(keyPath)) {
    fail(`ASC API key file not found: ${keyPath}`)
  }
  return { keyPath, keyId, issuerId }
}

function base64url(input) {
  return Buffer.from(input).toString('base64url')
}

function createToken({ keyPath, keyId, issuerId }) {
  const now = Math.floor(Date.now() / 1000)
  const header = base64url(JSON.stringify({ alg: 'ES256', kid: keyId, typ: 'JWT' }))
  const payload = base64url(JSON.stringify({
    iss: issuerId,
    iat: now,
    exp: now + 20 * 60,
    aud: 'appstoreconnect-v1',
  }))
  const signingInput = `${header}.${payload}`
  const signature = cryptoSign('sha256', Buffer.from(signingInput), {
    key: createPrivateKey(readFileSync(keyPath)),
    dsaEncoding: 'ieee-p1363',
  })
  return `${signingInput}.${base64url(signature)}`
}

async function request(method, pathOrUrl, params = {}, body = undefined) {
  const url = pathOrUrl.startsWith('https://') ? new URL(pathOrUrl) : new URL(pathOrUrl, baseUrl)
  for (const [key, value] of Object.entries(params || {})) {
    if (value !== undefined && value !== null && value !== '') {
      url.searchParams.set(key, String(value))
    }
  }
  const response = await fetch(url, {
    method,
    headers: {
      Authorization: `Bearer ${authToken}`,
      'Content-Type': 'application/json',
    },
    body: body === undefined ? undefined : JSON.stringify(body),
  })
  const text = await response.text()
  const parsed = text ? JSON.parse(text) : {}
  return [response.status, parsed]
}

function requireOk(status, body, operation) {
  if (status >= 200 && status < 300) {
    return body
  }
  fail(`${operation} failed: HTTP ${status}\n${JSON.stringify(body, null, 2)}`)
}

async function getAll(path, params = {}) {
  let [status, body] = await request('GET', path, { ...params, limit: params.limit ?? '200' })
  body = requireOk(status, body, `GET ${path}`)
  const data = [...(body.data ?? [])]
  let nextUrl = body.links?.next
  while (nextUrl) {
    ;[status, body] = await request('GET', nextUrl)
    body = requireOk(status, body, `GET ${nextUrl}`)
    data.push(...(body.data ?? []))
    nextUrl = body.links?.next
  }
  return data
}

async function findApp() {
  const apps = await getAll('apps', { 'filter[bundleId]': bundleId, limit: '10' })
  return apps.find((app) => app.attributes?.bundleId === bundleId) ?? apps[0] ?? null
}

async function exactBundleId() {
  const bundleIds = await getAll('bundleIds', { 'filter[identifier]': bundleId, limit: '200' })
  const exact = bundleIds.find((candidate) => candidate.attributes?.identifier === bundleId)
  if (!exact) {
    fail(`No Apple Developer bundle ID found for ${bundleId}`)
  }
  return exact
}

async function ensureApp() {
  const existing = await findApp()
  if (existing) {
    console.log(`App Store Connect app exists: ${existing.attributes?.name ?? appName} [${existing.id}]`)
    return existing
  }

  const developerBundleId = await exactBundleId()
  const body = {
    data: {
      type: 'apps',
      attributes: {
        name: appName,
        primaryLocale: appPrimaryLocale,
        sku: appSku,
        platform: 'IOS',
      },
      relationships: {
        bundleId: {
          data: {
            type: 'bundleIds',
            id: developerBundleId.id,
          },
        },
      },
    },
  }
  const [status, response] = await request('POST', 'apps', {}, body)
  if (status === 409) {
    const app = await findApp()
    if (app) {
      console.log(`App Store Connect app exists: ${app.attributes?.name ?? appName} [${app.id}]`)
      return app
    }
  }
  const app = requireOk(status, response, 'Create App Store Connect app').data
  console.log(`Created App Store Connect app: ${appName} [${app.id}]`)
  return app
}

async function findBuild(appId) {
  const builds = await getAll('builds', {
    'filter[app]': appId,
    'filter[version]': buildNumber,
    limit: '10',
  })
  return builds.find((build) => String(build.attributes?.version) === buildNumber) ?? builds[0] ?? null
}

async function requireVisibleBuild(appId) {
  const build = await findBuild(appId)
  if (!build) {
    fail(`No TestFlight build ${buildNumber} found for ${bundleId}`)
  }
  return build
}

async function getBuild(buildId) {
  const [status, body] = await request('GET', `builds/${buildId}`, {
    'fields[builds]': [
      'version',
      'uploadedDate',
      'processingState',
      'buildAudienceType',
      'usesNonExemptEncryption',
      'expired',
      'expirationDate',
    ].join(','),
  })
  return requireOk(status, body, `GET builds/${buildId}`).data
}

async function waitForValidBuild(appId) {
  for (let attempt = 1; attempt <= waitAttempts; attempt += 1) {
    const build = await findBuild(appId)
    if (build) {
      const state = build.attributes?.processingState
      console.log(`Found TestFlight build ${buildNumber}: ${state} (${attempt}/${waitAttempts})`)
      if (state === 'VALID') {
        return build
      }
      if (['FAILED', 'INVALID'].includes(state)) {
        fail(`Build ${buildNumber} is ${state}`)
      }
    } else {
      console.log(`Waiting for TestFlight build ${buildNumber} (${attempt}/${waitAttempts})`)
    }
    if (attempt < waitAttempts) {
      await sleep(waitSeconds * 1000)
    }
  }
  fail(`Build ${buildNumber} did not become VALID in time`)
}

async function buildBetaDetail(build) {
  const [status, body] = await request('GET', `builds/${build.id}/buildBetaDetail`)
  return requireOk(status, body, 'Read build beta detail').data
}

async function waitForInternalTestableBuild(build) {
  for (let attempt = 1; attempt <= waitAttempts; attempt += 1) {
    const detail = await buildBetaDetail(build)
    const state = detail.attributes?.internalBuildState
    console.log(`Internal TestFlight state for build ${buildNumber}: ${state} (${attempt}/${waitAttempts})`)
    if (['READY_FOR_BETA_TESTING', 'IN_BETA_TESTING'].includes(state)) {
      return build
    }
    if (['PROCESSING_EXCEPTION', 'EXPIRED', 'MISSING_EXPORT_COMPLIANCE'].includes(state)) {
      fail(`Build ${buildNumber} is not internally testable: ${state}`)
    }
    if (attempt < waitAttempts) {
      await sleep(waitSeconds * 1000)
    }
  }
  fail(`Build ${buildNumber} did not become internally testable in time`)
}

async function ensureExportCompliance(build) {
  build = await getBuild(build.id)
  const current = build.attributes?.usesNonExemptEncryption
  if (current === usesNonExemptEncryption) {
    console.log(`Build ${buildNumber} export compliance is already ${current}`)
    return build
  }
  if (usesNonExemptEncryption && !encryptionDeclarationId) {
    fail('IRIS_DRIVE_TESTFLIGHT_APP_ENCRYPTION_DECLARATION_ID is required for non-exempt encryption')
  }
  const data = {
    type: 'builds',
    id: build.id,
    attributes: { usesNonExemptEncryption },
  }
  if (encryptionDeclarationId) {
    data.relationships = {
      appEncryptionDeclaration: {
        data: { type: 'appEncryptionDeclarations', id: encryptionDeclarationId },
      },
    }
  }
  const [status, body] = await request('PATCH', `builds/${build.id}`, {}, { data })
  requireOk(status, body, 'Set build export compliance')
  console.log(`Set build ${buildNumber} export compliance: ${usesNonExemptEncryption}`)
  return getBuild(build.id)
}

async function groups(appId) {
  return getAll('betaGroups', {
    'filter[app]': appId,
    'fields[betaGroups]': [
      'name',
      'createdDate',
      'isInternalGroup',
      'hasAccessToAllBuilds',
      'publicLinkEnabled',
      'publicLinkId',
      'publicLinkLimitEnabled',
      'publicLinkLimit',
      'publicLink',
      'feedbackEnabled',
    ].join(','),
    limit: '200',
  })
}

async function createInternalGroup(appId) {
  const name = envValue(['IRIS_DRIVE_TESTFLIGHT_DEFAULT_GROUP'], 'Internal Testers')
  const body = {
    data: {
      type: 'betaGroups',
      attributes: { name, isInternalGroup: true },
      relationships: { app: { data: { type: 'apps', id: appId } } },
    },
  }
  const [status, response] = await request('POST', 'betaGroups', {}, body)
  const group = requireOk(status, response, 'Create internal TestFlight group').data
  console.log(`Created internal TestFlight group: ${name}`)
  return group
}

async function createExternalGroup(appId) {
  const name = envValue(['IRIS_DRIVE_TESTFLIGHT_PUBLIC_GROUP_NAME'], defaultPublicGroupName)
  const attrs = {
    name,
    isInternalGroup: false,
    hasAccessToAllBuilds: false,
    feedbackEnabled: boolEnv('IRIS_DRIVE_TESTFLIGHT_FEEDBACK_ENABLED', true),
  }
  if (boolEnv('IRIS_DRIVE_TESTFLIGHT_PUBLIC_LINK_ENABLED', true)) {
    attrs.publicLinkEnabled = true
    attrs.publicLinkLimitEnabled = boolEnv('IRIS_DRIVE_TESTFLIGHT_PUBLIC_LINK_LIMIT_ENABLED', true)
    if (attrs.publicLinkLimitEnabled) {
      attrs.publicLinkLimit = intEnv('IRIS_DRIVE_TESTFLIGHT_PUBLIC_LINK_LIMIT', defaultPublicLinkLimit)
    }
  }
  const [status, response] = await request('POST', 'betaGroups', {}, {
    data: {
      type: 'betaGroups',
      attributes: attrs,
      relationships: { app: { data: { type: 'apps', id: appId } } },
    },
  })
  const group = requireOk(status, response, 'Create external TestFlight group').data
  console.log(`Created external TestFlight group: ${name}`)
  return group
}

async function selectInternalGroups(appId) {
  const wantedRaw = envValue(['IRIS_DRIVE_TESTFLIGHT_INTERNAL_GROUPS', 'IRIS_DRIVE_TESTFLIGHT_GROUPS'])
  const allGroups = await groups(appId)
  let selected
  if (wantedRaw) {
    const wanted = new Set(splitCsv(wantedRaw))
    selected = allGroups.filter((group) => wanted.has(group.id) || wanted.has(group.attributes?.name))
    const matched = new Set(selected.flatMap((group) => [group.id, group.attributes?.name].filter(Boolean)))
    const missing = [...wanted].filter((item) => !matched.has(item))
    if (missing.length > 0) {
      fail(`TestFlight group not found: ${missing.join(', ')}`)
    }
  } else {
    selected = allGroups.filter((group) => group.attributes?.isInternalGroup === true)
    if (selected.length === 0) {
      selected = [await createInternalGroup(appId)]
    }
  }
  const external = selected.filter((group) => group.attributes?.isInternalGroup !== true)
  if (external.length > 0 && !boolEnv('IRIS_DRIVE_TESTFLIGHT_ALLOW_EXTERNAL', false)) {
    fail(`Refusing to attach external TestFlight group(s): ${external.map(groupName).join(', ')}`)
  }
  if (selected.length === 0) {
    fail('No internal TestFlight groups found')
  }
  return selected
}

async function selectExternalGroups(appId, { createMissing }) {
  const wantedRaw = envValue(['IRIS_DRIVE_TESTFLIGHT_PUBLIC_GROUPS'])
  const allGroups = await groups(appId)
  let selected
  if (wantedRaw) {
    const wanted = new Set(splitCsv(wantedRaw))
    selected = allGroups.filter((group) => wanted.has(group.id) || wanted.has(group.attributes?.name))
    const matched = new Set(selected.flatMap((group) => [group.id, group.attributes?.name].filter(Boolean)))
    const missing = [...wanted].filter((item) => !matched.has(item))
    if (missing.length > 0) {
      fail(`TestFlight group not found: ${missing.join(', ')}`)
    }
  } else {
    selected = allGroups.filter((group) => group.attributes?.isInternalGroup !== true)
    if (selected.length === 0 && createMissing) {
      selected = [await createExternalGroup(appId)]
    }
  }
  const internal = selected.filter((group) => group.attributes?.isInternalGroup === true)
  if (internal.length > 0) {
    fail('Refusing to attach internal group(s) in public TestFlight mode')
  }
  if (selected.length === 0) {
    fail('No external TestFlight groups found')
  }
  return selected
}

async function ensurePublicGroupSettings(selectedGroups) {
  if (!boolEnv('IRIS_DRIVE_TESTFLIGHT_PUBLIC_LINK_ENABLED', true)) {
    return selectedGroups
  }
  const updated = []
  for (const group of selectedGroups) {
    const attrs = {
      feedbackEnabled: boolEnv('IRIS_DRIVE_TESTFLIGHT_FEEDBACK_ENABLED', true),
      publicLinkEnabled: true,
      publicLinkLimitEnabled: boolEnv('IRIS_DRIVE_TESTFLIGHT_PUBLIC_LINK_LIMIT_ENABLED', true),
    }
    if (attrs.publicLinkLimitEnabled) {
      attrs.publicLinkLimit =
        intEnv('IRIS_DRIVE_TESTFLIGHT_PUBLIC_LINK_LIMIT', group.attributes?.publicLinkLimit || defaultPublicLinkLimit)
    }
    const [status, body] = await request('PATCH', `betaGroups/${group.id}`, {}, {
      data: { type: 'betaGroups', id: group.id, attributes: attrs },
    })
    updated.push(status >= 200 && status < 300 ? body.data : group)
  }
  return updated
}

async function groupHasBuild(groupId, build) {
  const builds = await getAll(`betaGroups/${groupId}/builds`, { limit: '200' })
  return builds.some((item) => item.id === build.id)
}

async function groupTesters(groupId) {
  return getAll(`betaGroups/${groupId}/betaTesters`, {
    'fields[betaTesters]': 'firstName,lastName,email,inviteType,state',
    limit: '200',
  })
}

async function attachBuild(build, selectedGroups, label) {
  const body = { data: selectedGroups.map((group) => ({ type: 'betaGroups', id: group.id })) }
  const [status, responseBody] = await request(
    'POST',
    `builds/${build.id}/relationships/betaGroups`,
    {},
    body,
  )
  if (!(status >= 200 && status < 300) && status !== 409) {
    fail(`Attach failed: HTTP ${status}\n${JSON.stringify(responseBody, null, 2)}`)
  }
  const failures = []
  for (const group of selectedGroups) {
    if (!(await groupHasBuild(group.id, build))) {
      failures.push(groupName(group))
    }
  }
  if (failures.length > 0) {
    fail(`Build was not found in group(s) after attach: ${failures.join(', ')}`)
  }
  console.log(`Attached build ${buildNumber} to ${label} TestFlight group(s):`)
  for (const group of selectedGroups) {
    console.log(`  - ${groupName(group)}`)
  }
}

async function ensurePublicBuild(build) {
  build = await getBuild(build.id)
  const state = build.attributes?.processingState
  if (state !== 'VALID') {
    fail(`Build ${buildNumber} is ${state}, not VALID`)
  }
  if (build.attributes?.buildAudienceType === 'INTERNAL_ONLY') {
    fail('Build is INTERNAL_ONLY. Upload with IRIS_DRIVE_IOS_INTERNAL_ONLY=false first.')
  }
  return build
}

async function betaReviewDetail(appId) {
  const [status, body] = await request('GET', `apps/${appId}/betaAppReviewDetail`)
  return requireOk(status, body, 'Read beta app review detail').data
}

async function betaAppLocalizations(appId) {
  return getAll(`apps/${appId}/betaAppLocalizations`, { limit: '200' })
}

function requiredMetadataValue(name, current, label, fallback = '') {
  const value = envValue([name], String(current ?? '').trim() || fallback)
  if (!value) {
    fail(`Missing TestFlight review metadata: ${label} (${name})`)
  }
  return value
}

async function ensurePublicMetadata(appId) {
  const detail = await betaReviewDetail(appId)
  const attrs = detail.attributes ?? {}
  const [status, body] = await request('PATCH', `betaAppReviewDetails/${detail.id}`, {}, {
    data: {
      type: 'betaAppReviewDetails',
      id: detail.id,
      attributes: {
        contactFirstName: requiredMetadataValue(
          'IRIS_DRIVE_TESTFLIGHT_CONTACT_FIRST_NAME',
          attrs.contactFirstName,
          'contact first name',
        ),
        contactLastName: requiredMetadataValue(
          'IRIS_DRIVE_TESTFLIGHT_CONTACT_LAST_NAME',
          attrs.contactLastName,
          'contact last name',
        ),
        contactPhone: requiredMetadataValue(
          'IRIS_DRIVE_TESTFLIGHT_CONTACT_PHONE',
          attrs.contactPhone,
          'contact phone',
        ),
        contactEmail: requiredMetadataValue(
          'IRIS_DRIVE_TESTFLIGHT_CONTACT_EMAIL',
          attrs.contactEmail,
          'contact email',
        ),
        demoAccountRequired: false,
        notes: envValue(['IRIS_DRIVE_TESTFLIGHT_REVIEW_NOTES'], attrs.notes || defaultReviewNotes),
      },
    },
  })
  requireOk(status, body, 'Update beta app review detail')
  await upsertBetaAppLocalization(appId)
  console.log('Updated Beta App Review metadata')
}

async function upsertBetaAppLocalization(appId) {
  const locale = envValue(['IRIS_DRIVE_TESTFLIGHT_LOCALE'], 'en-US')
  const existing =
    (await betaAppLocalizations(appId)).find((item) => item.attributes?.locale === locale) ?? null
  const attrs = existing?.attributes ?? {}
  const localizationAttrs = {
    description: envValue(['IRIS_DRIVE_TESTFLIGHT_BETA_DESCRIPTION'], attrs.description || defaultBetaDescription),
    feedbackEmail: requiredMetadataValue(
      'IRIS_DRIVE_TESTFLIGHT_FEEDBACK_EMAIL',
      attrs.feedbackEmail,
      'feedback email',
    ),
    privacyPolicyUrl: requiredMetadataValue(
      'IRIS_DRIVE_TESTFLIGHT_PRIVACY_POLICY_URL',
      attrs.privacyPolicyUrl,
      'privacy policy URL',
    ),
  }
  const marketingUrl = envValue(['IRIS_DRIVE_TESTFLIGHT_MARKETING_URL'])
  if (marketingUrl) {
    localizationAttrs.marketingUrl = marketingUrl
  }
  if (existing) {
    const [status, body] = await request('PATCH', `betaAppLocalizations/${existing.id}`, {}, {
      data: { type: 'betaAppLocalizations', id: existing.id, attributes: localizationAttrs },
    })
    requireOk(status, body, 'Update beta app localization')
  } else {
    localizationAttrs.locale = locale
    const [status, body] = await request('POST', 'betaAppLocalizations', {}, {
      data: {
        type: 'betaAppLocalizations',
        attributes: localizationAttrs,
        relationships: { app: { data: { type: 'apps', id: appId } } },
      },
    })
    requireOk(status, body, 'Create beta app localization')
  }
}

async function betaBuildLocalizations(build) {
  return getAll(`builds/${build.id}/betaBuildLocalizations`, { limit: '200' })
}

async function upsertWhatToTest(build) {
  const locale = envValue(['IRIS_DRIVE_TESTFLIGHT_LOCALE'], 'en-US')
  const whatsNew = envValue(['IRIS_DRIVE_TESTFLIGHT_WHATS_NEW'], defaultWhatsNew)
  const existing =
    (await betaBuildLocalizations(build)).find((item) => item.attributes?.locale === locale) ?? null
  if (existing) {
    const [status, body] = await request('PATCH', `betaBuildLocalizations/${existing.id}`, {}, {
      data: { type: 'betaBuildLocalizations', id: existing.id, attributes: { whatsNew } },
    })
    requireOk(status, body, 'Update What to Test')
  } else {
    const [status, body] = await request('POST', 'betaBuildLocalizations', {}, {
      data: {
        type: 'betaBuildLocalizations',
        attributes: { locale, whatsNew },
        relationships: { build: { data: { type: 'builds', id: build.id } } },
      },
    })
    requireOk(status, body, 'Create What to Test')
  }
  console.log(`Updated What to Test for ${locale}`)
}

async function betaReviewState(build) {
  const [status, body] = await request('GET', `builds/${build.id}/betaAppReviewSubmission`)
  if (status === 404 || !body.data) {
    return 'not submitted'
  }
  if (!(status >= 200 && status < 300)) {
    return `unknown (HTTP ${status})`
  }
  return body.data.attributes?.betaReviewState || body.data.attributes?.state || 'submitted'
}

async function submitBetaReview(build) {
  const state = await betaReviewState(build)
  if (!['not submitted', 'REJECTED'].includes(state)) {
    console.log(`Beta App Review already submitted: ${state}`)
    return
  }
  const [status, body] = await request('POST', 'betaAppReviewSubmissions', {}, {
    data: {
      type: 'betaAppReviewSubmissions',
      relationships: { build: { data: { type: 'builds', id: build.id } } },
    },
  })
  if (status === 409) {
    console.log('Beta App Review submission already exists')
    return
  }
  requireOk(status, body, 'Submit Beta App Review')
  console.log(`Submitted build ${buildNumber} for Beta App Review`)
}

async function printGroups(appId) {
  console.log(`TestFlight groups for ${bundleId}:`)
  for (const group of await groups(appId)) {
    const attrs = group.attributes ?? {}
    const kind = attrs.isInternalGroup ? 'internal' : 'external'
    const testers = await groupTesters(group.id)
    const publicLink = kind === 'external' && attrs.publicLink ? `, ${attrs.publicLink}` : ''
    console.log(`[${kind}] ${groupName(group)} ${group.id}: ${testers.length} tester(s)${publicLink}`)
  }
}

async function summarizeInternalBuild(appId, build, selectedGroups = null) {
  build = await getBuild(build.id)
  const detail = await buildBetaDetail(build)
  console.log(`Iris Drive ${versionName} (${buildNumber})`)
  console.log(`Build: ${build.id}`)
  console.log(`State: ${build.attributes?.processingState}`)
  console.log(`Audience: ${build.attributes?.buildAudienceType}`)
  console.log(`Uses Non-Exempt Encryption: ${build.attributes?.usesNonExemptEncryption}`)
  console.log(`Internal Beta State: ${detail.attributes?.internalBuildState}`)
  console.log(`Uploaded: ${build.attributes?.uploadedDate}`)
  for (const group of selectedGroups ?? await selectInternalGroups(appId)) {
    const attached = await groupHasBuild(group.id, build)
    const testers = await groupTesters(group.id)
    console.log(`  - ${groupName(group)}: ${attached ? 'attached' : 'not attached'}, ${testers.length} tester(s)`)
  }
}

async function summarizePublicBuild(build, selectedGroups) {
  build = await getBuild(build.id)
  const detail = await buildBetaDetail(build)
  console.log(`Iris Drive ${versionName} (${buildNumber})`)
  console.log(`Build: ${build.id}`)
  console.log(`State: ${build.attributes?.processingState}`)
  console.log(`Audience: ${build.attributes?.buildAudienceType}`)
  console.log(`Uses Non-Exempt Encryption: ${build.attributes?.usesNonExemptEncryption}`)
  console.log(`External Beta State: ${detail.attributes?.externalBuildState}`)
  console.log(`Beta Review: ${await betaReviewState(build)}`)
  console.log(`Uploaded: ${build.attributes?.uploadedDate}`)
  for (const group of selectedGroups) {
    const attached = await groupHasBuild(group.id, build)
    const testers = await groupTesters(group.id)
    const link = group.attributes?.publicLink ? `, ${group.attributes.publicLink}` : ''
    console.log(`  - ${groupName(group)}: ${attached ? 'attached' : 'not attached'}, ${testers.length} tester(s)${link}`)
  }
}

function groupName(group) {
  return group.attributes?.name || group.id
}

function splitCsv(value) {
  return String(value)
    .split(',')
    .map((item) => item.trim())
    .filter(Boolean)
}

function sleep(ms) {
  return new Promise((resolveSleep) => setTimeout(resolveSleep, ms))
}

function fail(message, code = 1) {
  console.error(message)
  process.exit(code)
}
