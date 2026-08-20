/**
 * `@dsh-alioth/auth-alioth` — user registration, login, and authorization for
 * B/S deployments of the Alioth plugin group.
 *
 * Model: single shared workspace, namespace-isolated users. Each user owns a
 * namespace (`u-<username>`) inside the shared preProcRoot / registry; all
 * alioth_* tools with a `namespace` parameter are guarded at the
 * `tools/pre-execute` waterfall — a user may only act on their own namespace
 * (admins span all). Credentials and sessions live in `dsh_alioth_auth`, a
 * schema SEPARATE from the registry so `resetRegistry()` never wipes users.
 *
 * HTTP surface (B/S): node:http server with
 *   POST /api/auth/register   {username, password} → {token, namespace, role}
 *   POST /api/auth/login      {username, password} → {token, namespace, role}
 *   POST /api/auth/logout     (Bearer token)
 *   GET  /api/auth/me         (Bearer token) → {username, namespace, role}
 *   GET  /                     minimal login/register page
 *
 * Guard mode: `mode: 'enforce'` requires an authenticated user for every
 * alioth tool call with a namespace argument; `mode: 'open'` (default) keeps
 * headless/unauthenticated deployments working and only checks when the call
 * carries an identity. Bootstrap admin via `ALIOTH_ADMIN_USERNAME` /
 * `ALIOTH_ADMIN_PASSWORD` (created on first ready when set).
 * @module @dsh-alioth/auth-alioth
 */

import { createHash, randomBytes, randomUUID } from 'node:crypto'
import { createServer, type IncomingMessage, type ServerResponse } from 'node:http'
import type { Context } from '@deepseek-ai/cordis'
import z from '@deepseek-ai/schemastery'
import type { ToolExecution } from '@deepseek-ai/dsh-tools'
import { hashPassword, verifyPassword } from './password.ts'
import {
  AUTH_SCHEMA, bindSession, deleteExpiredSessions, deleteSession, ensureAuthSchema,
  insertSession, insertUser, sessionByTokenHash, userById, userByNamespace, userByUsername,
} from './store.ts'

export const name = 'auth-alioth'
export const inject = ['aliothEnv']

export interface Config {
  /** HTTP port for the auth API. */
  readonly port: number
  /** Guard mode: 'open' keeps unauthenticated calls working (headless); 'enforce' rejects them. */
  readonly mode: 'open' | 'enforce'
  /** Session lifetime in seconds; default 7 days. */
  readonly sessionTtlSeconds?: number
  /** Username charset rule: ^[a-z0-9][a-z0-9-]{2,31}$ (namespaces derive from it). */
  readonly usernamePattern?: string
}

export const Config: z<Config> = z.object({
  port: z.number().default(3900),
  mode: z.union(['open', 'enforce'] as const).default('open'),
  sessionTtlSeconds: z.number().default(7 * 24 * 3600),
  usernamePattern: z.string().default('^[a-z0-9][a-z0-9-]{2,31}$'),
})

export interface AliothAuthService {
  register(username: string, password: string): Promise<{ token: string; namespace: string; role: 'admin' | 'user' }>
  login(username: string, password: string): Promise<{ token: string; namespace: string; role: 'admin' | 'user' }>
  userForToken(token: string | null): Promise<{ id: string; username: string; namespace: string; role: 'admin' | 'user' } | null>
  logout(token: string | null): Promise<void>
  authorizeNamespace(exec: ToolExecution, namespace: string): Promise<boolean>
  bind(token: string, sessionId: string): Promise<void>
  userForSessionId(sessionId: string): Promise<{ namespace: string; role: 'admin' | 'user' } | null>
}

declare module '@deepseek-ai/cordis' {
  interface Context {
    aliothAuth: AliothAuthService
  }
}

const USERNAME_RE = /^[a-z0-9][a-z0-9-]{2,31}$/

/** Derive the user's isolated namespace: `U-<username>` — the Alioth
 * namespace contract requires ^[A-Z][a-zA-Z0-9-]*$ (Gateway runtime), so the
 * prefix is uppercase. */
export function namespaceFor(username: string): string {
  return `U-${username}`
}

/** Hash a session token for storage (never store the raw token). */
export function hashToken(token: string): string {
  return createHash('sha256').update(token).digest('hex')
}

