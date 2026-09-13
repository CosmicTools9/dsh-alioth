import { describe, expect, it, beforeAll, afterAll } from 'vitest'
import { mkdir, mkdtemp, readFile, writeFile } from 'node:fs/promises'
import { request as httpRequest } from 'node:http'
import { connect } from 'node:net'
import { tmpdir } from 'node:os'
import path from 'node:path'
import { setTimeout as delay } from 'node:timers/promises'
import { Context } from '@deepseek-ai/cordis'
import SystemPrompt from '@deepseek-ai/dsh-system-prompt'
import ToolRuntime from '@deepseek-ai/dsh-tools'
import WebServer from '@deepseek-ai/dsh-host-webserver'
import * as envAlioth from '@dsh-alioth/env-alioth'
import * as authAlioth from '@dsh-alioth/auth-alioth'
import * as landingAlioth from '@dsh-alioth/landing-alioth'
import * as authWeb from '../src/index.ts'

const SCHEMA_DDL = `
CREATE TYPE isahl_meta.collection_type AS ENUM ('table', 'view');
CREATE TYPE isahl_meta.field_category AS ENUM ('scalar', 'reference', 'computed', 'auto');
CREATE TYPE isahl_meta.field_data_type AS ENUM ('text', 'decimal', 'bigint');
CREATE TABLE isahl_meta.meta_collections (
    table_name text NOT NULL,
    name text NOT NULL,
    type isahl_meta.collection_type,
    config jsonb DEFAULT '{}'::jsonb,
    data_source text,
    schema text DEFAULT 'isahl'::text,
    biz_description text,
    PRIMARY KEY (table_name)
);
CREATE TABLE isahl_meta.meta_fields (
    fk_collection text NOT NULL REFERENCES isahl_meta.meta_collections(table_name) ON DELETE CASCADE,
    name text NOT NULL,
    category isahl_meta.field_category,
    data_type isahl_meta.field_data_type,
    is_required boolean DEFAULT false,
    default_value text,
    config jsonb DEFAULT '{}'::jsonb,
    title text NOT NULL DEFAULT ''::text,
    PRIMARY KEY (fk_collection, name)
);
`

let ctx: Context
const disposers: Array<() => Promise<void>> = []
let port: number
let previewPreProcRoot: string
let dataRoot: string

/** Harness account-resolver face the carrier registers into `connection`. */
type AccountResolver = (headers: { cookie?: string; host?: string }) => string | null | Promise<string | null>
let accountResolver: AccountResolver | undefined

/** The `connection` stand-in mounted in beforeAll (restored after variants). */
let connectionStub: {
  authenticatedUrl: (base: string) => string
  registerAccountResolver: (resolver: AccountResolver) => () => void
}

/** Extract the `{ error }` payload of a failing response (narrowed, no cast). */
async function jsonError(response: Response): Promise<string> {
  const body: unknown = await response.json()
  if (typeof body === 'object' && body !== null && 'error' in body && typeof body.error === 'string') {
    return body.error
  }
  throw new Error(`response has no error field: ${JSON.stringify(body)}`)
}

/** Register a fresh account on the standalone server; returns token + cookie. */
async function registerUser(username: string): Promise<{ token: string; cookie: string }> {
  const response = await fetch(`http://127.0.0.1:${port}/api/auth/register`, {
    method: 'POST',
    headers: { 'content-type': 'application/json' },
    body: JSON.stringify({ username, password: 'password-789' }),
  })
  if (response.status !== 201) {
    throw new Error(`register ${username} failed: ${response.status} ${await response.text()}`)
  }
  const body: unknown = await response.json()
  if (typeof body !== 'object' || body === null || !('token' in body) || typeof body.token !== 'string') {
    throw new Error(`register ${username}: no token in response`)
  }
  return {
    token: body.token,
    cookie: response.headers.getSetCookie().map(cookie => cookie.split(';')[0]).join('; '),
  }
}

/** Minimal HTTP client — the tests that must control Host/Origin cannot use
 * fetch (forbidden headers). */
function rawRequest(target: {
  port: number
  path: string
  method?: string
  headers?: Record<string, string>
  body?: string
}): Promise<{ status: number; headers: Record<string, string | string[] | undefined>; body: string }> {
  const { promise, resolve, reject } = Promise.withResolvers<{
    status: number
    headers: Record<string, string | string[] | undefined>
    body: string
  }>()
  const request = httpRequest({
    host: '127.0.0.1',
    port: target.port,
    path: target.path,
    method: target.method ?? 'GET',
    headers: target.headers,
  }, response => {
    const chunks: Buffer[] = []
    response.on('data', chunk => chunks.push(Buffer.from(chunk)))
    response.on('end', () => resolve({
      status: response.statusCode ?? 0,
      headers: response.headers,
      body: Buffer.concat(chunks).toString('utf8'),
    }))
  })
  request.on('error', reject)
  if (target.body !== undefined) request.write(target.body)
  request.end()
  return promise
}

/** HTTP/1.0 over a raw socket — the only way to reach the server without a
 * Host header (Node's HTTP/1.1 parser rejects those requests with 400 before
 * any route sees them). */
function hostlessRequest(
  targetPort: number,
  path: string,
  options: { method?: string; headers?: Record<string, string>; body?: string } = {},
): Promise<{ status: number; head: string; body: string }> {
  const { promise, resolve, reject } = Promise.withResolvers<{ status: number; head: string; body: string }>()
  const socket = connect(targetPort, '127.0.0.1', () => {
    const body = options.body ?? ''
    const lines = [`${options.method ?? 'GET'} ${path} HTTP/1.0`]
    for (const [name, value] of Object.entries(options.headers ?? {})) lines.push(`${name}: ${value}`)
    if (body !== '') lines.push(`content-length: ${Buffer.byteLength(body)}`)
    socket.write(`${lines.join('\r\n')}\r\n\r\n${body}`)
  })
  let raw = ''
  socket.on('data', chunk => { raw += chunk.toString('utf8') })
  socket.on('end', () => {
    const [head = '', ...rest] = raw.split('\r\n\r\n')
    resolve({
      status: Number(/^HTTP\/1\.[01] (\d{3})/.exec(head)?.[1] ?? 0),
      head,
      body: rest.join('\r\n\r\n'),
    })
  })
  socket.on('error', reject)
  return promise
}

/** Wait for the carrier's account-resolver registration — the web gate keeps
 * retrying until the `connection` service becomes reachable. */
