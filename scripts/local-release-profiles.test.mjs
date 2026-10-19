import assert from 'node:assert/strict'
import { generateKeyPairSync } from 'node:crypto'
import { mkdtempSync, readFileSync, writeFileSync } from 'node:fs'
import { createServer } from 'node:http'
import { tmpdir } from 'node:os'
import { join } from 'node:path'
import { fileURLToPath } from 'node:url'
import { spawn } from 'node:child_process'
import test from 'node:test'

const script = fileURLToPath(new URL('./ios-profiles', import.meta.url))
for (const scenario of ['current-device-missing', 'certificate-missing', 'matching-profile']) {
  test(`provisioning profile selection: ${scenario}`, async (t) => {
    const home = mkdtempSync(join(tmpdir(), 'drive-profile-membership-'))
    const key = join(home, 'test.p8')
    const { privateKey } = generateKeyPairSync('ec', { namedCurve: 'P-256' })
    writeFileSync(key, privateKey.export({ type: 'pkcs8', format: 'pem' }))
    const identifiers = ['fi.siriusbusiness.drive', 'fi.siriusbusiness.drive.FileProvider',
      'fi.siriusbusiness.drive.ShareExtension']
    const created = []
    const json = (res, data) => { res.setHeader('content-type', 'application/json'); res.end(JSON.stringify({ data })) }
    const profile = (index, fresh = false) => ({
      type: 'profiles', id: `${fresh ? 'new' : 'old'}-${index}`,
      attributes: { name: `profile-${index}`, uuid: `${fresh ? 'new' : 'old'}-${index}`,
        profileState: 'ACTIVE', expirationDate: '2099-01-01T00:00:00Z',
        profileContent: Buffer.from('isolated test profile').toString('base64') },
      relationships: { bundleId: { data: { type: 'bundleIds', id: `bundle-${index}` } } },
    })
    const server = createServer(async (req, res) => {
      const url = new URL(req.url, 'http://localhost')
      if (req.method === 'GET' && url.pathname === '/v1/devices') {
        return json(res, [{ type: 'devices', id: 'current-device',
          attributes: { udid: 'CURRENT-UDID', status: 'ENABLED' } }])
      }
      if (req.method === 'GET' && url.pathname === '/v1/bundleIds') {
        const identifier = url.searchParams.get('filter[identifier]')
        return json(res, [{ type: 'bundleIds', id: `bundle-${identifiers.indexOf(identifier)}`,
          attributes: { identifier } }])
      }
      if (req.method === 'GET' && url.pathname === '/v1/profiles') {
        return json(res, identifiers.map((_, i) => profile(i)))
      }
      if (req.method === 'GET' && /^\/v1\/profiles\/old-\d\/devices$/.test(url.pathname)) {
        return json(res, [{ type: 'devices', id: scenario === 'current-device-missing' ? 'old-device' : 'current-device' }])
      }
      if (req.method === 'GET' && /^\/v1\/profiles\/old-\d\/certificates$/.test(url.pathname)) {
        return json(res, [{ type: 'certificates', id: scenario === 'certificate-missing' ? 'old-certificate' : 'current-certificate' }])
      }
      if (req.method === 'POST' && url.pathname === '/v1/profiles') {
        let body = ''; for await (const chunk of req) body += chunk
        const data = JSON.parse(body).data; created.push(data)
        return json(res, profile(Number(data.relationships.bundleId.data.id.split('-')[1]), true))
      }
      res.statusCode = 404; res.end(JSON.stringify({ errors: [{ title: `Unexpected ${req.method} ${url.pathname}` }] }))
    })
    await new Promise(resolve => server.listen(0, '127.0.0.1', resolve))
    t.after(() => server.close())
    const env = { ...process.env, HOME: home,
      IRIS_DRIVE_ASC_BASE_URL: `http://127.0.0.1:${server.address().port}/v1/`,
      IRIS_DRIVE_ASC_AUTH_KEY_PATH: key, IRIS_DRIVE_ASC_AUTH_KEY_ID: 'TEST',
      IRIS_DRIVE_ASC_AUTH_KEY_ISSUER_ID: '00000000-0000-0000-0000-000000000000',
      IRIS_DRIVE_ASC_CERTIFICATE_ID: 'current-certificate',
      IRIS_DRIVE_PROFILES_PLATFORM: 'ios', IRIS_DRIVE_IOS_PROFILE_TYPE: 'IOS_APP_DEVELOPMENT',
      IRIS_DRIVE_IOS_PROFILE_RECREATE: 'false', IRIS_DRIVE_IOS_DEVICE_UDIDS: 'CURRENT-UDID',
      IRIS_DRIVE_IOS_PROFILES_ENV_PATH: join(home, 'provisioning.env') }
    const result = await new Promise(resolve => {
      const child = spawn(process.execPath, [script, 'ensure'], { env })
      let output = ''; child.stdout.on('data', x => { output += x }); child.stderr.on('data', x => { output += x })
      child.on('close', code => resolve({ code, output }))
    })
    assert.equal(result.code, 0, result.output)
    assert.equal(created.length, scenario === 'matching-profile' ? 0 : 3)
    for (const item of created) {
      assert.deepEqual(item.relationships.devices.data, [{ type: 'devices', id: 'current-device' }])
      assert.deepEqual(item.relationships.certificates.data, [{ type: 'certificates', id: 'current-certificate' }])
    }
    assert.match(readFileSync(join(home, 'provisioning.env'), 'utf8'),
      scenario === 'matching-profile' ? /old-0/ : /new-0/)
  })
}
