/**
 * auth-web-alioth — branch coverage for deployments the real AppCreator-tier
 * services never reach: the 工作区 (unlimited workspace browser) presentation,
 * the custom-workspace create flows, the connection/landing/webServer shapes
 * the carrier is written against, and the standalone-server failure paths.
 *
 * The carrier is defined over structural faces, so the stand-ins here mirror
 * exactly the members it reads:
 * - `aliothAuth`: register/login/logout/userForToken/bind/workspaces/
 *   workspaceMode/createWorkspace (the real auth-alioth is standard-mode and
 *   has no super-admin, so the unlimited/admin paths are unreachable with it);
 * - `webServer`: register/tapIndex (real harness service in the sibling spec);
 * - `connection`: authenticatedUrl/registerAccountResolver;
 * - `aliothLanding`: path/html.
 */
import { describe, expect, it, beforeAll, afterAll } from 'vitest'
import { request as httpRequest } from 'node:http'
import { createServer as createNetServer } from 'node:net'
import { Context } from '@deepseek-ai/cordis'
import * as authWeb from '../src/index.ts'

interface StubWorkspace {
  namespace: string
  preProcPath: string
  deployPath: string
  apps: Array<{ code: string; name: string }>
}

/** The carrier's `aliothAuth` face plus the knobs the tests drive. */
interface AuthStandIn {
  service: Record<string, unknown>
  state: {
    mode: 'standard' | 'unlimited'
    workspaces: StubWorkspace[]
    /** Thrown verbatim when set — the carrier must survive non-Error failures. */
    registerFailure: unknown
    createFailure: unknown
    created: string[]
  }
}

interface FakeWebServer {
  routes: Array<{ kind: 'exact' | 'prefix'; path: string }>
  taps: Array<(html: string) => string>
  register(route: { kind: 'exact' | 'prefix'; path: string; handler: unknown }): () => void
  tapIndex(transform: (html: string) => string): () => void
}

/** Stand-in for `ctx.aliothAuth` with the two knobs the real service cannot
 * express: the workspace mode and a service that fails with non-Errors. */
function authStandIn(): AuthStandIn {
  const sessions = new Map<string, string>()
  const passwords = new Map<string, string>()
  const state = {
    mode: 'unlimited' as 'standard' | 'unlimited',
    workspaces: [] as StubWorkspace[],
    /** Thrown verbatim when set — the carrier must survive non-Error failures. */
    registerFailure: null as unknown,
    createFailure: null as unknown,
    created: [] as string[],
  }
  const service: Record<string, unknown> = {
    async register(username: string, password: string) {
      if (state.registerFailure !== null) throw state.registerFailure
      if (!/^[a-z0-9][a-z0-9-]{2,31}$/.test(username)) {
        throw new Error('aliothAuth.register: username must match ^[a-z0-9][a-z0-9-]{2,31}$')
      }
      if (sessions.has(`tok-${username}`)) throw new Error('aliothAuth.register: username already taken')
      sessions.set(`tok-${username}`, username)
      passwords.set(username, password)
      return { token: `tok-${username}`, namespace: `U-${username}`, role: 'user' as const }
    },
    async login(username: string, password: string) {
      if (sessions.get(`tok-${username}`) !== username || passwords.get(username) !== password) {
        throw new Error('aliothAuth.login: invalid credentials')
      }
      return { token: `tok-${username}`, namespace: `U-${username}`, role: 'user' as const }
    },
    async userForToken(token: string | null) {
      if (token === null) return null
      const username = sessions.get(token)
      return username === undefined ? null : { id: `id-${username}`, username, namespace: `U-${username}`, role: 'user' as const }
    },
    async logout(token: string | null) {
      if (token !== null) sessions.delete(token)
    },
    async bind(token: string, sessionId: string) {
      if (sessions.has(token)) sessions.set(`session-${sessionId}`, sessions.get(token)!)
    },
    workspaceMode: () => state.mode,
    async workspaces() {
      return { mode: state.mode, workspaces: state.workspaces }
    },
    async createWorkspace(namespace: string) {
      if (state.createFailure !== null) throw state.createFailure
      if (!/^[A-Z][a-zA-Z0-9-]*$/.test(namespace)) {
        throw new Error(`aliothAuth.createWorkspace: invalid namespace ${JSON.stringify(namespace)} (expected ^[A-Z][a-zA-Z0-9-]*$)`)
      }
      state.created.push(namespace)
      return {
        namespace,
        preProcPath: `/pre/Pre-Proc/${namespace}`,
        deployPath: `/deploy/${namespace}`,
        apps: [],
      }
    },
  }
  return { service, state }
}