async function waitForAccountResolver(timeoutMs = 5000): Promise<AccountResolver> {
  const deadline = Date.now() + timeoutMs
  while (Date.now() < deadline) {
    if (accountResolver !== undefined) return accountResolver
    await delay(50)
  }
  throw new Error('the carrier never registered an account resolver')
}

beforeAll(async () => {
  const modelDir = await mkdtemp(path.join(tmpdir(), 'authweb-model-'))
  dataRoot = await mkdtemp(path.join(tmpdir(), 'authweb-data-'))
  await mkdir(path.join(modelDir, 'backend', 'ddl'), { recursive: true })
  await mkdir(path.join(modelDir, 'backend', 'vendor', 'alioth-gen', 'src'), { recursive: true })
  await mkdir(path.join(modelDir, 'skill-adapters'), { recursive: true })
  await mkdir(path.join(modelDir, 'Pre-Proc', 'Alioth', '_schema'), { recursive: true })
  await writeFile(path.join(modelDir, 'backend', 'ddl', '002_isahl_meta_schema.sql'), SCHEMA_DDL)
  await writeFile(path.join(modelDir, 'skill-adapters', 'a.yaml'), 'x\n')
  await writeFile(path.join(modelDir, 'Pre-Proc', 'Alioth', '_schema', 'a.schema.json'), '{}\n')
  await writeFile(
    path.join(modelDir, 'backend', 'vendor', 'alioth-gen', 'src', 'lib.rs'),
    'pub static ALIOTH_MODEL_VERSION: LazyLock<String> =\n    LazyLock::new(|| env::var("MODEL_VERSION").unwrap_or_else(|_| "10.0.0".to_string()));\n',
  )

  ctx = new Context()
  // The harness `connection` service slot (client-connection is web-only).
  // Kept value-less so the carrier behaves as in a connection-less tree until
  // a test mounts the stand-in below.
  ctx.provide('connection')
  const system = await ctx.plugin(SystemPrompt)
  disposers.push(() => system.dispose())
  const tools = await ctx.plugin(ToolRuntime)
  disposers.push(() => tools.dispose())
  const env = await ctx.plugin(envAlioth, { modelSource: modelDir, dataRoot })
  disposers.push(() => env.dispose())
  await ctx.aliothEnv.ready()

  // Real harness webServer (port 0 → kernel-assigned) mounted BEFORE the
  // carriers so their deferred ctx.inject(['webServer']) callbacks find it.
  const webServerPlugin = await ctx.plugin(WebServer, { host: '127.0.0.1', port: 0 })
  disposers.push(() => webServerPlugin.dispose())
  const landing = await ctx.plugin(landingAlioth, {})
  disposers.push(() => landing.dispose())
  previewPreProcRoot = path.join(dataRoot, 'Pre-Proc')
  const auth = await ctx.plugin(authAlioth, {
    mode: 'enforce',
    preProcRoot: path.join(dataRoot, 'Pre-Proc'),
    deployRoot: path.join(dataRoot, 'deploy'),
  })
  disposers.push(() => auth.dispose())

  port = 3987 + Math.floor(Math.random() * 500)
  // Harness `connection` service stand-in: the real one (client-connection)
  // is mounted by web profiles only; here the carrier's portal-URL
  // derivation and host-login account resolver are exercised over this
  // structural face. `authenticatedUrl` mimics the real launch-token append.
  connectionStub = {
    authenticatedUrl: (base: string) => {
      if (base.includes('throw.test')) throw new Error('not a reachable origin')
      return `${base}/?launch=stub-token`
    },
    registerAccountResolver: (resolver: AccountResolver) => {
      accountResolver = resolver
      return () => { accountResolver = undefined }
    },
  }
  const carrier = await ctx.plugin(authWeb, { port, preProcRoot: previewPreProcRoot })
  disposers.push(() => carrier.dispose())
}, 120_000)

afterAll(async () => {
  for (const dispose of disposers.reverse()) {
    await dispose().catch(() => {})
  }
})

