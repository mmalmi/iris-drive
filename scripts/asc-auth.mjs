// App Store Connect API key for release scripts. A Mac can hold keys for more
// than one team in ~/.appstoreconnect, so the key must be named in settings.
// Picking the first key there once sent uploads to the wrong team.
import { basename } from 'node:path'

function named(env, names) {
  return names.map(name => String(env[name] ?? '').trim()).find(Boolean) ?? ''
}

export function ascAuth(env = process.env) {
  const keyPath = named(env, ['IRIS_DRIVE_ASC_AUTH_KEY_PATH', 'IRIS_DRIVE_ASC_KEY_PATH'])
  const fileKeyId = /^AuthKey_(.+)\.p8$/.exec(basename(keyPath))?.[1] ?? ''
  const keyId = named(env, ['IRIS_DRIVE_ASC_AUTH_KEY_ID', 'IRIS_DRIVE_ASC_KEY_ID']) || fileKeyId
  const issuerId = named(env, ['IRIS_DRIVE_ASC_AUTH_KEY_ISSUER_ID', 'IRIS_DRIVE_ASC_ISSUER_ID'])
  const problems = [
    ...(keyPath ? [] : ['IRIS_DRIVE_ASC_AUTH_KEY_PATH is required']),
    ...(keyId ? [] : ['IRIS_DRIVE_ASC_AUTH_KEY_ID is required']),
    ...(issuerId ? [] : ['IRIS_DRIVE_ASC_AUTH_KEY_ISSUER_ID is required']),
    ...(fileKeyId && keyId !== fileKeyId
      ? [`App Store Connect key ID ${keyId} does not match ${basename(keyPath)}`] : []),
  ]
  return { keyPath, keyId, issuerId, problems }
}
