import { readFileSync } from 'node:fs'

const editable = new Set(['PREPARE_FOR_SUBMISSION', 'READY_FOR_REVIEW', 'DEVELOPER_REJECTED', 'REJECTED', 'METADATA_REJECTED', 'INVALID_BINARY'])
const submitted = new Set(['WAITING_FOR_REVIEW', 'IN_REVIEW', 'COMPLETING'])
const released = new Set(['READY_FOR_DISTRIBUTION', 'READY_FOR_SALE'])
const versionState = version => version?.attributes?.appVersionState ?? version?.attributes?.appStoreState
const link = (type, id) => ({ data: { type, id } })

export function appStoreInputs(action, env = process.env) {
  for (const key of ['IRIS_DRIVE_IOS_MARKETING_VERSION', 'IRIS_DRIVE_IOS_BUILD_NUMBER']) {
    if (!env[key]?.trim()) throw new Error(`${key} is required for App Store actions`)
  }
  if (action === 'status') return null
  if (!env.IRIS_DRIVE_APP_STORE_NOTES_PATH?.trim()) throw new Error('IRIS_DRIVE_APP_STORE_NOTES_PATH is required (JSON locale-to-release-notes map)')
  const notes = JSON.parse(readFileSync(env.IRIS_DRIVE_APP_STORE_NOTES_PATH, 'utf8'))
  if (!notes || Array.isArray(notes) || typeof notes !== 'object' || !Object.keys(notes).length ||
      Object.values(notes).some(text => typeof text !== 'string' || !text.trim() || text.length > 4000)) {
    throw new Error('Release notes must map each store locale to nonempty text of at most 4000 characters')
  }
  return notes
}