/** Parse the request body for both content types the B/S surface uses:
 * application/json (API clients) and application/x-www-form-urlencoded
 * (browser form submissions from the login/register pages). */
function readBody(request: IncomingMessage): Promise<Record<string, unknown>> {
  return new Promise((resolve, reject) => {
    const chunks: Buffer[] = []
    request.on('data', chunk => { chunks.push(Buffer.from(chunk)) })
    request.on('end', () => {
      const raw = Buffer.concat(chunks).toString('utf8')
      const contentType = request.headers['content-type'] ?? ''
      try {
        if (contentType.includes('application/json')) {
          resolve(JSON.parse(raw || '{}') as Record<string, unknown>)
        } else if (contentType.includes('application/x-www-form-urlencoded')) {
          const params = new URLSearchParams(raw)
          const body: Record<string, unknown> = {}
          for (const [key, value] of params.entries()) {
            body[key] = value
          }
          resolve(body)
        } else {
          resolve({})
        }
      } catch (error) {
        reject(new Error(`invalid request body: ${error instanceof Error ? error.message : String(error)}`))
      }
    })
    request.on('error', reject)
  })
}

function sendJson(response: ServerResponse, status: number, body: unknown): void {
  const payload = JSON.stringify(body)
  response.writeHead(status, {
    'content-type': 'application/json; charset=utf-8',
    'content-length': Buffer.byteLength(payload),
  })
  response.end(payload)
}

function bearerToken(request: IncomingMessage): string | null {
  const header = request.headers.authorization
  if (header === undefined) {
    return null
  }
  const match = /^Bearer\s+(.+)$/i.exec(header)
  return match === null ? null : match[1]!
}

function sendPage(response: ServerResponse, title: string, form: string): void {
  response.writeHead(200, { 'content-type': 'text/html; charset=utf-8' })
  response.end(`<!doctype html><html lang="zh"><head><meta charset="utf-8">
<title>${title}</title></head><body style="font-family:system-ui;max-width:24rem;margin:4rem auto">
<h1>Alioth B/S</h1>${form}</body></html>`)
}

function loginForm(extra: string): string {
  return `<form method="post" action="/api/auth/login" style="display:grid;gap:0.5rem">
<label>用户名 <input name="username" required></label>
<label>密码 <input name="password" type="password" required></label>
<button>登录</button></form><p>${extra}</p>
<p><a href="/register">注册</a></p>`
}

