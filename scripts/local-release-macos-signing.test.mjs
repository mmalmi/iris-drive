import test from 'node:test'
import assert from 'node:assert/strict'
import { readFileSync } from 'node:fs'

import { assertSignedTeam, selectMacosSigningIdentity } from './macos-signing-identity.mjs'

const identities = `  1) 1111111111111111111111111111111111111111 "Apple Development: Someone (KQSLL8VYCE)"
  2) 2222222222222222222222222222222222222222 "Developer ID Application: Martti Malmi (J8PPJKD7TA)"
  3) 021AAE8875AEC52B083AF6EF0A4B591CA1837AE0 "Developer ID Application: Sirius Business Oy (8G9P8AN75D)"
     3 valid identities found
`

test('project policy names the macOS signing team', () => {
  const policy = JSON.parse(readFileSync(new URL('../release-policy.json', import.meta.url), 'utf8'))
  assert.match(policy.macos.signingTeam, /^[A-Z0-9]{10}$/)
})

test('macOS signing never picks an identity on its own', () => {
  assert.throws(
    () => selectMacosSigningIdentity({ requested: '', policyTeam: '8G9P8AN75D', identities }),
    /IRIS_DRIVE_MACOS_SIGN_IDENTITY/,
  )
})

test('macOS signing selects the requested identity by hash or name', () => {
  assert.deepEqual(
    selectMacosSigningIdentity({
      requested: '021aae8875aec52b083af6ef0a4b591ca1837ae0', policyTeam: '8G9P8AN75D', identities,
    }),
    {
      hash: '021AAE8875AEC52B083AF6EF0A4B591CA1837AE0',
      name: 'Developer ID Application: Sirius Business Oy (8G9P8AN75D)',
      team: '8G9P8AN75D',
    },
  )
  assert.equal(
    selectMacosSigningIdentity({
      requested: 'Developer ID Application: Martti Malmi (J8PPJKD7TA)', policyTeam: 'J8PPJKD7TA', identities,
    }).hash,
    '2222222222222222222222222222222222222222',
  )
})

test('macOS signing refuses an identity from another team than the policy', () => {
  assert.throws(
    () => selectMacosSigningIdentity({
      requested: '2222222222222222222222222222222222222222', policyTeam: '8G9P8AN75D', identities,
    }),
    /team J8PPJKD7TA.*release-policy\.json requires 8G9P8AN75D/,
  )
})

test('macOS signing refuses missing, ambiguous or non Developer ID identities', () => {
  assert.throws(
    () => selectMacosSigningIdentity({ requested: 'ABCDEF', policyTeam: '8G9P8AN75D', identities }),
    /not a valid code signing identity/,
  )
  assert.throws(
    () => selectMacosSigningIdentity({
      requested: '1111111111111111111111111111111111111111', policyTeam: 'KQSLL8VYCE', identities,
    }),
    /Developer ID Application/,
  )
  assert.throws(
    () => selectMacosSigningIdentity({ requested: 'Developer ID Application', policyTeam: '8G9P8AN75D', identities }),
    /not a valid code signing identity/,
  )
  assert.throws(
    () => selectMacosSigningIdentity({ requested: '021AAE88', policyTeam: 'nope', identities }),
    /release-policy\.json macos\.signingTeam/,
  )
})

test('dry runs describe the policy team without reading the keychain', () => {
  assert.equal(selectMacosSigningIdentity({ requested: '', policyTeam: '8G9P8AN75D', dryRun: true }).team, '8G9P8AN75D')
})

test('signed bundles must carry the policy team', () => {
  const signed = 'Identifier=to.iris.drive.macos\nTeamIdentifier=8G9P8AN75D\nSealed Resources version=2\n'
  assert.doesNotThrow(() => assertSignedTeam(signed, '8G9P8AN75D', 'Iris Drive.app'))
  assert.throws(() => assertSignedTeam(signed, 'J8PPJKD7TA', 'Iris Drive.app'), /Iris Drive\.app is signed by team 8G9P8AN75D/)
  assert.throws(() => assertSignedTeam('TeamIdentifier=not set\n', '8G9P8AN75D', 'idrive'), /idrive is signed by team not set/)
})