// Reuse the TestFlight helper's authenticated client and exact build selector.
export async function runAppStore({ action, app, build, versionName, notes, request, requireOk, getAll, ensurePublicBuild }) {
  const call = async (method, path, data) => {
    const [status, body] = await request(method, path, {}, data === undefined ? undefined : { data })
    return requireOk(status, body, `${method} ${path}`).data
  }
  const versions = (await getAll(`apps/${app.id}/appStoreVersions`, { 'filter[platform]': 'IOS' }))
    .filter(v => v.attributes?.platform === 'IOS')
  const matches = versions.filter(v => v.attributes?.versionString === versionName)
  if (matches.length > 1) throw new Error('Multiple App Store versions match the requested version')
  let version = matches[0]
  const reviews = (await getAll(`apps/${app.id}/reviewSubmissions`)).filter(r =>
    (!r.attributes?.platform || r.attributes.platform === 'IOS') && r.attributes?.state !== 'COMPLETE')
  let review = null
  const report = () => console.log(JSON.stringify({
    version: versionName, build: build.attributes?.version, buildId: build.id,
    buildAudienceType: build.attributes?.buildAudienceType, processingState: build.attributes?.processingState,
    appStoreVersionId: version?.id ?? null, appStoreState: versionState(version) ?? null,
    reviewSubmissions: reviews.map(r => ({ id: r.id, state: r.attributes?.state })),
  }, null, 2))
  if (action === 'status') return report()
  build = await ensurePublicBuild(build)
  if (build.attributes?.buildAudienceType !== 'APP_STORE_ELIGIBLE') throw new Error('Build App Store eligibility is not confirmed')
  if (build.attributes?.expired) throw new Error('App Store build is expired')
  if (typeof build.attributes?.usesNonExemptEncryption !== 'boolean') throw new Error('Build export compliance must be completed before App Store preparation')
  let attached = version ? await call('GET', `appStoreVersions/${version.id}/relationships/build`) : null
  if (attached && attached.id !== build.id) throw new Error('App Store version already has a different build attached')
  for (const candidate of reviews) {
    const items = (await getAll(`reviewSubmissions/${candidate.id}/items`)).filter(i => i.attributes?.state !== 'REMOVED')
    if (items.some(i => !version || i.relationships?.appStoreVersion?.data?.id !== version.id)) {
      throw new Error('An active review submission contains unrelated items; resolve it in App Store Connect')
    }
    if (review) throw new Error('Multiple active review submissions; resolve them in App Store Connect')
    review = { ...candidate, items }
    if (candidate.attributes?.state !== 'READY_FOR_REVIEW' && !submitted.has(candidate.attributes?.state)) {
      throw new Error(`Review submission is ${candidate.attributes?.state}; resolve it in App Store Connect`)
    }
  }
  if (review && submitted.has(review.attributes.state)) {
    if (attached?.id !== build.id || !review.items.length) throw new Error('Submitted review does not confirm the exact attached build')
    return report()
  }
  if (version && !editable.has(versionState(version))) {
    if (attached?.id === build.id && (released.has(versionState(version)) ||
        ['WAITING_FOR_REVIEW', 'IN_REVIEW', 'PENDING_APPLE_RELEASE', 'PENDING_DEVELOPER_RELEASE', 'PROCESSING_FOR_DISTRIBUTION', 'PROCESSING_FOR_APP_STORE'].includes(versionState(version)))) return report()
    throw new Error(`App Store version is ${versionState(version)} and cannot be prepared`)
  }
  if (!version) {
    const live = versions.find(v => released.has(versionState(v)))
    if (!live) throw new Error('An existing released App Store version is required to inherit metadata')
    if (!['MANUAL', 'AFTER_APPROVAL'].includes(live.attributes.releaseType)) throw new Error('Current release schedule requires review in App Store Connect')
    // Apple transfers the current version's metadata. Preserve its release policy.
    version = await call('POST', 'appStoreVersions', {
      type: 'appStoreVersions', attributes: { platform: 'IOS', versionString: versionName, releaseType: live.attributes.releaseType },
      relationships: { app: link('apps', app.id) },
    })
  }
  const localizations = await getAll(`appStoreVersions/${version.id}/appStoreVersionLocalizations`)
  if (!localizations.length || Object.keys(notes).sort().join('\n') !== localizations.map(l => l.attributes.locale).sort().join('\n')) {
    throw new Error('Release notes must explicitly cover every existing App Store locale')
  }
  if (!version.attributes?.copyright?.trim()) throw new Error('Missing inherited App Store copyright')
  for (const locale of localizations) {
    for (const key of ['description', 'supportUrl']) {
      if (!locale.attributes?.[key]?.trim()) throw new Error(`Missing ${locale.attributes.locale} ${key} in App Store Connect`)
    }
  }
  const detail = await call('GET', `appStoreVersions/${version.id}/appStoreReviewDetail`)
  const attrs = detail?.attributes ?? {}
  const required = ['contactFirstName', 'contactLastName', 'contactPhone', 'contactEmail']
  if (attrs.demoAccountRequired) required.push('demoAccountName', 'demoAccountPassword')
  if (typeof attrs.demoAccountRequired !== 'boolean' || required.some(key => !attrs[key]?.trim())) {
    throw new Error('Missing inherited App Store review contact or demo account metadata')
  }
  for (const locale of localizations) {
    const whatsNew = notes[locale.attributes.locale]
    if (locale.attributes.whatsNew !== whatsNew) await call('PATCH', `appStoreVersionLocalizations/${locale.id}`, {
      type: 'appStoreVersionLocalizations', id: locale.id, attributes: { whatsNew },
    })
  }
  if (!attached) await call('PATCH', `appStoreVersions/${version.id}/relationships/build`, { type: 'builds', id: build.id })
  attached = await call('GET', `appStoreVersions/${version.id}/relationships/build`)
  if (attached?.id !== build.id) throw new Error('Exact build was not attached; rerun after checking App Store Connect')
  if (action === 'prepare') return report()
  if (!review) {
    review = await call('POST', 'reviewSubmissions', {
      type: 'reviewSubmissions', attributes: { platform: 'IOS' }, relationships: { app: link('apps', app.id) },
    })
    review.items = []
  }
  if (!review.items.length) await call('POST', 'reviewSubmissionItems', {
    type: 'reviewSubmissionItems', relationships: { reviewSubmission: link('reviewSubmissions', review.id), appStoreVersion: link('appStoreVersions', version.id) },
  })
  const items = (await getAll(`reviewSubmissions/${review.id}/items`)).filter(i => i.attributes?.state !== 'REMOVED')
  if (items.length !== 1 || items[0].relationships?.appStoreVersion?.data?.id !== version.id || items[0].attributes?.state !== 'READY_FOR_REVIEW') {
    throw new Error('Review must contain only the exact version, ready for review; submission stopped')
  }
  if ((await call('GET', `appStoreVersions/${version.id}/relationships/build`))?.id !== build.id) throw new Error('Exact attached build changed before submission')
  await call('PATCH', `reviewSubmissions/${review.id}`, { type: 'reviewSubmissions', id: review.id, attributes: { submitted: true } })
  review = await call('GET', `reviewSubmissions/${review.id}`)
  if (!submitted.has(review.attributes?.state)) throw new Error(`App Store submission not confirmed: ${review.attributes?.state}; inspect status before retrying`)
  console.log(JSON.stringify({ version: versionName, build: build.attributes.version, buildId: build.id, appStoreVersionId: version.id, reviewSubmissionId: review.id, state: review.attributes.state }, null, 2))
}
