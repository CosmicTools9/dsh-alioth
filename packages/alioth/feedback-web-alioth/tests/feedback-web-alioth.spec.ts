import { describe, expect, it, beforeAll, afterAll } from 'vitest'
import { mkdtemp } from 'node:fs/promises'
import { DatabaseSync } from 'node:sqlite'
import { networkInterfaces, tmpdir } from 'node:os'
import path from 'node:path'
import { createServer } from 'node:net'
import { Context } from '@deepseek-ai/cordis'
import * as pageFeedback from '@deepseek-ai/dsh-page-feedback'
import * as feedbackWeb from '../src/index.ts'

let ctx: Context
const disposers: Array<() => Promise<void>> = []
let port: number

beforeAll(async () => {
  const dir = await mkdtemp(path.join(tmpdir(), 'feedbackweb-'))
  ctx = new Context()
  const store = await ctx.plugin(pageFeedback, { dbPath: path.join(dir, 'f.db') })
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
    await adminCtx.plugin(pageFeedback, { dbPath: path.join(dir, 'f.db') })
    const adminPort = 14960 + Math.floor(Math.random() * 50)
    await adminCtx.plugin(feedbackWeb, { port: adminPort })
    const adminBase = `http://127.0.0.1:${adminPort}`

    const annotation = adminCtx.pageFeedback.addAnnotation({ origin: 'o', url: 'u', comment: 'c' })
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

// ── Trust-boundary coverage ────────────────────────────────────────────────
// One carrier per concern: the origin allowlist, the loopback predicate, the
// watch seam and the hand-built-config defaults each need their own config.
// Every context owns its SQLite file and its own port.

/** Probe a free loopback port (bind :0, read it, release). */
async function freePort(): Promise<number> {
  const { promise, resolve, reject } = Promise.withResolvers<number>()
  const probe = createServer()
  probe.once('error', reject)
  probe.listen(0, '127.0.0.1', () => {
    const address = probe.address()
    if (address === null || typeof address === 'string') {
      probe.close(() => reject(new Error('no port assigned')))
      return
    }
    probe.close(() => resolve(address.port))
  })
  return promise
}

interface Carrier {
  readonly ctx: Context
  readonly port: number
  readonly base: string
  readonly dbPath: string
  dispose(): Promise<void>
}

/** Start a carrier with its own annotation store; `config.port` wins when given. */
async function startCarrier(config: Record<string, unknown> = {}, host?: string): Promise<Carrier> {
  const dir = await mkdtemp(path.join(tmpdir(), 'feedbackweb-boundary-'))
  const dbPath = path.join(dir, 'f.db')
  const carrierCtx = new Context()
  const store = await carrierCtx.plugin(pageFeedback, { dbPath })
  const port = typeof config.port === 'number' ? config.port : await freePort()
  const carrier = await carrierCtx.plugin(feedbackWeb, { port, ...(host === undefined ? {} : { host }), ...config })
  return {
    ctx: carrierCtx,
    port,
    base: `http://127.0.0.1:${port}`,
    dbPath,
    dispose: async () => {
      await carrier.dispose().catch(() => {})
      await store.dispose().catch(() => {})
    },
  }
}

/** Read one field of an external JSON object, failing loud on a bad shape. */
function field(body: unknown, key: string, context: string): unknown {
  if (typeof body !== 'object' || body === null || !(key in body)) {
    throw new Error(`${context}: response body has no ${key}: ${JSON.stringify(body)}`)
  }
  return (body as Record<string, unknown>)[key]
}

function stringField(body: unknown, key: string, context: string): string {
  const value = field(body, key, context)
  if (typeof value !== 'string') {
    throw new Error(`${context}: ${key} is not a string: ${JSON.stringify(value)}`)
  }
  return value
}

/** Annotation ids of a JSON array response. */
function annotationIds(body: unknown, context: string): string[] {
  if (!Array.isArray(body)) {
    throw new Error(`${context}: expected a JSON array: ${JSON.stringify(body)}`)
  }
  return body.map((entry, index) => stringField(entry, 'id', `${context}[${String(index)}]`))
}

/** POST an annotation; the default origin is the allowlisted browser origin. */
function postAnnotation(base: string, body: string, headers: Record<string, string> = {}): Promise<Response> {
  return fetch(`${base}/api/feedback/annotations`, {
    method: 'POST',
    headers: { 'content-type': 'application/json', origin: 'http://127.0.0.1:9999', ...headers },
    body,
  })
}

/** The first non-internal IPv4 of this host, when it has one. */
function lanAddress(): string | undefined {
  for (const addresses of Object.values(networkInterfaces())) {
    for (const address of addresses ?? []) {
      if (address.family === 'IPv4' && !address.internal) {
        return address.address
      }
    }
  }
  return undefined
}

describe('feedback carrier origin allowlist and payload defaults', () => {
  let carrier: Carrier

  beforeAll(async () => {
    carrier = await startCarrier({ allowedOrigins: ['http://127.0.0.1:9999'], allowNullOrigin: true })
  }, 30_000)

  afterAll(async () => {
    await carrier.dispose().catch(() => {})
  })

  it('rejects a write that carries no Origin header at all', async () => {
    const response = await fetch(`${carrier.base}/api/feedback/annotations`, {
      method: 'POST',
      headers: { 'content-type': 'application/json' },
      body: JSON.stringify({ comment: '无来源头' }),
    })
    expect(response.status).toBe(403)
    expect(await response.json()).toEqual({ error: 'origin not allowed' })

    // An origin-less preflight is answered, but carries no CORS grant.
    const preflight = await fetch(`${carrier.base}/api/feedback/annotations`, { method: 'OPTIONS' })
    expect(preflight.status).toBe(204)
    expect(preflight.headers.get('access-control-allow-origin')).toBeNull()
  })

  it('answers a preflight from an unknown origin without CORS headers', async () => {
    const response = await fetch(`${carrier.base}/api/feedback/annotations`, {
      method: 'OPTIONS',
      headers: { origin: 'http://evil.example', 'access-control-request-method': 'POST' },
    })
    expect(response.status).toBe(204)
    expect(response.headers.get('access-control-allow-origin')).toBeNull()
  })

  it('accepts the opaque origin only when it is opted in', async () => {
    const optedIn = await postAnnotation(carrier.base, JSON.stringify({ comment: 'file:// 原型批注' }), { origin: 'null' })
    expect(optedIn.status).toBe(201)
    expect(optedIn.headers.get('access-control-allow-origin')).toBe('null')
    expect(stringField(await optedIn.json(), 'origin', 'null-origin write')).toBe('null')

    const strict = await startCarrier({ allowedOrigins: ['http://127.0.0.1:9999'] })
    try {
      const rejected = await postAnnotation(strict.base, JSON.stringify({ comment: '沙箱 iframe' }), { origin: 'null' })
      expect(rejected.status).toBe(403)
      expect(await rejected.json()).toEqual({ error: 'origin not allowed' })
    } finally {
      await strict.dispose().catch(() => {})
    }
  })

  it('takes the origin from the request header and pads the absent fields', async () => {
    const response = await postAnnotation(carrier.base, JSON.stringify({ comment: '只有注释' }))
    expect(response.status).toBe(201)
    expect(await response.json()).toMatchObject({
      origin: 'http://127.0.0.1:9999',
      url: '',
      element: '',
      elementPath: '',
      cssClasses: '',
      status: 'pending',
      reply: null,
    })
  })

  it('carries element metadata and honours an explicit session id', async () => {
    // The browser may name an existing session; the carrier forwards it.
    const session = carrier.ctx.pageFeedback.ensureSession('http://127.0.0.1:9999', 'http://127.0.0.1:9999/page')
    const explicit = await postAnnotation(carrier.base, JSON.stringify({
      comment: '按钮错位',
      sessionId: session.id,
      element: 'button#go',
      elementPath: 'main > button#go',
      cssClasses: 'btn primary',
    }))
    expect(await explicit.json()).toMatchObject({
      sessionId: session.id,
      element: 'button#go',
      elementPath: 'main > button#go',
      cssClasses: 'btn primary',
    })

    // An empty sessionId means "no session": the store opens a fresh one.
    const generated = await postAnnotation(carrier.base, JSON.stringify({ comment: '无会话', sessionId: '' }))
    const generatedId = stringField(await generated.json(), 'sessionId', 'implicit session')
    expect(generatedId).not.toBe('')
    expect(generatedId).not.toBe(session.id)
  })

  it('rejects an empty comment with the store error over HTTP', async () => {
    const response = await postAnnotation(carrier.base, JSON.stringify({ comment: '   ' }))
    expect(response.status).toBe(400)
    expect(stringField(await response.json(), 'error', 'empty comment')).toContain('comment must not be empty')
  })

  it('rejects a malformed JSON body', async () => {
    const response = await postAnnotation(carrier.base, '{"comment": "unterminated')
    expect(response.status).toBe(400)
    expect(stringField(await response.json(), 'error', 'bad json write')).toContain('invalid JSON body')
  })

  it('reads an empty body as an empty payload', async () => {
    const response = await postAnnotation(carrier.base, '')
    expect(response.status).toBe(400)
    // The body defaults to `{}` — not a parse failure.
    expect(stringField(await response.json(), 'error', 'empty body write')).toContain('comment must not be empty')
  })

  it('rejects a malformed JSON patch body on the consumer endpoint', async () => {
    const created = await postAnnotation(carrier.base, JSON.stringify({ comment: '待补丁' }))
    const id = stringField(await created.json(), 'id', 'create for patch')
    const response = await fetch(`${carrier.base}/api/feedback/annotations/${id}`, {
      method: 'PATCH',
      headers: { 'content-type': 'application/json' },
      body: '{not json',
    })
    expect(response.status).toBe(400)
    expect(stringField(await response.json(), 'error', 'bad json patch')).toContain('invalid JSON body')
  })

  it('wakes a long-poll watch with the annotation written after it opened', async () => {
    // No ?timeout → the carrier's 25 s default; the POST below settles the poll.
    const watching = fetch(`${carrier.base}/api/feedback/watch`)
    await new Promise(resolve => setTimeout(resolve, 750))
    const created = await postAnnotation(carrier.base, JSON.stringify({ comment: '唤醒 watch' }))
    const id = stringField(await created.json(), 'id', 'create for watch')

    const response = await watching
    expect(response.status).toBe(200)
    expect(annotationIds(await response.json(), 'watch batch')).toContain(id)
  })

  it('returns the pending batch immediately for the zero timeout', async () => {
    const started = Date.now()
    const response = await fetch(`${carrier.base}/api/feedback/watch?timeout=0`)
    expect(response.status).toBe(200)
    expect(Array.isArray(await response.json())).toBe(true)
    expect(Date.now() - started).toBeLessThan(2_000)
  })

  it('walks the state machine over HTTP and refuses to re-open a terminal annotation', async () => {
    const created = await postAnnotation(carrier.base, JSON.stringify({ comment: '状态机' }))
    const id = stringField(await created.json(), 'id', 'create for state machine')
    const patch = (status: string): Promise<Response> => fetch(`${carrier.base}/api/feedback/annotations/${id}`, {
      method: 'PATCH',
      headers: { 'content-type': 'application/json' },
      body: JSON.stringify({ status }),
    })

    // pending → acknowledged → resolved is the legal path.
    expect(await (await patch('acknowledged')).json()).toMatchObject({ status: 'acknowledged' })
    expect(await (await patch('resolved')).json()).toMatchObject({ status: 'resolved' })

    // resolved is terminal: re-opening it is refused and the row stays resolved.
    const illegal = await patch('acknowledged')
    expect(illegal.status).toBe(400)
    expect(stringField(await illegal.json(), 'error', 'terminal re-open')).toContain('is not an allowed transition')
    expect(annotationIds(await (await fetch(`${carrier.base}/api/feedback/pending`)).json(), 'pending after re-open'))
      .not.toContain(id)
  })

  it('prunes a dismissed annotation past the 24h cutoff and keeps fresh ones', async () => {
    const session = carrier.ctx.pageFeedback.ensureSession('http://127.0.0.1:9999', 'http://127.0.0.1:9999/prune')
    const dismissed = await postAnnotation(carrier.base, JSON.stringify({ sessionId: session.id, comment: '旧批注' }))
    const staleId = stringField(await dismissed.json(), 'id', 'stale annotation')
    const fresh = await postAnnotation(carrier.base, JSON.stringify({ sessionId: session.id, comment: '新鲜批注' }))
    const freshId = stringField(await fresh.json(), 'id', 'fresh annotation')
    await fetch(`${carrier.base}/api/feedback/annotations/${staleId}`, {
      method: 'PATCH',
      headers: { 'content-type': 'application/json' },
      body: JSON.stringify({ status: 'dismissed' }),
    })

    // Nothing is a candidate while every row is fresh…
    const prunedFresh = await fetch(`${carrier.base}/api/feedback/prune`, { method: 'POST' })
    expect(await prunedFresh.json()).toEqual({ pruned: 0 })

    // …then backdate the dismissed row past the cutoff and prune again.
    const database = new DatabaseSync(carrier.dbPath)
    try {
      database.prepare('UPDATE annotations SET updated_at = ? WHERE id = ?')
        .run(Date.now() - 25 * 3600 * 1000, staleId)
    } finally {
      database.close()
    }
    const prunedStale = await fetch(`${carrier.base}/api/feedback/prune`, { method: 'POST' })
    expect(await prunedStale.json()).toEqual({ pruned: 1 })

    // Only the fresh pending row survives.
    const remaining = await (await fetch(`${carrier.base}/api/feedback/pending`)).json()
    expect(annotationIds(remaining, 'pending after prune')).toContain(freshId)
    expect((await (await fetch(`${carrier.base}/api/feedback/annotations/${staleId}`, { method: 'GET' })).status)).toBe(404)
  })
})

describe('feedback carrier loopback boundary and config defaults', () => {
  const started: Carrier[] = []

  afterAll(async () => {
    for (const carrier of started.reverse()) {
      await carrier.dispose().catch(() => {})
    }
  })

  it('treats a LAN caller as foreign and keeps a wildcard bind reachable', async () => {
    const wildcard = await startCarrier({}, '0.0.0.0')
    started.push(wildcard)
    // A wildcard bind still advertises a reachable origin for the bookmarklet.
    const page = await (await fetch(`${wildcard.base}/feedback`)).text()
    expect(page).toContain(`http://127.0.0.1:${String(wildcard.port)}/feedback/overlay.js`)

    const lan = lanAddress()
    if (lan !== undefined) {
      // Same machine, but the socket is not on a loopback address.
      const foreign = await fetch(`http://${lan}:${String(wildcard.port)}/api/feedback/pending`)
      expect(foreign.status).toBe(403)
      expect(await foreign.json()).toEqual({ error: 'loopback only' })
    }

    // The consumer endpoints answer the loopback caller on the same carrier.
    const local = await fetch(`${wildcard.base}/api/feedback/pending`)
    expect(local.status).toBe(200)
  }, 30_000)

  it('says the allowlist is empty on the landing page of a consumers-only carrier', async () => {
    const consumersOnly = await startCarrier({ allowedOrigins: [] })
    started.push(consumersOnly)
    const page = await (await fetch(`${consumersOnly.base}/feedback`)).text()
    expect(page).toContain('（空——仅消费者回环可用）')

    // No browser origin can write any more.
    const rejected = await postAnnotation(consumersOnly.base, JSON.stringify({ comment: 'x' }))
    expect(rejected.status).toBe(403)
  })

  it('normalizes a hand-built partial config to the documented defaults', async () => {
    // `apply` is the plugin entry: a deployment that hand-builds its context
    // (rather than going through the Loader's schema defaults) must still get
    // the documented allowlist, bind address and null-origin policy.
    const dir = await mkdtemp(path.join(tmpdir(), 'feedbackweb-defaults-'))
    const defaultsCtx = new Context()
    const store = await defaultsCtx.plugin(pageFeedback, { dbPath: path.join(dir, 'f.db') })
    const port = await freePort()
    feedbackWeb.apply(defaultsCtx, { port })

    try {
      const health = await (await fetch(`http://127.0.0.1:${port}/health`)).json()
      expect(health).toMatchObject({ ok: true })

      for (const origin of ['http://127.0.0.1:3100', 'http://localhost:3100']) {
        const preflight = await fetch(`http://127.0.0.1:${port}/api/feedback/annotations`, {
          method: 'OPTIONS',
          headers: { origin },
        })
        expect(preflight.status).toBe(204)
        expect(preflight.headers.get('access-control-allow-origin')).toBe(origin)
      }

      // allowNullOrigin defaults to false: the opaque origin stays refused.
      const opaque = await postAnnotation(`http://127.0.0.1:${port}`, JSON.stringify({ comment: 'x' }), { origin: 'null' })
      expect(opaque.status).toBe(403)
    } finally {
      await store.dispose().catch(() => {})
    }
  }, 30_000)

  it('logs a port collision instead of crashing the deployment', async () => {
    const holder = await startCarrier({})
    started.push(holder)

    const errors: string[] = []
    const collidingCtx = new Context()
    const store = await collidingCtx.plugin(pageFeedback, {
      dbPath: path.join(await mkdtemp(path.join(tmpdir(), 'feedbackweb-collision-')), 'f.db'),
    })
    collidingCtx.logger.error = (...args: unknown[]) => { errors.push(args.map(String).join(' ')) }
    const colliding = await collidingCtx.plugin(feedbackWeb, { port: holder.port })
    try {
      await new Promise(resolve => setTimeout(resolve, 300))
      expect(errors.join('\n')).toContain('HTTP server failed')
      expect(errors.join('\n')).toContain('EADDRINUSE')
    } finally {
      await colliding.dispose().catch(() => {})
      await store.dispose().catch(() => {})
    }
  }, 30_000)
})