function fakeWebServer(): FakeWebServer {
  const routes: FakeWebServer['routes'] = []
  const taps: FakeWebServer['taps'] = []
  return {
    routes,
    taps,
    register(route) {
      routes.push({ kind: route.kind, path: route.path })
      return () => {}
    },
    tapIndex(transform) {
      taps.push(transform)
      return () => {}
    },
  }
}

/** Free TCP port for the standalone carriers under test. */
async function freePort(): Promise<number> {
  const { promise, resolve } = Promise.withResolvers<number>()
  const server = createNetServer()
  server.listen(0, '127.0.0.1', () => {
    const address = server.address()
    const port = typeof address === 'object' && address !== null ? address.port : 0
    server.close(() => resolve(port))
  })
  return promise
}

/** Host-controlled request — fetch refuses to set the Host header. */
function rawRequest(targetPort: number, path: string, headers: Record<string, string>): Promise<{ status: number; headers: Record<string, string | string[] | undefined> }> {
  const { promise, resolve, reject } = Promise.withResolvers<{
    status: number
    headers: Record<string, string | string[] | undefined>
  }>()
  const request = httpRequest({ host: '127.0.0.1', port: targetPort, path, headers }, response => {
    response.resume()
    response.on('end', () => resolve({ status: response.statusCode ?? 0, headers: response.headers }))
  })
  request.on('error', reject)
  request.end()
  return promise
}

function formBody(fields: Record<string, string>): { headers: Record<string, string>; body: string } {
  return {
    headers: { 'content-type': 'application/x-www-form-urlencoded' },
    body: new URLSearchParams(fields).toString(),
  }
}

/** Extract the `{ error }` payload of a failing response (narrowed, no cast). */
async function jsonError(response: Response): Promise<string> {
  const body: unknown = await response.json()
  if (typeof body === 'object' && body !== null && 'error' in body && typeof body.error === 'string') {
    return body.error
  }
  throw new Error(`response has no error field: ${JSON.stringify(body)}`)
}

const disposers: Array<() => Promise<void>> = []
let ctx: Context
let port: number
let gatePort: number
let auth: AuthStandIn
let web: FakeWebServer

beforeAll(async () => {
  ctx = new Context()
  // Stand-ins are mounted before the carrier applies: the web gate registers
  // its account resolver on the spot instead of retrying.
  auth = authStandIn()
  ctx.provide('aliothAuth')
  ctx.set('aliothAuth', auth.service as never)
  web = fakeWebServer()
  ctx.provide('webServer')
  ctx.set('webServer', web as never)
  ctx.provide('connection')
  ctx.set('connection', {
    authenticatedUrl: (base: string) => `${base}/?launch=stub-token`,
    registerAccountResolver: () => () => {},
  } as never)
  // A landing service of the wrong shape: the carrier falls back to /login.
  ctx.provide('aliothLanding')
  ctx.set('aliothLanding', 42 as never)

  port = await freePort()
  gatePort = await freePort()
  // Carrier #1: workspace mode unlimited, preProcRoot unset (defaults to the
  // deployment convention under $HOME), web gate mounted on the fake service.
  const carrier = await ctx.plugin(authWeb, { port })
  disposers.push(() => carrier.dispose())
  // Carrier #2: webGate off, preProcRoot explicitly empty (same default path).
  const second = await ctx.plugin(authWeb, { port: gatePort, webGate: false, preProcRoot: '' })
  disposers.push(() => second.dispose())
}, 60_000)

afterAll(async () => {
  for (const dispose of disposers.reverse()) {
    await dispose().catch(() => {})
  }
})

describe('standalone carrier without a landing service (bogus shape)', () => {
  const base = (): string => `http://127.0.0.1:${port}`

  it('bounces GET / to /login when aliothLanding is not a landing', async () => {
    const response = await fetch(`${base()}/`, { redirect: 'manual' })
    expect(response.status).toBe(302)
    expect(response.headers.get('location')).toBe('/login')
  })

  it('still serves the login/register pages and ignores wrong-shaped landings', async () => {
    expect((await fetch(`${base()}/login`)).status).toBe(200)
    expect((await fetch(`${base()}/register`)).status).toBe(200)

    // A landing object that is missing path/html is not a landing either.
    ctx.set('aliothLanding', {} as never)
    const after = await fetch(`${base()}/`, { redirect: 'manual' })
    expect(after.status).toBe(302)
    expect(after.headers.get('location')).toBe('/login')
  })
})