describe('B/S HTTP surface (real server)', () => {
  const base = (): string => `http://127.0.0.1:${port}`

  it('serves the landing page at GET / (via the aliothLanding service)', async () => {
    const response = await fetch(`${base()}/`)
    expect(response.status).toBe(200)
    const html = await response.text()
    expect(html).toContain('Alioth AppCreator')
    expect(html).toContain('app-creation')
    expect(html).toContain('e2e-verification')
  })

  it('serves the login page (GET /login)', async () => {
    const response = await fetch(`${base()}/login`)
    expect(response.status).toBe(200)
    const html = await response.text()
    expect(html).toContain('<form')
    expect(html).toContain('/api/auth/login')
  })

  it('links register page back to /login', async () => {
    const response = await fetch(`${base()}/register`)
    expect(response.status).toBe(200)
    const html = await response.text()
    expect(html).toContain('href="/login"')
  })

  it('registers via browser form submission (urlencoded) → styled success page + cookies', async () => {
    const form = new URLSearchParams({ username: 'carol', password: 'password-789' })
    const response = await fetch(`${base()}/api/auth/register`, {
      method: 'POST',
      headers: { 'content-type': 'application/x-www-form-urlencoded' },
      body: form.toString(),
    })
    expect(response.status).toBe(201)
    expect(response.headers.get('content-type')).toContain('text/html')
    const html = await response.text()
    expect(html).toContain('U-carol')
    expect(html).toMatch(/class="token">[0-9a-f]{64}</)
    const cookies = response.headers.getSetCookie()
    expect(cookies.some(c => c.startsWith('alioth_session=') && c.includes('HttpOnly'))).toBe(true)
    expect(cookies.some(c => c.startsWith('alioth_user=carol'))).toBe(true)
  })

  it('rejects a browser form login with bad credentials → styled 401 page', async () => {
    const form = new URLSearchParams({ username: 'carol', password: 'wrong-pass-000' })
    const response = await fetch(`${base()}/api/auth/login`, {
      method: 'POST',
      headers: { 'content-type': 'application/x-www-form-urlencoded' },
      body: form.toString(),
    })
    expect(response.status).toBe(401)
    const html = await response.text()
    expect(html).toContain('用户名或密码错误')
    expect(html).toContain('/api/auth/login') // form re-rendered for retry
  })


  it('serves 成品预览 builds with namespace isolation (GET /preview/*)', async () => {
    // Fixture: prototype builds for Demo (carol's namespace U-carol) + a
    // shared design asset + another namespace's build (must NOT leak).
    const protoRoot = previewPreProcRoot
    await mkdir(path.join(protoRoot, 'U-pv-owner', 'Prototypes', 'Apps', 'demo-app'), { recursive: true })
    await writeFile(
      path.join(protoRoot, 'U-pv-owner', 'Prototypes', 'Apps', 'demo-app', 'a-v1.html'),
      '<!doctype html><html lang="zh"><title>成品预览</title><body>demo</body></html>',
    )
    await mkdir(path.join(previewPreProcRoot, '..', '.agents', 'skills'), { recursive: true })
    await writeFile(path.join(previewPreProcRoot, '..', '.agents', 'skills', 'asset.js'), 'shared-asset')

    // Self-contained fixtures: register owner + intruder fresh.
    const registerUser = async (username: string) => {
      const response = await fetch(`${base()}/api/auth/register`, {
        method: 'POST',
        headers: { 'content-type': 'application/json' },
        body: JSON.stringify({ username, password: 'password-789' }),
      })
      if (response.status !== 201) throw new Error(`register ${username}: ${response.status}`)
      return response.headers.getSetCookie().map(c => c.split(';')[0]).join('; ')
    }
    const ownerCookie = await registerUser('pv-owner')
    const intruderCookie = await registerUser('pv-intruder')
    const cookie = ownerCookie

    // Session cookie resolves identity (same as /api/auth/me).
    const me = await fetch(`${base()}/api/auth/me`, { headers: { cookie } })
    expect(me.status).toBe(200)
    expect(((await me.json()) as { namespace: string }).namespace).toBe('U-pv-owner')

    // Own-namespace build serves with the html content type.
    const own = await fetch(`${base()}/preview/Pre-Proc/U-pv-owner/Prototypes/Apps/demo-app/a-v1.html`, {
      headers: { cookie },
    })
    expect(own.status).toBe(200)
    expect(own.headers.get('content-type')).toContain('text/html')
    expect(await own.text()).toContain('成品预览')

    // Shared design assets serve for authenticated users…
    const shared = await fetch(`${base()}/preview/.agents/skills/asset.js`, { headers: { cookie } })
    expect(shared.status).toBe(200)

    // …but non-allowlisted roots stay 404.
    expect((await fetch(`${base()}/preview/backend/ddl/002_isahl_meta_schema.sql`, { headers: { cookie } })).status).toBe(404)

    // Another user cannot see carol's builds — silent 404, no existence leak.
    await fetch(`${base()}/api/auth/register`, {
      method: 'POST',
      headers: { 'content-type': 'application/json' },
      body: JSON.stringify({ username: 'mallory', password: 'password-789' }),
    })
    await fetch(`${base()}/api/auth/login`, {
      method: 'POST',
      headers: { 'content-type': 'application/json' },
      body: JSON.stringify({ username: 'mallory', password: 'password-789' }),
    })
    const leak = await fetch(`${base()}/preview/Pre-Proc/U-pv-owner/Prototypes/Apps/demo-app/a-v1.html`, {
      headers: { cookie: intruderCookie },
    })
    expect(leak.status).toBe(404)

    // Traversal is rejected before any filesystem access.
    expect((await fetch(`${base()}/preview/Pre-Proc/U-pv-owner/..%2F..%2F..%2Fbackend%2Fddl%2F002_isahl_meta_schema.sql`, { headers: { cookie } })).status).toBe(404)

    // Unauthenticated requests get 401.
    expect((await fetch(`${base()}/preview/Pre-Proc/U-pv-owner/Prototypes/Apps/demo-app/a-v1.html`)).status).toBe(401)
  })

  it('logs in via JSON API and reads /me', async () => {
    const login = await fetch(`${base()}/api/auth/login`, {
      method: 'POST',
      headers: { 'content-type': 'application/json' },
      body: JSON.stringify({ username: 'carol', password: 'password-789' }),
    })
    expect(login.status).toBe(200)
    const session = await login.json() as { token: string }
    const me = await fetch(`${base()}/api/auth/me`, {
      headers: { authorization: `Bearer ${session.token}` },
    })
    expect(me.status).toBe(200)
    expect(await me.json()).toMatchObject({ username: 'carol', namespace: 'U-carol', role: 'user' })
  })

  it('rejects invalid credentials over HTTP', async () => {
    const response = await fetch(`${base()}/api/auth/login`, {
      method: 'POST',
      headers: { 'content-type': 'application/json' },
      body: JSON.stringify({ username: 'carol', password: 'wrong-password' }),
    })
    expect(response.status).toBe(400)
    expect((await response.json() as { error: string }).error).toContain('invalid credentials')
  })

  it('logs out over HTTP and the token stops working', async () => {
    const login = await fetch(`${base()}/api/auth/login`, {
      method: 'POST',
      headers: { 'content-type': 'application/json' },
      body: JSON.stringify({ username: 'carol', password: 'password-789' }),
    })
    const { token } = await login.json() as { token: string }
    await fetch(`${base()}/api/auth/logout`, {
      method: 'POST',
      headers: { authorization: `Bearer ${token}` },
    })
    const me = await fetch(`${base()}/api/auth/me`, { headers: { authorization: `Bearer ${token}` } })
    expect(me.status).toBe(401)
  })
})

