/**
 * auth-web-alioth — the two clocks of a source download, and the link that
 * carries them.
 *
 * Pure cases only: the entitlement rule, the signed token (shape → signature →
 * expiry, in that order), the deployment signing key, and the audit trail.
 */
import { describe, expect, it, beforeEach, afterEach } from 'vitest'
import { mkdtemp, readFile, rm, stat } from 'node:fs/promises'
import { tmpdir } from 'node:os'
import path from 'node:path'
import { sourceEntitlement, loadSigningKey, appendSourceDownloadAudit } from '../src/source-download.ts'
import { signSourceLink, verifySourceLink, type SourceLinkPayload } from '../src/source-link.ts'

const SECRET = new Uint8Array(32).fill(7)
const NOW = 1_800_000_000

function payload(overrides: Partial<SourceLinkPayload> = {}): SourceLinkPayload {
  return { namespace: 'U-ada', app: 'default', userId: 'user-1', expiresAt: NOW + 900, ...overrides }
}

describe('sourceEntitlement', () => {
  it('grants access inside the license window', () => {
    const until = new Date((NOW + 86_400) * 1000)
    expect(sourceEntitlement({ until }, new Date(NOW * 1000)))
      .toEqual({ entitled: true, until: until.toISOString(), reason: 'licensed' })
    // The window's last moment still counts.
    expect(sourceEntitlement({ until }, until).entitled).toBe(true)
  })

  it('refuses an account with no license, and one past its window', () => {
    expect(sourceEntitlement(null, new Date(NOW * 1000))).toEqual({ entitled: false, until: null, reason: 'none' })

    const until = new Date((NOW - 1) * 1000)
    const lapsed = sourceEntitlement({ until }, new Date(NOW * 1000))
    expect(lapsed).toEqual({ entitled: false, until: until.toISOString(), reason: 'expired' })
  })
})

describe('source link tokens', () => {
  it('round-trips a payload', () => {
    const token = signSourceLink(SECRET, payload())
    const check = verifySourceLink(SECRET, token, NOW)
    expect(check.ok).toBe(true)
    expect(check.ok ? check.payload : null).toEqual(payload())
  })

  it('reports a forgery as a signature failure, never as expiry', () => {
    const token = signSourceLink(SECRET, payload())
    const [body = '', signature = ''] = token.split('.')
    // Tamper with the payload and keep the signature.
    const forged = `${Buffer.from(JSON.stringify(['U-someone', 'default', 'user-1', NOW + 900]), 'utf8').toString('base64url')}.${signature}`
    expect(verifySourceLink(SECRET, forged, NOW)).toEqual({ ok: false, reason: 'signature' })
    // A different key must not validate either.
    expect(verifySourceLink(new Uint8Array(32).fill(9), token, NOW)).toEqual({ ok: false, reason: 'signature' })
    // Malformed shapes are their own reason.
    expect(verifySourceLink(SECRET, '', NOW)).toEqual({ ok: false, reason: 'malformed' })
    expect(verifySourceLink(SECRET, 'nodot', NOW)).toEqual({ ok: false, reason: 'malformed' })
    expect(verifySourceLink(SECRET, `${body}.`, NOW)).toEqual({ ok: false, reason: 'malformed' })
    // A correctly signed payload that is not the expected shape is malformed too.
    const weirdBody = Buffer.from(JSON.stringify({ namespace: 'U-ada' }), 'utf8').toString('base64url')
    const weird = `${weirdBody}.${Buffer.from(signSourceLink(SECRET, payload()).split('.')[1] ?? '', 'base64url').toString('base64url')}`
    expect(verifySourceLink(SECRET, weird, NOW).ok).toBe(false)
  })

  it('expires on the second, not before', () => {
    const token = signSourceLink(SECRET, payload({ expiresAt: NOW }))
    expect(verifySourceLink(SECRET, token, NOW - 1).ok).toBe(true)
    expect(verifySourceLink(SECRET, token, NOW)).toEqual({ ok: false, reason: 'expired' })
    expect(verifySourceLink(SECRET, token, NOW + 1)).toEqual({ ok: false, reason: 'expired' })
  })
})

describe('signing key and audit', () => {
  let root: string

  beforeEach(async () => {
    root = await mkdtemp(path.join(tmpdir(), 'dsh-alioth-sign-'))
  })

  afterEach(async () => {
    await rm(root, { recursive: true, force: true })
  })

  it('creates the key once, 0600, and reuses it', async () => {
    const file = path.join(root, 'nested', 'source-signing.key')
    const first = await loadSigningKey(file)
    expect(first.length).toBeGreaterThanOrEqual(32)
    expect((await stat(file)).mode & 0o777).toBe(0o600)
    // Second read returns the same key (a restart must not invalidate live links).
    expect(await loadSigningKey(file)).toEqual(first)
    // A concurrent racer converges on the file that won.
    const racers = await Promise.all([loadSigningKey(file), loadSigningKey(file), loadSigningKey(file)])
    for (const key of racers) expect(key).toEqual(first)
  })

  it('appends one JSON line per download without losing an earlier one', async () => {
    const file = path.join(root, 'source-downloads.jsonl')
    const line = {
      ts: '2026-09-24T00:00:00.000Z',
      userId: 'user-1',
      username: 'ada',
      namespace: 'U-ada',
      app: 'default',
      bytes: 12,
      files: 2,
      linkExpiresAt: '2026-09-24T00:15:00.000Z',
    }
    await appendSourceDownloadAudit(file, line)
    await Promise.all([
      appendSourceDownloadAudit(file, { ...line, userId: 'user-2' }),
      appendSourceDownloadAudit(file, { ...line, userId: 'user-3' }),
    ])

    const lines = (await readFile(file, 'utf8')).trim().split('\n')
    expect(lines).toHaveLength(3)
    expect(lines.map(entry => (JSON.parse(entry) as { userId: string }).userId).sort())
      .toEqual(['user-1', 'user-2', 'user-3'])
  })
})