describe('unlimited workspace browser (工作区)', () => {
  const base = (): string => `http://127.0.0.1:${port}`
  let cookie: string

  beforeAll(async () => {
    auth.state.workspaces = [
      {
        namespace: 'Acme',
        preProcPath: '/data/Pre-Proc/Acme',
        deployPath: '/data/Deploy/Acme',
        apps: [{ code: 'CRM', name: '客户管理' }, { code: 'anon-app', name: '' }],
      },
      { namespace: 'Empty', preProcPath: '/data/Pre-Proc/Empty', deployPath: '/data/Deploy/Empty', apps: [] },
    ]
    const registered = await fetch(`${base()}/api/auth/register`, {
      method: 'POST',
      ...formBody({ username: 'boss', password: 'password-123' }),
    })
    expect(registered.status).toBe(201)
    expect(registered.headers.getSetCookie().some(c => c.startsWith('alioth_user=boss'))).toBe(true)
    cookie = registered.headers.getSetCookie().map(c => c.split(';')[0]).join('; ')
  })

  it('bounces anonymous visitors to /login', async () => {
    const response = await fetch(`${base()}/workspace`, { redirect: 'manual' })
    expect(response.status).toBe(302)
    expect(response.headers.get('location')).toBe('/login')
  })

  it('shows every namespace with its paths, apps, and the create form', async () => {
    const response = await fetch(`${base()}/workspace`, { headers: { cookie } })
    expect(response.status).toBe(200)
    const html = await response.text()
    expect(html).toContain('<h1>工作区</h1>')
    expect(html).toContain('2 个应用')
    expect(html).toContain('<code>Pre-Proc/Acme/</code>')
    expect(html).toContain('<code>Deploy/Acme/</code>')
    expect(html).toContain('<span class="code">CRM</span> — 客户管理')
    expect(html).toContain('<span class="code">anon-app</span></li>') // no name → no suffix
    expect(html).toContain('暂无应用 — 在对话中让 Alioth 助手创建') // Empty has no apps
    expect(html).toContain('新建自定义工作区')
    expect(html).not.toContain('class="banner error"')
  })

  it('renders the create-failure banner (escaped) and the empty list', async () => {
    const failed = await fetch(`${base()}/workspace?error=${encodeURIComponent('<b>boom</b>')}`, { headers: { cookie } })
    const html = await failed.text()
    expect(html).toContain('class="banner error"')
    expect(html).toContain('&lt;b&gt;boom&lt;/b&gt;')
    expect(html).not.toContain('<b>boom</b>')

    auth.state.workspaces = []
    const empty = await fetch(`${base()}/workspace`, { headers: { cookie } })
    expect(await empty.text()).toContain('<p class="dim">（空）</p>')
    auth.state.workspaces = [
      { namespace: 'Acme', preProcPath: '/data/Pre-Proc/Acme', deployPath: '/data/Deploy/Acme', apps: [] },
    ]
  })

  it('creates custom workspaces from the form (302) and reports failures there', async () => {
    const createA = formBody({ namespace: 'ProjectA' })
    const created = await fetch(`${base()}/api/workspace`, {
      method: 'POST',
      headers: { ...createA.headers, cookie },
      body: createA.body,
      redirect: 'manual',
    })
    expect(created.status).toBe(302)
    expect(created.headers.get('location')).toBe('/workspace')
    expect(auth.state.created).toContain('ProjectA')

    const createLower = formBody({ namespace: 'lower' })
    const invalid = await fetch(`${base()}/api/workspace`, {
      method: 'POST',
      headers: { ...createLower.headers, cookie },
      body: createLower.body,
      redirect: 'manual',
    })
    expect(invalid.status).toBe(302)
    expect(decodeURIComponent(String(invalid.headers.get('location')))).toContain('invalid namespace "lower"')

    // A service that fails with a non-Error: the browser still lands back on
    // the workspace page with the reason rendered.
    auth.state.createFailure = 'aliothAuth.createWorkspace: raw boom'
    try {
      const createB = formBody({ namespace: 'ProjectB' })
      const raw = await fetch(`${base()}/api/workspace`, {
        method: 'POST',
        headers: { ...createB.headers, cookie },
        body: createB.body,
        redirect: 'manual',
      })
      expect(raw.status).toBe(302)
      expect(String(raw.headers.get('location'))).toContain(`error=${encodeURIComponent('aliothAuth.createWorkspace: raw boom')}`)
    } finally {
      auth.state.createFailure = null
    }
  })

  it('creates custom workspaces over JSON (201) and reports failures as JSON', async () => {
    const created = await fetch(`${base()}/api/workspace`, {
      method: 'POST',
      headers: { 'content-type': 'application/json', cookie },
      body: JSON.stringify({ namespace: 'ProjectC' }),
    })
    expect(created.status).toBe(201)
    expect(await created.json()).toMatchObject({ namespace: 'ProjectC', preProcPath: '/pre/Pre-Proc/ProjectC' })

    const invalid = await fetch(`${base()}/api/workspace`, {
      method: 'POST',
      headers: { 'content-type': 'application/json', cookie },
      body: JSON.stringify({ namespace: 'lower' }),
    })
    expect(invalid.status).toBe(400)
    expect(await jsonError(invalid)).toContain('invalid namespace')

    auth.state.createFailure = 'raw json boom'
    try {
      const raw = await fetch(`${base()}/api/workspace`, {
        method: 'POST',
        headers: { 'content-type': 'application/json', cookie },
        body: JSON.stringify({ namespace: 'ProjectD' }),
      })
      expect(raw.status).toBe(400)
      expect(await jsonError(raw)).toBe('raw json boom')
    } finally {
      auth.state.createFailure = null
    }
  })

  it('survives a service that fails with a non-Error on the form + JSON auth paths', async () => {
    auth.state.registerFailure = 'aliothAuth.register: raw register boom'
    try {
      const form = await fetch(`${base()}/api/auth/register`, {
        method: 'POST',
        ...formBody({ username: 'rawuser', password: 'password-123' }),
      })
      expect(form.status).toBe(400)
      expect(await form.text()).toContain('raw register boom')

      const json = await fetch(`${base()}/api/auth/register`, {
        method: 'POST',
        headers: { 'content-type': 'application/json' },
        body: JSON.stringify({ username: 'rawuser', password: 'password-123' }),
      })
      expect(json.status).toBe(400)
      expect(await jsonError(json)).toBe('aliothAuth.register: raw register boom')
    } finally {
      auth.state.registerFailure = null
    }
  })

  it('hands a form login to the workspace page itself (no GUI origin)', async () => {
    const response = await fetch(`${base()}/api/auth/login`, {
      method: 'POST',
      ...formBody({ username: 'boss', password: 'password-123' }),
    })
    expect(response.status).toBe(200)
    const html = await response.text()
    expect(html).toContain('登录成功')
    expect(html).toContain('<a href="/">进入工作台</a>')
    expect(html).not.toContain('/api/auth/accept')
  })

  it('derives the console origin from the connection, defaulting the port', async () => {
    // No GUI origin (web gate on a port-less web server) and a Host without a
    // port: the console URL falls back to the conventional 3100.
    const response = await rawRequest(port, '/api/auth/portal', { cookie, host: '127.0.0.1' })
    expect(response.status).toBe(302)
    expect(response.headers.location).toBe('http://127.0.0.1:3100/?launch=stub-token')
  })

  it('serves nothing from an unconfigured preview root', async () => {
    const response = await fetch(`${base()}/preview/Pre-Proc/Acme/Prototypes/Apps/demo/a-v1.html`, { headers: { cookie } })
    expect(response.status).toBe(404)
    const second = await fetch(`http://127.0.0.1:${gatePort}/preview/.agents/skills/x.js`, { headers: { cookie } })
    expect(second.status).toBe(404)
  })
})