describe('web gate (real harness WebServer)', () => {
  const webBase = (): string => `http://127.0.0.1:${ctx.webServer.port}`

  it('serves the landing route mounted by landing-alioth on the GUI origin', async () => {
    const response = await fetch(`${webBase()}/landing`)
    expect(response.status).toBe(200)
    const html = await response.text()
    expect(html).toContain('Alioth AppCreator')
    expect(html).toContain('app-creation')
  })

  it('serves login/register on the GUI origin', async () => {
    const login = await fetch(`${webBase()}/login`)
    expect(login.status).toBe(200)
    expect(await login.text()).toContain('/api/auth/login')

    const register = await fetch(`${webBase()}/register`)
    expect(register.status).toBe(200)
    expect(await register.text()).toContain('href="/login"')
  })

  it('sets session + marker cookies on login and accepts the cookie on /me', async () => {
    const login = await fetch(`${webBase()}/api/auth/login`, {
      method: 'POST',
      headers: { 'content-type': 'application/json' },
      body: JSON.stringify({ username: 'carol', password: 'password-789' }),
    })
    expect(login.status).toBe(200)
    const cookies = login.headers.getSetCookie()
    const session = cookies.find(c => c.startsWith('alioth_session='))
    expect(session).toBeDefined()
    expect(session).toContain('HttpOnly')
    expect(cookies.some(c => c.startsWith('alioth_user=carol'))).toBe(true)

    const me = await fetch(`${webBase()}/api/auth/me`, { headers: { cookie: session!.split(';')[0]! } })
    expect(me.status).toBe(200)
    expect(await me.json()).toMatchObject({ username: 'carol', namespace: 'U-carol' })
  })

  it('binds agent sessions via /api/auth/bind; rejects bad tokens', async () => {
    const login = await fetch(`${webBase()}/api/auth/login`, {
      method: 'POST',
      headers: { 'content-type': 'application/json' },
      body: JSON.stringify({ username: 'carol', password: 'password-789' }),
    })
    const { token } = await login.json() as { token: string }
    const bind = await fetch(`${webBase()}/api/auth/bind`, {
      method: 'POST',
      headers: { 'content-type': 'application/json' },
      body: JSON.stringify({ token, sessionId: 'web-session-1' }),
    })
    expect(bind.status).toBe(204)
    expect(await ctx.aliothAuth.userForSessionId('web-session-1')).toMatchObject({ namespace: 'U-carol' })

    const bad = await fetch(`${webBase()}/api/auth/bind`, {
      method: 'POST',
      headers: { 'content-type': 'application/json' },
      body: JSON.stringify({ token: 'deadbeef'.repeat(8), sessionId: 'web-session-2' }),
    })
    expect(bad.status).toBe(401)
    expect(await ctx.aliothAuth.userForSessionId('web-session-2')).toBeNull()
  })

  it('injects the gate script through the index tap (target from aliothLanding)', () => {
    const out = ctx.webServer.applyIndexTaps('<html><head></head><body></body></html>')
    expect(out).toContain('alioth_user')
    expect(out).toContain("location.replace('/landing')")
    expect(out).toContain('/api/session/create')
    // The injected script must be syntactically valid JS — a broken gate
    // silently never redirects (this regressed once on string-concat seams).
    const match = out.match(/<script>([\s\S]*?)<\/script>/)
    if (match === null) throw new Error('gate script not injected')
    expect(() => new Function(match[1]!)).not.toThrow()
  })

  it('answers form logins on the GUI origin with a same-origin /workspace redirect', async () => {
    const response = await fetch(`${webBase()}/api/auth/login`, {
      method: 'POST',
      headers: { 'content-type': 'application/x-www-form-urlencoded' },
      body: new URLSearchParams({ username: 'carol', password: 'password-789' }),
      redirect: 'manual',
    })
    expect(response.status).toBe(302)
    expect(response.headers.get('location')).toBe('/workspace')
    // Cookies land on the caller's origin — no cross-origin token handoff.
    expect(response.headers.get('set-cookie') ?? '').toContain('alioth_user')
  })

  it('does not shadow harness workspace RPC sub-paths (/api/workspace/*)', async () => {
    const login = await fetch(`${webBase()}/api/auth/login`, {
      method: 'POST',
      headers: { 'content-type': 'application/json' },
      body: JSON.stringify({ username: 'carol', password: 'password-789' }),
    })
    const { token } = await login.json() as { token: string }
    const response = await fetch(`${webBase()}/api/workspace/create`, {
      method: 'POST',
      headers: { 'content-type': 'application/json', authorization: `Bearer ${token}` },
      body: JSON.stringify({ path: '/tmp/ws' }),
    })
    // The Alioth surface registers an EXACT /api/workspace route; a prefix
    // there would answer sub-paths with its own JSON 404 and starve the
    // harness client-connection /api route (longest-prefix-wins) of the
    // workspace RPC namespace. This tree has no harness /api interceptor, so
    // any non-JSON answer proves the fall-through (WebServer 404), while the
    // JSON body is the auth surface's signature.
    expect(response.headers.get('content-type') ?? '').not.toContain('application/json')
  })
})

describe('workspace surface (应用 — AppCreator standard only)', () => {
  const base = (): string => `http://127.0.0.1:${port}`

  it('requires authentication for /api/workspace', async () => {
    const response = await fetch(`${base()}/api/workspace`)
    expect(response.status).toBe(401)
  })

  it('rejects custom workspace creation in standard mode', async () => {
    const login = await fetch(`${base()}/api/auth/login`, {
      method: 'POST',
      headers: { 'content-type': 'application/json' },
      body: JSON.stringify({ username: 'carol', password: 'password-789' }),
    })
    const { token } = await login.json() as { token: string }
    const response = await fetch(`${base()}/api/workspace`, {
      method: 'POST',
      headers: { 'content-type': 'application/json', authorization: `Bearer ${token}` },
      body: JSON.stringify({ namespace: 'ProjectB' }),
    })
    expect(response.status).toBe(400)
    expect(((await response.json()) as { error: string }).error).toContain('disabled')
  })

  it('serves the workspace page (standard renders 应用 without workspace chrome)', async () => {
    const login = await fetch(`${base()}/api/auth/login`, {
      method: 'POST',
      headers: { 'content-type': 'application/json' },
      body: JSON.stringify({ username: 'carol', password: 'password-789' }),
    })
    const { token } = await login.json() as { token: string }
    const page = await fetch(`${base()}/workspace`, {
      headers: { authorization: `Bearer ${token}` },
    })
    expect(page.status).toBe(200)
    const html = await page.text()
    expect(html).toContain('<h1>应用</h1>')
    expect(html).toContain('U-carol')
    // standard hides the custom-workspace chrome: no Pre-Proc/Deploy paths, no create form
    expect(html).not.toContain('Pre-Proc/U-carol/')
    expect(html).not.toContain('Deploy/U-carol/')
    expect(html).not.toContain('action="/api/workspace"')
  })

  it('redirects unauthenticated visitors from /workspace to /login', async () => {
    const response = await fetch(`${base()}/workspace`, { redirect: 'manual' })
    expect(response.status).toBe(302)
    expect(response.headers.get('location')).toBe('/login')
  })

  it('includes the resolved workspace mode in /api/auth/me (client chip entry)', async () => {
    const login = await fetch(`${base()}/api/auth/login`, {
      method: 'POST',
      headers: { 'content-type': 'application/json' },
      body: JSON.stringify({ username: 'carol', password: 'password-789' }),
    })
    const { token } = await login.json() as { token: string }
    const me = await fetch(`${base()}/api/auth/me`, { headers: { authorization: `Bearer ${token}` } })
    expect(me.status).toBe(200)
    expect(await me.json()).toMatchObject({ username: 'carol', workspaceMode: 'standard' })
  })
})