export function apply(ctx: Context, config: Config): void {
  // Deployment override: ALIOTH_AUTH_MODE=enforce turns on mandatory
  // authentication for namespace-scoped tools (B/S production); headless
  // deployments stay open unless asked.
  const effectiveMode: Config['mode'] = process.env.ALIOTH_AUTH_MODE === 'enforce' ? 'enforce' : config.mode
  const ttlSeconds = config.sessionTtlSeconds ?? 7 * 24 * 3600

  // ── service: ctx.aliothAuth ────────────────────────────────────────────
  const aliothAuth = {
    /** Register a new user; returns the raw session token (shown once). */
    async register(username: string, password: string): Promise<{ token: string; namespace: string; role: 'admin' | 'user' }> {
      if (!USERNAME_RE.test(username)) {
        throw new Error(`aliothAuth.register: username must match ${config.usernamePattern}`)
      }
      if (password.length < 8) {
        throw new Error('aliothAuth.register: password must be at least 8 characters')
      }
      const existing = await userByUsername(ctx, username)
      if (existing !== null) {
        throw new Error('aliothAuth.register: username already taken')
      }
      const namespace = namespaceFor(username)
      const occupied = await userByNamespace(ctx, namespace)
      if (occupied !== null) {
        throw new Error(`aliothAuth.register: namespace ${namespace} already allocated`)
      }
      // First registered user bootstraps as admin (single-tenant start);
      // subsequent users are plain users.
      const count = await ctx.aliothEnv.sql<{ count: string }>(`SELECT count(*) AS count FROM ${AUTH_SCHEMA}.users`)
      const user = {
        id: randomUUID(),
        username,
        passwordHash: await hashPassword(password),
        namespace,
        role: (Number(count.rows[0]?.count ?? 0) === 0 ? 'admin' : 'user') as 'admin' | 'user',
      }
      await insertUser(ctx, user)
      const token = randomBytes(32).toString('hex')
      const expiresAt = new Date(Date.now() + ttlSeconds * 1000)
      await insertSession(ctx, { tokenHash: hashToken(token), userId: user.id, sessionId: null, expiresAt })
      return { token, namespace, role: user.role }
    },

    /** Log in; returns a fresh session token. */
    async login(username: string, password: string): Promise<{ token: string; namespace: string; role: 'admin' | 'user' }> {
      const user = await userByUsername(ctx, username)
      if (user === null || !(await verifyPassword(password, user.passwordHash))) {
        throw new Error('aliothAuth.login: invalid credentials')
      }
      const token = randomBytes(32).toString('hex')
      const expiresAt = new Date(Date.now() + ttlSeconds * 1000)
      await insertSession(ctx, { tokenHash: hashToken(token), userId: user.id, sessionId: null, expiresAt })
      return { token, namespace: user.namespace, role: user.role }
    },

    /** Resolve the authenticated user for a bearer token; null when absent/expired. */
    async userForToken(token: string | null): Promise<{ id: string; username: string; namespace: string; role: 'admin' | 'user' } | null> {
      if (token === null) {
        return null
      }
      const session = await sessionByTokenHash(ctx, hashToken(token))
      if (session === null || new Date(session.expiresAt).getTime() < Date.now()) {
        return null
      }
      const user = await userById(ctx, session.userId)
      return user === null ? null : { id: user.id, username: user.username, namespace: user.namespace, role: user.role }
    },

    async logout(token: string | null): Promise<void> {
      if (token !== null) {
        await deleteSession(ctx, hashToken(token))
      }
    },

    /**
     * Authorization for a tool execution: the caller (resolved via the
     * execution's session binding or the configured identity source) may only
     * act on their own namespace; admins span all. Returns the granted
     * namespace or null when the call must be denied.
     */
    async authorizeNamespace(exec: ToolExecution, namespace: string): Promise<boolean> {
      void exec
      // The identity carrier for model-driven calls: the harness exposes the
      // agent's SessionId on the execution; deployments bind it to a user via
      // bindSession. Direct HTTP-driven calls carry their own token path.
      const agentId = exec.agent?.id
      if (agentId === undefined) {
        return effectiveMode !== 'enforce'
      }
      // agent.id is a SessionId; find the user bound to that session.
      const user = await this.userForSessionId(String(agentId))
      if (user === null) {
        return effectiveMode !== 'enforce'
      }
      return user.role === 'admin' || user.namespace === namespace
    },

    /** Bind a user's session token to a dsh agent session id. */
    async bind(token: string, sessionId: string): Promise<void> {
      await bindSession(ctx, hashToken(token), sessionId)
    },

    /** User for a bound agent session id (session-bound identity). */
    async userForSessionId(sessionId: string): Promise<{ namespace: string; role: 'admin' | 'user' } | null> {
      const result = await ctx.aliothEnv.sql<{ user_id: string }>(
        `SELECT user_id FROM ${AUTH_SCHEMA}.sessions WHERE session_id = $1 AND expires_at > now() LIMIT 1`,
        [sessionId],
      )
      if (result.rows[0] === undefined) {
        return null
      }
      const user = await userById(ctx, result.rows[0].user_id)
      return user === null ? null : { namespace: user.namespace, role: user.role }
    },
  }
  ctx.provide('aliothAuth', aliothAuth)

  // ── guard: tools/pre-execute ────────────────────────────────────────────
  ctx.on('tools/pre-execute', async (exec, next) => {
    if (!exec.name.startsWith('alioth_')) {
      return next()
    }
    const args = exec.arguments as Record<string, unknown>
    const namespace = typeof args.namespace === 'string' ? args.namespace : undefined
    if (namespace === undefined) {
      return next()
    }
    const allowed = await aliothAuth.authorizeNamespace(exec, namespace)
    if (allowed) {
      return next()
    }
    return { kind: 'deny', reason: `auth-alioth: user is not authorized for namespace ${namespace} (own namespace: ${await ownNamespace(exec) ?? 'none'})` }
  })

  /** The caller's own namespace for a friendlier deny reason. */
  async function ownNamespace(exec: ToolExecution): Promise<string | null> {
    const agentId = exec.agent?.id
    if (agentId === undefined) {
      return null
    }
    const user = await aliothAuth.userForSessionId(String(agentId))
    return user?.namespace ?? null
  }

  // ── HTTP server ────────────────────────────────────────────────────────
  const server = createServer(async (request, response) => {
    const url = new URL(request.url ?? '/', 'http://localhost')
    try {
      if (request.method === 'GET' && url.pathname === '/') {
        sendPage(response, '登录', loginForm(''))
        return
      }
      if (request.method === 'GET' && url.pathname === '/register') {
        sendPage(response, '注册', `<form method="post" action="/api/auth/register" style="display:grid;gap:0.5rem">
<label>用户名（小写字母/数字/连字符，≥3 位）<input name="username" required></label>
<label>密码（≥8 位）<input name="password" type="password" required></label>
<button>注册</button></form><p><a href="/">登录</a></p>`)
        return
      }
      if (request.method === 'POST' && url.pathname === '/api/auth/register') {
        const body = await readBody(request)
        const username = typeof body.username === 'string' ? body.username : ''
        const password = typeof body.password === 'string' ? body.password : ''
        const result = await aliothAuth.register(username, password)
        sendJson(response, 201, result)
        return
      }
      if (request.method === 'POST' && url.pathname === '/api/auth/login') {
        const body = await readBody(request)
        const username = typeof body.username === 'string' ? body.username : ''
        const password = typeof body.password === 'string' ? body.password : ''
        const result = await aliothAuth.login(username, password)
        sendJson(response, 200, result)
        return
      }
      if (request.method === 'POST' && url.pathname === '/api/auth/logout') {
        await aliothAuth.logout(bearerToken(request))
        sendJson(response, 204, null)
        return
      }
      if (request.method === 'GET' && url.pathname === '/api/auth/me') {
        const user = await aliothAuth.userForToken(bearerToken(request))
        if (user === null) {
          sendJson(response, 401, { error: 'unauthorized' })
          return
        }
        sendJson(response, 200, user)
        return
      }
      sendJson(response, 404, { error: 'not found' })
    } catch (error) {
      sendJson(response, 400, { error: error instanceof Error ? error.message : String(error) })
    }
  })

  // ── lifecycle: lazy idempotent init (harness boots without a 'ready'
  //    event — boot() awaits the Loader instead), HTTP listens immediately,
  //    every DB entry awaits the same cached readiness promise.
  let readyPromise: Promise<void> | undefined
  async function ensureReady(): Promise<void> {
    readyPromise ??= (async () => {
      await ensureAuthSchema(ctx)
      await deleteExpiredSessions(ctx)
      const adminUsername = process.env.ALIOTH_ADMIN_USERNAME
      const adminPassword = process.env.ALIOTH_ADMIN_PASSWORD
      if (adminUsername !== undefined && adminPassword !== undefined && adminUsername !== '') {
        const existing = await userByUsername(ctx, adminUsername)
        if (existing === null) {
          const user = {
            id: randomUUID(),
            username: adminUsername,
            passwordHash: await hashPassword(adminPassword),
            namespace: namespaceFor(adminUsername),
            role: 'admin' as const,
          }
          await insertUser(ctx, user)
          ctx.logger.info(`auth-alioth: bootstrap admin ${adminUsername} created (namespace ${user.namespace})`)
        }
      }
    })()
    return readyPromise
  }

  // Route DB entries through readiness.
  const withReady = <A extends unknown[], R>(fn: (...args: A) => Promise<R>): ((...args: A) => Promise<R>) =>
    async (...args: A) => { await ensureReady(); return fn(...args) }
  aliothAuth.register = withReady(aliothAuth.register)
  aliothAuth.login = withReady(aliothAuth.login)
  aliothAuth.userForToken = withReady(aliothAuth.userForToken)
  aliothAuth.logout = withReady(aliothAuth.logout)
  aliothAuth.bind = withReady(aliothAuth.bind)
  aliothAuth.userForSessionId = withReady(aliothAuth.userForSessionId)

  server.once('error', error => {
    ctx.logger.error(`auth-alioth: HTTP server failed: ${error instanceof Error ? error.message : String(error)}`)
  })
  server.listen(config.port)
  ctx.logger.info(`auth-alioth: B/S auth API on :${config.port} (mode ${effectiveMode})`)

  // HTTP server teardown rides the registry effect path (harness plugins
  // return void from apply; effects unwind with the context).
  ctx.effect(() => () => {
    server.close()
  })
}