describe('web gate mounting contract (fake webServer)', () => {
  it('registers the auth surface + gate on the composing web server', () => {
    expect(web.routes).toEqual([
      { kind: 'exact', path: '/login' },
      { kind: 'exact', path: '/register' },
      { kind: 'exact', path: '/workspace' },
      { kind: 'prefix', path: '/api/auth' },
      { kind: 'exact', path: '/api/workspace' },
      { kind: 'prefix', path: '/preview' },
    ])
  })

  it('taps the index with a gate that targets /login without a landing', () => {
    expect(web.taps).toHaveLength(1)
    const tapped = web.taps[0]!('<html><head></head><body></body></html>')
    expect(tapped).toContain("location.replace('/login')")
    expect(tapped).toContain('alioth_user')
    // Marker-format drift would silently disable the gate script.
    const match = tapped.match(/<script>([\s\S]*?)<\/script>/)
    expect(match).not.toBeNull()
    expect(() => new Function(match![1]!)).not.toThrow()

    // Pages without a head are returned untouched.
    expect(web.taps[0]!('<html><body>no head</body></html>')).toBe('<html><body>no head</body></html>')
  })
})

describe('standalone server failure paths', () => {
  let errorCtx: Context
  const extraDisposers: Array<() => Promise<void>> = []
  const logs: string[] = []

  beforeAll(async () => {
    errorCtx = new Context()
    // Exporter levels accept everything: the carrier's failure paths log warn/error.
    errorCtx.logger.exporter({ levels: { default: 3 }, export: message => { logs.push(JSON.stringify(message)) } })
  })

  afterAll(async () => {
    for (const dispose of extraDisposers.reverse()) {
      await dispose().catch(() => {})
    }
  })

  it('logs instead of crashing when the web server service has the wrong shape', async () => {
    errorCtx.provide('aliothAuth')
    errorCtx.provide('webServer')

    // An object that is not a web server, then a value that is not even an
    // object: both leave the standalone server serving and the gate unmounted.
    for (const bogus of [{}, 'not-a-service']) {
      errorCtx.set('webServer', bogus as never)
      const carrier = await errorCtx.plugin(authWeb, { port: await freePort() })
      extraDisposers.push(() => carrier.dispose())
    }
    await expect.poll(() => logs.filter(line => line.includes('shape mismatch')).length, { timeout: 5_000 }).toBe(2)
    expect(logs.some(line => line.includes('web gate mounted'))).toBe(false)
  })

  it('derives the GUI origin from a web server that reports a port but no host', async () => {
    errorCtx.set('aliothAuth', auth.service as never)
    errorCtx.set('webServer', { register: () => () => {}, tapIndex: () => () => {}, port: 4321 } as never)
    const carrierPort = await freePort()
    const carrier = await errorCtx.plugin(authWeb, { port: carrierPort })
    extraDisposers.push(() => carrier.dispose())

    const registered = await fetch(`http://127.0.0.1:${carrierPort}/api/auth/register`, {
      method: 'POST',
      ...formBody({ username: 'gui-origin', password: 'password-123' }),
    })
    expect(registered.status).toBe(201)

    // The standalone success page hands the token to the derived GUI origin
    // (host defaults to loopback when the web server reports none).
    const response = await fetch(`http://127.0.0.1:${carrierPort}/api/auth/login`, {
      method: 'POST',
      ...formBody({ username: 'gui-origin', password: 'password-123' }),
    })
    expect(response.status).toBe(200)
    expect(await response.text()).toContain('action="http://127.0.0.1:4321/api/auth/accept"')
  }, 30_000)

  it('reports a port conflict through the logger and keeps serving elsewhere', async () => {
    // Wildcard binding collides with the carrier's own 0.0.0.0 socket.
    const blocker = createNetServer()
    const { promise: listening, resolve: onListening } = Promise.withResolvers<void>()
    blocker.listen(0, onListening)
    await listening
    const address = blocker.address()
    const busyPort = typeof address === 'object' && address !== null ? address.port : 0

    const conflicted = await errorCtx.plugin(authWeb, { port: busyPort, webGate: false })
    extraDisposers.push(() => conflicted.dispose())
    await expect.poll(() => logs.filter(line => line.includes('EADDRINUSE')).length, { timeout: 5_000 }).toBe(1)

    // The carrier that could not bind stays inert; a sibling on a free port serves.
    const healthyPort = await freePort()
    const healthy = await errorCtx.plugin(authWeb, { port: healthyPort, webGate: false })
    extraDisposers.push(() => healthy.dispose())
    await expect.poll(
      async () => fetch(`http://127.0.0.1:${healthyPort}/login`).then(response => response.status).catch(() => 0),
      { timeout: 5_000 },
    ).toBe(200)

    const { promise: closed, resolve: onClosed } = Promise.withResolvers<void>()
    blocker.close(() => onClosed())
    await closed
  }, 30_000)
})
