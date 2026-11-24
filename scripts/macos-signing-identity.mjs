// macOS release signing uses exactly the identity named in the environment, and
// only when it belongs to the team committed in release-policy.json. Picking
// whichever Developer ID certificate the keychain lists first signed releases
// with the wrong team.

const TEAM = /^[A-Z0-9]{10}$/
const IDENTITY = /^\s*\d+\)\s+([0-9A-F]{40})\s+"([^"]+)"\s*$/

export function selectMacosSigningIdentity({ requested, policyTeam, identities = '', dryRun = false }) {
  const team = String(policyTeam ?? '').trim()
  if (!TEAM.test(team)) {
    throw new Error('Set release-policy.json macos.signingTeam to the 10-character Apple team ID')
  }
  const wanted = String(requested ?? '').trim()
  // Dry runs never read the keychain; they plan with the identity as given.
  if (dryRun) return { hash: '', name: wanted || `Developer ID Application: (${team})`, team }
  if (!wanted) {
    throw new Error('Set IRIS_DRIVE_MACOS_SIGN_IDENTITY to the SHA-1 hash or full name of the Developer ID Application identity')
  }
  const matches = String(identities).split('\n').map(line => line.match(IDENTITY)).filter(Boolean)
    .map(([, hash, name]) => ({ hash, name }))
    .filter(({ hash, name }) => hash === wanted.toUpperCase() || name === wanted)
  if (matches.length !== 1) {
    throw new Error(`IRIS_DRIVE_MACOS_SIGN_IDENTITY is not a valid code signing identity in this keychain: ${wanted}`)
  }
  const [{ hash, name }] = matches
  if (!name.startsWith('Developer ID Application: ')) {
    throw new Error(`macOS releases must be signed with a Developer ID Application identity, not ${name}`)
  }
  const signer = name.match(/\(([A-Z0-9]{10})\)$/)?.[1] ?? 'unknown'
  if (signer !== team) {
    throw new Error(`${name} signs for team ${signer}; release-policy.json requires ${team}`)
  }
  return { hash, name, team }
}

export function assertSignedTeam(codesignDetails, policyTeam, label) {
  const signed = String(codesignDetails).match(/^TeamIdentifier=(.+)$/m)?.[1]?.trim() ?? 'missing'
  if (signed !== policyTeam) {
    throw new Error(`${label} is signed by team ${signed}; release-policy.json requires ${policyTeam}`)
  }
}