describe('auth API transport branches (real server)', () => {
  const base = (): string => `http://127.0.0.1:${port}`

  it('answers unknown auth + unknown top-level paths with 404 JSON', async () => {
    const unknownAuth = await fetch(`${base()}/api/auth/nope`)
    expect(unknownAuth.status).toBe(404)
    expect(await unknownAuth.json()).toEqual({ error: 'not found' })

    const unknownTop = await fetch(`${base()}/nope`)
    expect(unknownTop.status).toBe(404)
    expect(await unknownTop.json()).toEqual({ error: 'not found' })

    // A non-GET /preview/* request is not a preview request at all.
    expect((await fetch(`${base()}/preview/Pre-Proc/x/a.html`, { method: 'POST' })).status).toBe(404)
  })

  it('rejects a malformed JSON body with the parse reason', async () => {
    const response = await fetch(`${base()}/api/auth/login`, {
      method: 'POST',
      headers: { 'content-type': 'application/json' },
      body: '{"username":',
    })
    expect(response.status).toBe(400)
    expect(await jsonError(response)).toMatch(/^invalid request body: /)
  })

  it('treats an empty JSON body as {} and rejects the missing credentials', async () => {
    const response = await fetch(`${base()}/api/auth/register`, {
      method: 'POST',
      headers: { 'content-type': 'application/json' },
    })
    expect(response.status).toBe(400)
    expect(await jsonError(response)).toContain('username must match')
  })

  it('parses a body with no content-type as empty (neither JSON nor form)', async () => {
    const response = await rawRequest({
      port,
      path: '/api/auth/login',
      method: 'POST',
      body: 'username=carol&password=password-789',
    })
    expect(response.status).toBe(400)
    expect(JSON.parse(response.body)).toMatchObject({ error: expect.stringContaining('invalid credentials') })
  })

  it('treats non-string credential fields as empty strings', async () => {
    const response = await fetch(`${base()}/api/auth/login`, {
      method: 'POST',
      headers: { 'content-type': 'application/json' },
      body: JSON.stringify({ username: 42, password: [1, 2] }),
    })
    expect(response.status).toBe(400)
    expect(await jsonError(response)).toContain('invalid credentials')
  })

  it('ignores a cookie jar without the session cookie and a tokenless Bearer header', async () => {
    const cookieJar = await fetch(`${base()}/api/auth/me`, { headers: { cookie: 'other=1; theme=dark' } })
    expect(cookieJar.status).toBe(401)
    expect(await cookieJar.json()).toEqual({ error: 'unauthorized' })

    const tokenless = await fetch(`${base()}/api/auth/me`, { headers: { authorization: 'Bearer' } })
    expect(tokenless.status).toBe(401)

    const basic = await fetch(`${base()}/api/auth/me`, { headers: { authorization: 'Basic Y2Fyb2w6eA==' } })
    expect(basic.status).toBe(401)
  })

  it('logs out a cookie-authenticated session and clears both cookies', async () => {
    const { token, cookie } = await registerUser('logout-cookie')
    const response = await fetch(`${base()}/api/auth/logout`, { method: 'POST', headers: { cookie } })
    expect(response.status).toBe(204)
    const cleared = response.headers.getSetCookie()
    expect(cleared.some(c => c.startsWith('alioth_session=;') && c.includes('Max-Age=0'))).toBe(true)
    expect(cleared.some(c => c.startsWith('alioth_user=;') && c.includes('Max-Age=0'))).toBe(true)
    // The session is gone server-side, not just in the browser.
    expect((await fetch(`${base()}/api/auth/me`, { headers: { authorization: `Bearer ${token}` } })).status).toBe(401)
  })

  it('renders the standalone form-login success page with the one-time token', async () => {
    const response = await fetch(`${base()}/api/auth/login`, {
      method: 'POST',
      headers: { 'content-type': 'application/x-www-form-urlencoded' },
      body: new URLSearchParams({ username: 'carol', password: 'password-789' }).toString(),
    })
    expect(response.status).toBe(200)
    expect(response.headers.get('content-type')).toContain('text/html')
    const html = await response.text()
    expect(html).toMatch(/class="token">[0-9a-f]{64}</)
    expect(html).toContain('命名空间 <code>U-carol</code>')
    // GUI origin known (web gate mounted): the page hands the token across
    // origins through its /api/auth/accept form.
    expect(html).toContain(`action="http://127.0.0.1:${ctx.webServer.port}/api/auth/accept"`)
  })

  it('renders register failure pages with the service reason', async () => {
    const taken = await fetch(`${base()}/api/auth/register`, {
      method: 'POST',
      headers: { 'content-type': 'application/x-www-form-urlencoded' },
      body: new URLSearchParams({ username: 'carol', password: 'password-789' }).toString(),
    })
    expect(taken.status).toBe(400)
    const takenHtml = await taken.text()
    expect(takenHtml).toContain('class="banner error"')
    expect(takenHtml).toContain('username already taken')
    expect(takenHtml).toContain('action="/api/auth/register"') // form re-rendered for retry

    const charset = await fetch(`${base()}/api/auth/register`, {
      method: 'POST',
      headers: { 'content-type': 'application/x-www-form-urlencoded' },
      body: new URLSearchParams({ username: 'Bad Name', password: 'password-789' }).toString(),
    })
    expect(charset.status).toBe(400)
    expect(await charset.text()).toContain('username must match')
  })

  it('serves the workspace list to an authenticated GET and rejects anonymous writes', async () => {
    const { token } = await registerUser('ws-read')
    const list = await fetch(`${base()}/api/workspace`, { headers: { authorization: `Bearer ${token}` } })
    expect(list.status).toBe(200)
    expect(await list.json()).toMatchObject({
      mode: 'standard',
      workspaces: [{ namespace: 'U-ws-read' }],
    })

    const anonymous = await fetch(`${base()}/api/workspace`, {
      method: 'POST',
      headers: { 'content-type': 'application/json' },
      body: JSON.stringify({ namespace: 'ProjectX' }),
    })
    expect(anonymous.status).toBe(401)
  })

  it('rejects non-string workspace payloads on the JSON channel', async () => {
    const { token } = await registerUser('ws-type')
    const response = await fetch(`${base()}/api/workspace`, {
      method: 'POST',
      headers: { 'content-type': 'application/json', authorization: `Bearer ${token}` },
      body: JSON.stringify({ namespace: 42 }),
    })
    expect(response.status).toBe(400)
    expect(await jsonError(response)).toContain('disabled')
  })
})

