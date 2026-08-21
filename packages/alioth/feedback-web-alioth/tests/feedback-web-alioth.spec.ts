import { describe, expect, it, beforeAll, afterAll } from 'vitest'
import { mkdtemp } from 'node:fs/promises'
import { tmpdir } from 'node:os'
import path from 'node:path'
import { Context } from '@deepseek-ai/cordis'
import * as feedbackAlioth from '@dsh-alioth/feedback-alioth'
import * as feedbackWeb from '../src/index.ts'

let ctx: Context
const disposers: Array<() => Promise<void>> = []
let port: number

beforeAll(async () => {
  const dir = await mkdtemp(path.join(tmpdir(), 'feedbackweb-'))
  ctx = new Context()
  const store = await ctx.plugin(feedbackAlioth, { dbPath: path.join(dir, 'f.db') })
  disposers.push(() => store.dispose())
  port = 14860 + Math.floor(Math.random() * 100)
  const carrier = await ctx.plugin(feedbackWeb, { port, allowedOrigins: ['http://127.0.0.1:9999'] })
  disposers.push(() => carrier.dispose())
}, 30_000)

afterAll(async () => {
  for (const dispose of disposers.reverse()) {
    await dispose().catch(() => {})
  }
})

describe('feedback carrier', () => {
  const base = (): string => `http://127.0.0.1:${port}`

  it('serves health, the bookmarklet page, and the overlay script', async () => {
    const health = await (await fetch(`${base()}/health`)).json() as { ok: boolean }
    expect(health.ok).toBe(true)

    const page = await fetch(`${base()}/feedback`)
    expect(page.status).toBe(200)
    const html = await page.text()
    expect(html).toContain('圈选批注')

    const overlay = await fetch(`${base()}/feedback/overlay.js`)
    expect(overlay.status).toBe(200)
    expect(await overlay.text()).toContain('Alt+点击')
  })

  it('rejects annotation writes from non-allowlisted origins', async () => {
    const response = await fetch(`${base()}/api/feedback/annotations`, {
      method: 'POST',
      headers: { 'content-type': 'application/json', origin: 'http://evil.example' },
      body: JSON.stringify({ origin: 'http://evil.example', url: 'http://evil.example/x', comment: 'xss' }),
    })
    expect(response.status).toBe(403)
  })

  it('accepts allowlisted annotation writes and drives the consumer loop over HTTP', async () => {
    const created = await fetch(`${base()}/api/feedback/annotations`, {
      method: 'POST',
      headers: { 'content-type': 'application/json', origin: 'http://127.0.0.1:9999' },
      body: JSON.stringify({ origin: 'http://127.0.0.1:9999', url: 'http://127.0.0.1:9999/page', comment: '按钮错位', element: 'button', elementPath: 'header > button.primary' }),
    })
    expect(created.status).toBe(201)
    const annotation = await created.json() as { id: string; status: string }
    expect(annotation.status).toBe('pending')
    expect(created.headers.get('access-control-allow-origin')).toBe('http://127.0.0.1:9999')

    const pending = await (await fetch(`${base()}/api/feedback/pending`)).json() as Array<{ id: string }>
    expect(pending).toHaveLength(1)

    const acked = await fetch(`${base()}/api/feedback/annotations/${annotation.id}`, {
      method: 'PATCH',
      headers: { 'content-type': 'application/json' },
      body: JSON.stringify({ status: 'acknowledged', reply: '处理中' }),
    })
    expect(acked.status).toBe(200)
    expect(((await acked.json()) as { status: string }).status).toBe('acknowledged')

    const illegal = await fetch(`${base()}/api/feedback/annotations/${annotation.id}`, {
      method: 'PATCH',
      headers: { 'content-type': 'application/json' },
      body: JSON.stringify({ status: 'nonsense' }),
    })
    expect(illegal.status).toBe(200) // unknown status treated as reply-only patch
  })

  it('answers OPTIONS preflight for allowlisted origins', async () => {
    const preflight = await fetch(`${base()}/api/feedback/annotations`, {
      method: 'OPTIONS',
      headers: { origin: 'http://127.0.0.1:9999', 'access-control-request-method': 'POST' },
    })
    expect(preflight.status).toBe(204)
    expect(preflight.headers.get('access-control-allow-origin')).toBe('http://127.0.0.1:9999')
  })

  it('upgrades mutation endpoints to admin bearer when the auth capability is present', async () => {
    // Hand-built context providing a fake auth service — exercises the
    // requireAdmin branch without the embedded-PG fixture.
    const dir = await mkdtemp(path.join(tmpdir(), 'feedbackweb-admin-'))
    const adminCtx = new Context()
    adminCtx.provide('aliothAuth', {
      async userForToken(token: string | null): Promise<{ role: 'admin' | 'user' } | null> {
        if (token === 'admin-token') return { role: 'admin' }
        if (token === 'user-token') return { role: 'user' }
        return null
      },
    })
    await adminCtx.plugin(feedbackAlioth, { dbPath: path.join(dir, 'f.db') })
    const adminPort = 14960 + Math.floor(Math.random() * 50)
    await adminCtx.plugin(feedbackWeb, { port: adminPort })
    const adminBase = `http://127.0.0.1:${adminPort}`

    const annotation = adminCtx.aliothFeedback.addAnnotation({ origin: 'o', url: 'u', comment: 'c' })
    const patch = (token: string | null): Promise<Response> => fetch(`${adminBase}/api/feedback/annotations/${annotation.id}`, {
      method: 'PATCH',
      headers: { 'content-type': 'application/json', ...(token === null ? {} : { authorization: `Bearer ${token}` }) },
      body: JSON.stringify({ status: 'acknowledged' }),
    })

    expect((await patch(null)).status).toBe(401)
    expect((await patch('user-token')).status).toBe(401)
    expect((await patch('admin-token')).status).toBe(200)

    const pruneNoAuth = await fetch(`${adminBase}/api/feedback/prune`, { method: 'POST' })
    expect(pruneNoAuth.status).toBe(401)
    const pruneAdmin = await fetch(`${adminBase}/api/feedback/prune`, {
      method: 'POST',
      headers: { authorization: 'Bearer admin-token' },
    })
    expect(pruneAdmin.status).toBe(200)
  })
})
