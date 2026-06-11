export const macosRestrictedProfileEntitlementKeys = ['com.apple.developer.associated-domains']

export function expandTeamIdentifier(value, teamId) {
  if (typeof value === 'string') {
    return teamId ? value.replaceAll('$(TeamIdentifierPrefix)', `${teamId}.`) : value
  }
  if (Array.isArray(value)) {
    return value.map((item) => expandTeamIdentifier(item, teamId))
  }
  if (value && typeof value === 'object') {
    return Object.fromEntries(
      Object.entries(value).map(([key, item]) => [key, expandTeamIdentifier(item, teamId)]),
    )
  }
  return value
}

function profileAuthorizesAssociatedDomains(requested, allowed) {
  if (!Array.isArray(requested) || !Array.isArray(allowed)) {
    return false
  }
  return requested.every((entry) => allowed.includes(entry))
}

export function profileAuthorizesEntitlement(key, requested, profileEntitlements = {}) {
  const allowed = profileEntitlements?.[key]
  if (key === 'com.apple.developer.associated-domains') {
    return profileAuthorizesAssociatedDomains(requested, allowed)
  }
  if (allowed === '*') {
    return true
  }
  if (Array.isArray(requested) && Array.isArray(allowed)) {
    return requested.every((entry) => allowed.includes(entry))
  }
  return JSON.stringify(requested) === JSON.stringify(allowed)
}

export function prepareMacosEntitlementsData({
  sourceEntitlements,
  teamId,
  keepProvisionedEntitlements = false,
  profileEntitlements = {},
}) {
  const entitlements = expandTeamIdentifier(sourceEntitlements ?? {}, teamId)
  const dropped = []
  for (const key of macosRestrictedProfileEntitlementKeys) {
    if (!(key in entitlements)) {
      continue
    }
    const shouldKeep =
      keepProvisionedEntitlements &&
      profileAuthorizesEntitlement(key, entitlements[key], profileEntitlements)
    if (!shouldKeep) {
      delete entitlements[key]
      dropped.push(key)
    }
  }
  return { entitlements, dropped }
}