describe('portal / accept / bind identity handoff (real server)', () => {
  const base = (): string => `http://127.0.0.1:${port}`
  let token: string
  let cookie: string

  beforeAll(async () => {
    ({ token, cookie } = await registerUser('handoff'))
    ctx.set('connection', connectionStub)
  })

  it('redirects an authenticated portal visit to the connection launch URL', async () => {
    const response = await fetch(`${base()}/api/auth/portal`, { headers: { cookie }, redirect: 'manual' })
    expect(response.status).toBe(302)
    expect(response.headers.get('location')).toBe(`http://127.0.0.1:${port}/?launch=stub-token`)
  })

  it('bounces an anonymous portal visit to /login', async () => {
    const response = await fetch(`${base()}/api/auth/portal`, { redirect: 'manual' })
    expect(response.status).toBe(302)
    expect(response.headers.get('location')).toBe('/login')
  })

  it('re-attaches the GUI port when the Host header carries none', async () => {
    const response = await rawRequest({ port, path: '/api/auth/portal', headers: { cookie, host: '127.0.0.1' } })
    expect(response.status).toBe(302)
    expect(response.headers.location).toBe(`http://127.0.0.1:${ctx.webServer.port}/?launch=stub-token`)
  })

  it('falls back to /workspace when no portal origin can be derived', async () => {
    // authenticatedUrl rejects this origin…
    const throwing = await rawRequest({ port, path: '/api/auth/portal', headers: { cookie, host: 'throw.test' } })
    expect(throwing.status).toBe(302)
    expect(throwing.headers.location).toBe('/workspace')

    // …and a request without a Host header has no origin at all.
    const hostless = await hostlessRequest(port, '/api/auth/portal', { headers: { cookie } })
    expect(hostless.status).toBe(302)
    expect(hostless.head).toContain('location: /workspace')

    // An absent/foreign connection service (shape mismatch) is the same story.
    for (const missing of [null, 42, {}]) {
      ctx.set('connection', missing)
      const response = await fetch(`${base()}/api/auth/portal`, { headers: { cookie }, redirect: 'manual' })
      expect(response.status).toBe(302)
      expect(response.headers.get('location')).toBe('/workspace')
    }
    ctx.set('connection', connectionStub)
    expect((await fetch(`${base()}/api/auth/portal`, { headers: { cookie }, redirect: 'manual' })).headers.get('location'))
      .toBe(`http://127.0.0.1:${port}/?launch=stub-token`)
  })

  it('accepts a cross-origin token handoff, setting cookies on this origin', async () => {
    const response = await fetch(`${base()}/api/auth/accept`, {
      method: 'POST',
      headers: { 'content-type': 'application/x-www-form-urlencoded' },
      body: new URLSearchParams({ token }).toString(),
      redirect: 'manual',
    })
    expect(response.status).toBe(302)
    expect(response.headers.get('location')).toBe(`http://127.0.0.1:${port}/?launch=stub-token`)
    const cookies = response.headers.getSetCookie()
    expect(cookies.some(c => c.startsWith(`alioth_session=${token}`) && c.includes('HttpOnly'))).toBe(true)
    expect(cookies.some(c => c.startsWith('alioth_user=handoff'))).toBe(true)
  })

  it('rejects an unknown or missing handoff token with a /login bounce', async () => {
    const unknown = await fetch(`${base()}/api/auth/accept`, {
      method: 'POST',
      headers: { 'content-type': 'application/x-www-form-urlencoded' },
      body: new URLSearchParams({ token: 'deadbeef'.repeat(8) }).toString(),
      redirect: 'manual',
    })
    expect(unknown.status).toBe(302)
    expect(unknown.headers.get('location')).toBe('/login')

    const missing = await fetch(`${base()}/api/auth/accept`, {
      method: 'POST',
      headers: { 'content-type': 'application/json' },
      body: '{}',
      redirect: 'manual',
    })
    expect(missing.status).toBe(302)
    expect(missing.headers.get('location')).toBe('/login')
  })

  it('stays on /workspace for a handoff without a Host header', async () => {
    const response = await hostlessRequest(port, '/api/auth/accept', {
      method: 'POST',
      headers: { 'content-type': 'application/x-www-form-urlencoded' },
      body: new URLSearchParams({ token }).toString(),
    })
    expect(response.status).toBe(302)
    expect(response.head).toContain('location: /workspace')
    expect(response.head).toContain('alioth_session=')
  })

  it('binds agent sessions from an explicit token, a cookie, or refuses', async () => {
    const explicit = await fetch(`${base()}/api/auth/bind`, {
      method: 'POST',
      headers: { 'content-type': 'application/json' },
      body: JSON.stringify({ token, sessionId: 'handoff-session-1' }),
    })
    expect(explicit.status).toBe(204)
    expect(await ctx.aliothAuth.userForSessionId('handoff-session-1')).toMatchObject({ namespace: 'U-handoff' })

    // An empty token field falls back to the same-origin session cookie.
    const viaCookie = await fetch(`${base()}/api/auth/bind`, {
      method: 'POST',
      headers: { 'content-type': 'application/json', cookie },
      body: JSON.stringify({ token: '', sessionId: 'handoff-session-2' }),
    })
    expect(viaCookie.status).toBe(204)
    expect(await ctx.aliothAuth.userForSessionId('handoff-session-2')).toMatchObject({ namespace: 'U-handoff' })

    const anonymous = await fetch(`${base()}/api/auth/bind`, {
      method: 'POST',
      headers: { 'content-type': 'application/json' },
      body: JSON.stringify({ sessionId: 'handoff-session-3' }),
    })
    expect(anonymous.status).toBe(401)
    expect(await ctx.aliothAuth.userForSessionId('handoff-session-3')).toBeNull()

    const badSessionId = await fetch(`${base()}/api/auth/bind`, {
      method: 'POST',
      headers: { 'content-type': 'application/json', authorization: `Bearer ${token}` },
      body: JSON.stringify({ sessionId: 42 }),
    })
    expect(badSessionId.status).toBe(401)
  })

  it('registers the host-login account resolver: cookie → namespace, else null', async () => {
    const resolve = await waitForAccountResolver()
    expect(await resolve({})).toBeNull()
    expect(await resolve({ cookie: 'other=1' })).toBeNull()
    expect(await resolve({ cookie: `alioth_session=${token}` })).toBe('U-handoff')
    expect(await resolve({ cookie: 'alioth_session=deadbeef' })).toBeNull()
  })
})

describe('workspace page with apps and 成品预览 (standard view)', () => {
  const base = (): string => `http://127.0.0.1:${port}`
  let cookie: string

  beforeAll(async () => {
    ({ cookie } = await registerUser('dana'))
    const nsRoot = path.join(previewPreProcRoot, 'U-dana')
    // Two apps: one declaring code+name, one falling back to the dir name.
    await mkdir(path.join(nsRoot, 'Apps', 'app-x'), { recursive: true })
    await writeFile(path.join(nsRoot, 'Apps', 'app-x', 'app.json'), JSON.stringify({ code: 'APPX', name: '示例应用' }))
    await mkdir(path.join(nsRoot, 'Apps', 'app-y'), { recursive: true })
    // Prototype builds: served from the content root (Pre-Proc/<ns>/…) …
    const served = path.join(nsRoot, 'Prototypes', 'Apps', 'demo')
    await mkdir(served, { recursive: true })
    await writeFile(path.join(served, 'a-v2.html'), '<html>v2</html>')
    await writeFile(path.join(served, 'a-v10.html'), '<html>v10</html>')
    // …and discovered relative to that same content root by the browser.
    const discovered = path.join(dataRoot, 'U-dana', 'Prototypes', 'Apps', 'demo')
    await mkdir(discovered, { recursive: true })
    await writeFile(path.join(discovered, 'a-v2.html'), '<html>v2</html>')
    await writeFile(path.join(discovered, 'a-v10.html'), '<html>v10</html>')
  })

  it('lists the apps (named + unnamed) and the latest-first 成品预览 builds', async () => {
    const response = await fetch(`${base()}/workspace`, { headers: { cookie } })
    expect(response.status).toBe(200)
    const html = await response.text()
    expect(html).toContain('<h1>应用</h1>')
    expect(html).toContain('<span class="code">APPX</span> — 示例应用')
    expect(html).toContain('<span class="code">app-y</span></li>') // no name → no suffix
    expect(html).toContain('成品预览')
    expect(html).toContain('demo · a-v10.html')
    expect(html).toContain('（1 KB）')

    // The listed build link actually serves (builds are under the content root).
    const href = /href="(\/preview\/Pre-Proc\/U-dana\/Prototypes\/Apps\/demo\/a-v10\.html)"/.exec(html)?.[1]
    expect(href).toBeDefined()
    const build = await fetch(`${base()}${href}`, { headers: { cookie } })
    expect(build.status).toBe(200)
    expect(await build.text()).toContain('v10')
  })

  it('has no custom-workspace chrome in standard mode (error param included)', async () => {
    const clean = await fetch(`${base()}/workspace`, { headers: { cookie } })
    const cleanHtml = await clean.text()
    expect(cleanHtml).not.toContain('class="banner error"')
    expect(cleanHtml).not.toContain('新建自定义工作区')

    // `?error=` feeds the custom-workspace form banner — absent in 应用 view.
    const withError = await fetch(`${base()}/workspace?error=${encodeURIComponent('boom')}`, { headers: { cookie } })
    const errorHtml = await withError.text()
    expect(withError.status).toBe(200)
    expect(errorHtml).not.toContain('class="banner error"')
    expect(errorHtml).not.toContain('boom')
  })
})

describe('preview surface edges', () => {
  const base = (): string => `http://127.0.0.1:${port}`

  it('serves unknown file types as octet-stream and never lists directories', async () => {
    const { cookie } = await registerUser('preview-edges')
    const demo = path.join(previewPreProcRoot, 'U-preview-edges', 'Prototypes', 'Apps', 'demo')
    await mkdir(demo, { recursive: true })
    await writeFile(path.join(demo, 'blob.bin'), Buffer.from([0, 1, 2]))

    const bin = await fetch(`${base()}/preview/Pre-Proc/U-preview-edges/Prototypes/Apps/demo/blob.bin`, { headers: { cookie } })
    expect(bin.status).toBe(200)
    expect(bin.headers.get('content-type')).toBe('application/octet-stream')

    // Directories are never listed…
    expect((await fetch(`${base()}/preview/Pre-Proc/U-preview-edges/Prototypes/Apps/demo`, { headers: { cookie } })).status).toBe(404)
    // …and neither the bare Pre-Proc root nor the .agents root itself is servable.
    expect((await fetch(`${base()}/preview/Pre-Proc`, { headers: { cookie } })).status).toBe(404)
    expect((await fetch(`${base()}/preview/.agents`, { headers: { cookie } })).status).toBe(404)
  })
})

describe('listPrototypeBuilds (exported listing contract)', () => {
  it('lists a-v{N} builds per app, newest version first, skipping non-builds', async () => {
    const root = await mkdtemp(path.join(tmpdir(), 'authweb-builds-'))
    const appsDir = path.join(root, 'Ns-1', 'Prototypes', 'Apps')
    await mkdir(path.join(appsDir, 'demo'), { recursive: true })
    await writeFile(path.join(appsDir, 'demo', 'a-v1.html'), 'x')
    await writeFile(path.join(appsDir, 'demo', 'a-v10.html'), 'x'.repeat(2048))
    await writeFile(path.join(appsDir, 'demo', 'a-v2.html'), 'x')
    await writeFile(path.join(appsDir, 'demo', 'index.html'), 'not a build')
    await writeFile(path.join(appsDir, 'demo', 'notes.txt'), 'not a build')
    await writeFile(path.join(appsDir, 'stray.txt'), 'not an app dir')
    await mkdir(path.join(appsDir, 'other'), { recursive: true })
    await writeFile(path.join(appsDir, 'other', 'a-v1.html'), 'y')

    const builds = await authWeb.listPrototypeBuilds(root, 'Ns-1')
    expect(builds.map(build => `${build.app}/${build.file}`)).toEqual([
      'demo/a-v10.html', 'demo/a-v2.html', 'demo/a-v1.html', 'other/a-v1.html',
    ])
    expect(builds[0]).toMatchObject({
      namespace: 'Ns-1',
      size: 2048,
      href: '/preview/Pre-Proc/Ns-1/Prototypes/Apps/demo/a-v10.html',
    })
    expect(builds[0]!.mtimeMs).toBeGreaterThan(0)
  })

  it('percent-encodes namespaces and app dirs in hrefs', async () => {
    const root = await mkdtemp(path.join(tmpdir(), 'authweb-builds-'))
    await mkdir(path.join(root, 'Ns 2', 'Prototypes', 'Apps', 'my app'), { recursive: true })
    await writeFile(path.join(root, 'Ns 2', 'Prototypes', 'Apps', 'my app', 'a-v1.html'), 'z')

    const builds = await authWeb.listPrototypeBuilds(root, 'Ns 2')
    expect(builds).toHaveLength(1)
    expect(builds[0]!.href).toBe('/preview/Pre-Proc/Ns%202/Prototypes/Apps/my%20app/a-v1.html')
  })

  it('returns [] for a namespace without a Prototypes/Apps tree', async () => {
    const root = await mkdtemp(path.join(tmpdir(), 'authweb-builds-'))
    expect(await authWeb.listPrototypeBuilds(root, 'Absent')).toEqual([])
  })
})

describe('web gate form register (same-origin redirect)', () => {
  const webBase = (): string => `http://127.0.0.1:${ctx.webServer.port}`

  beforeAll(() => {
    ctx.set('connection', connectionStub)
  })

  it('redirects a form register on the GUI origin to the console portal with cookies', async () => {
    const response = await fetch(`${webBase()}/api/auth/register`, {
      method: 'POST',
      headers: { 'content-type': 'application/x-www-form-urlencoded' },
      body: new URLSearchParams({ username: 'gate-reg', password: 'password-789' }).toString(),
      redirect: 'manual',
    })
    expect(response.status).toBe(302)
    expect(response.headers.get('location')).toBe(`http://127.0.0.1:${ctx.webServer.port}/?launch=stub-token`)
    const cookies = response.headers.getSetCookie()
    expect(cookies.some(c => c.startsWith('alioth_session=') && c.includes('HttpOnly'))).toBe(true)
    expect(cookies.some(c => c.startsWith('alioth_user=gate-reg'))).toBe(true)
  })

  it('falls back to /workspace for a form register when the portal is unavailable', async () => {
    ctx.set('connection', {})
    try {
      const response = await fetch(`${webBase()}/api/auth/register`, {
        method: 'POST',
        headers: { 'content-type': 'application/x-www-form-urlencoded' },
        body: new URLSearchParams({ username: 'gate-reg-2', password: 'password-789' }).toString(),
        redirect: 'manual',
      })
      expect(response.status).toBe(302)
      expect(response.headers.get('location')).toBe('/workspace')
      expect(response.headers.get('set-cookie') ?? '').toContain('alioth_user=gate-reg-2')
    } finally {
      ctx.set('connection', connectionStub)
    }
  })

  it('serves the workspace browser on the GUI origin (cookie session)', async () => {
    const anonymous = await fetch(`${webBase()}/workspace`, { redirect: 'manual' })
    expect(anonymous.status).toBe(302)
    expect(anonymous.headers.get('location')).toBe('/login')

    const login = await fetch(`${webBase()}/api/auth/login`, {
      method: 'POST',
      headers: { 'content-type': 'application/json' },
      body: JSON.stringify({ username: 'carol', password: 'password-789' }),
    })
    const { token } = await login.json() as { token: string }
    const page = await fetch(`${webBase()}/workspace`, { headers: { authorization: `Bearer ${token}` } })
    expect(page.status).toBe(200)
    expect(await page.text()).toContain('<h1>应用</h1>')

    // The ?error= feed of the mounted page is read per request.
    const withError = await fetch(`${webBase()}/workspace?error=nope`, { headers: { cookie: `alioth_session=${token}` } })
    expect(withError.status).toBe(200)
  })

  it('falls back to /workspace when the console portal is unavailable', async () => {
    ctx.set('connection', {})
    try {
      const response = await fetch(`${webBase()}/api/auth/login`, {
        method: 'POST',
        headers: { 'content-type': 'application/x-www-form-urlencoded' },
        body: new URLSearchParams({ username: 'carol', password: 'password-789' }).toString(),
        redirect: 'manual',
      })
      expect(response.status).toBe(302)
      expect(response.headers.get('location')).toBe('/workspace')
      // Cookies still land on the caller's origin.
      expect(response.headers.get('set-cookie') ?? '').toContain('alioth_user')
    } finally {
      ctx.set('connection', connectionStub)
    }
  })
})

describe('client face artifact', () => {
  it('ships a valid client module (shell.overlay user chip)', async () => {
    // Hand-authored closure-factory (no build step) — guard its contract:
    // correct module id, react as the only platform require, inject+apply
    // exports, registration into shell.overlay with a stable entry id.
    const source = await readFile(new URL('../lib/client.js', import.meta.url), 'utf8')
    let registration: { id: string; factory: (require: (name: string) => unknown) => Record<string, unknown> } | undefined
    const fakeWindow = { __ModuleLoader__: { load: (r: typeof registration) => { registration = r } } }
    new Function('window', source)(fakeWindow)
    expect(registration?.id).toBe('@dsh-alioth/auth-web-alioth')

    const reactStub = {
      createElement: (...args: unknown[]) => args,
      useState: (value: unknown) => [value, () => {}],
      useEffect: () => {},
    }
    const exports = registration!.factory((name: string) => {
      if (name !== 'react') throw new Error(`unexpected require: ${name}`)
      return reactStub
    })
    expect(exports.inject).toEqual(['slots'])
    expect(typeof exports.apply).toBe('function')

    let injectedKey: string | undefined
    let captured: { options: { name: string; id: string }; component: unknown } | undefined
    const ctxStub = {
      effect: (fn: () => unknown) => { fn() },
      slots: {
        // Registration defers through slots.inject (shell.overlay is declared
        // by ui-layout after plugin apply; direct register races it).
        inject: (key: string, callback: () => unknown) => {
          injectedKey = key
          callback()
          return () => {}
        },
        register: (options: { name: string; id: string }, component: unknown) => {
          captured = { options, component }
          return () => {}
        },
      },
    }
    ;(exports.apply as (c: unknown) => void)(ctxStub)
    expect(injectedKey).toBe('shell.overlay')
    expect(captured?.options).toEqual({ name: 'shell.overlay', id: 'alioth-user-chip' })
    expect(typeof captured?.component).toBe('function')
  })
})
