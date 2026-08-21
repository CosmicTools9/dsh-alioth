/**
 * `@dsh-alioth/auth-alioth` — the auth CAPABILITY for B/S deployments of the
 * Alioth plugin group: the `ctx.aliothAuth` service (register / login /
 * session resolution / namespace authorization / session binding) plus the
 * two enforcement guards. HTTP surfaces live in `auth-web-alioth` (carrier).
 *
 * Model: single shared workspace, namespace-isolated users. Each user owns a
 * namespace (`U-<username>`) inside the shared preProcRoot / registry; the
 * namespace's workspace dirs (`Pre-Proc/{namespace}/`, `Deploy/{namespace}/`
 * — the AliothStudio layout) are created automatically at registration.
 * All alioth_* tools with a `namespace` parameter are guarded at the
 * `tools/pre-execute` waterfall, and in enforce mode every agent step of an
 * unbound session is rejected at `agent/pre-step` (before any model call).
 * Credentials and sessions live in `dsh_alioth_auth`, a schema SEPARATE from
 * the registry so `resetRegistry()` never wipes users.
 *
 * Deployment environment: `ALIOTH_ENV` (production|local) wins, then the
 * `environment` config, then auto-detection — a non-loopback webServer host
 * means production, anything else is local (dev). The B/S surface uses it to
 * decide between the workspace browser (local) and the fixed app view
 * (production).
 *
 * Guard mode: `mode: 'enforce'` requires an authenticated, session-bound
 * identity (deployment override `ALIOTH_AUTH_MODE=enforce` for B/S
 * production); `mode: 'open'` (default) keeps headless/unauthenticated
 * deployments working. Bootstrap admin via `ALIOTH_ADMIN_USERNAME` /
 * `ALIOTH_ADMIN_PASSWORD` (created on first ready when set).
 * @module @dsh-alioth/auth-alioth
 */

import { createHash, randomBytes, randomUUID } from 'node:crypto'
import { mkdir, readFile, readdir } from 'node:fs/promises'
import { homedir } from 'node:os'
import path from 'node:path'
import { Context } from '@deepseek-ai/cordis'
import z from '@deepseek-ai/schemastery'
import type { ToolExecution } from '@deepseek-ai/dsh-tools'
import { hashPassword, verifyPassword } from './password.ts'
import {
  AUTH_SCHEMA, bindSession, deleteExpiredSessions, deleteSession, ensureAuthSchema,
  insertSession, insertUser, sessionByTokenHash, userById, userByNamespace, userByUsername,
} from './store.ts'

export { hashPassword, verifyPassword }

/** Derive the user's isolated namespace: `U-<username>` — the Alioth
 * namespace contract requires ^[A-Z][a-zA-Z0-9-]*$ (Gateway runtime), so the
 * prefix is uppercase. */
export function namespaceFor(username: string): string {
  return `U-${username}`
}

export const name = 'auth-alioth'
export const inject = ['aliothEnv']

export interface Config {
  /** Guard mode: 'open' keeps unauthenticated calls working (headless); 'enforce' rejects them. */
  readonly mode: 'open' | 'enforce'
  /** Session lifetime in seconds; default 7 days. */
  readonly sessionTtlSeconds?: number
  /** Username charset rule: ^[a-z0-9][a-z0-9-]{2,31}$ (namespaces derive from it). */
  readonly usernamePattern?: string
  /**
   * Deployment environment. 'auto' (default): ALIOTH_ENV wins, then the
   * webServer host — loopback is local, anything else production.
   */
  readonly environment?: 'auto' | 'local' | 'production'
  /** Workspace root for app artifacts; default ALIOTH_PRE_PROC_ROOT ?? ~/WorkSpace/AliothStudio/Pre-Proc. */
  readonly preProcRoot?: string
  /** Workspace root for deployment artifacts; default ALIOTH_DEPLOY_ROOT ?? ~/WorkSpace/AliothStudio/Deploy. */
  readonly deployRoot?: string
}

export const Config: z<Config> = z.object({
  mode: z.union(['open', 'enforce'] as const).default('open'),
  sessionTtlSeconds: z.number().default(7 * 24 * 3600),
  usernamePattern: z.string().default('^[a-z0-9][a-z0-9-]{2,31}$'),
  environment: z.union(['auto', 'local', 'production'] as const).default('auto'),
  preProcRoot: z.string(),
  deployRoot: z.string(),
})

/** One app entry inside a workspace. */
export interface WorkspaceApp {
  readonly code: string
  readonly name: string
}

/** One namespace workspace: the AliothStudio layout (`Pre-Proc/{ns}/`, `Deploy/{ns}/`). */
export interface WorkspaceView {
  readonly namespace: string
  readonly preProcPath: string
  readonly deployPath: string
  readonly apps: readonly WorkspaceApp[]
}

/** The B/S workspace browser response: environment decides 工作区 vs 应用 presentation. */
export interface WorkspaceList {
  readonly environment: 'local' | 'production'
  readonly workspaces: readonly WorkspaceView[]
}

/** Alioth namespace contract — also the workspace dir-name safety boundary. */
const NAMESPACE_PATTERN_RE = /^[A-Z][a-zA-Z0-9-]*$/

/** App entries under one namespace's Apps/ dir (tolerant: broken app.json → code only). */
async function listWorkspaceApps(preProcRoot: string, namespace: string): Promise<WorkspaceApp[]> {
  const appsRoot = path.join(preProcRoot, namespace, 'Apps')
  const dirs = await readdir(appsRoot, { withFileTypes: true }).then(entries =>
    entries.filter(entry => entry.isDirectory() && !entry.name.startsWith('.')).map(entry => entry.name)).catch(() => [])
  const apps: WorkspaceApp[] = []
  for (const dir of dirs) {
    try {
      const parsed = JSON.parse(await readFile(path.join(appsRoot, dir, 'app.json'), 'utf8')) as Record<string, unknown>
      apps.push({ code: typeof parsed.code === 'string' ? parsed.code : dir, name: typeof parsed.name === 'string' ? parsed.name : '' })
    } catch {
      apps.push({ code: dir, name: '' })
    }
  }
  apps.sort((a, b) => a.code.localeCompare(b.code))
  return apps
}

/** Default workspace roots (the AliothStudio checkout layout). */
function defaultRoots(): { preProc: string; deploy: string } {
  return {
    preProc: process.env.ALIOTH_PRE_PROC_ROOT ?? path.join(homedir(), 'WorkSpace', 'AliothStudio', 'Pre-Proc'),
    deploy: process.env.ALIOTH_DEPLOY_ROOT ?? path.join(homedir(), 'WorkSpace', 'AliothStudio', 'Deploy'),
  }
}

/**
 * Resolve the deployment environment. Precedence: ALIOTH_ENV > config
 * `environment` > host hint (loopback = local, anything else production;
 * no hint = local, the dev default).
 */
export function resolveEnvironment(
  configured: Config['environment'],
  hostHint?: string,
): 'local' | 'production' {
  if (process.env.ALIOTH_ENV === 'production' || process.env.ALIOTH_ENV === 'local') {
    return process.env.ALIOTH_ENV
  }
  if (configured === 'production' || configured === 'local') {
    return configured
  }
  const host = hostHint ?? '127.0.0.1'
  return host === '127.0.0.1' || host === 'localhost' || host === '::1' ? 'local' : 'production'
}

export interface AliothAuthService {
  register(username: string, password: string): Promise<{ token: string; namespace: string; role: 'admin' | 'user' }>
  login(username: string, password: string): Promise<{ token: string; namespace: string; role: 'admin' | 'user' }>
  userForToken(token: string | null): Promise<{ id: string; username: string; namespace: string; role: 'admin' | 'user' } | null>
  logout(token: string | null): Promise<void>
  authorizeNamespace(exec: ToolExecution, namespace: string): Promise<boolean>
  bind(token: string, sessionId: string): Promise<void>
  userForSessionId(sessionId: string): Promise<{ namespace: string; role: 'admin' | 'user' } | null>
  /** Resolved deployment environment ('local' | 'production'). */
  environment(hostHint?: string): 'local' | 'production'
  /** Create the user's namespace workspace dirs (Pre-Proc/{ns}, Deploy/{ns}). Idempotent. */
  ensureWorkspace(namespace: string): Promise<void>
  /** Workspaces visible to an identity: users see their own, admins span all. */
  workspaces(identity: { namespace: string; role: 'admin' | 'user' }): Promise<WorkspaceList>
}

declare module '@deepseek-ai/cordis' {
  interface Context {
    aliothAuth: AliothAuthService
  }
}

/** Hash a session token for storage (never store the raw token). */
export function hashToken(token: string): string {
  return createHash('sha256').update(token).digest('hex')
}

export function apply(ctx: Context, config: Config): void {
  // Deployment override: ALIOTH_AUTH_MODE=enforce turns on mandatory
  // authentication for namespace-scoped tools (B/S production); headless
  // deployments stay open unless asked.
  const effectiveMode: Config['mode'] = process.env.ALIOTH_AUTH_MODE === 'enforce' ? 'enforce' : config.mode
  const ttlSeconds = config.sessionTtlSeconds ?? 7 * 24 * 3600
  const USERNAME_RE = new RegExp(config.usernamePattern ?? '^[a-z0-9][a-z0-9-]{2,31}$')
  const roots = defaultRoots()
  const preProcRoot = path.resolve(config.preProcRoot ?? roots.preProc)
  const deployRoot = path.resolve(config.deployRoot ?? roots.deploy)

  /**
   * Create the namespace's workspace dirs — the AliothStudio layout the
   * B/S surface promises: Pre-Proc/{namespace}/ (app artifacts) and
   * Deploy/{namespace}/ (deployment artifacts). Idempotent; the namespace
   * pattern is the path-traversal safety boundary. Shared by registration,
   * admin bootstrap, and the service surface.
   */
  async function ensureWorkspace(namespace: string): Promise<void> {
    if (!NAMESPACE_PATTERN_RE.test(namespace)) {
      throw new Error(`aliothAuth.ensureWorkspace: invalid namespace ${JSON.stringify(namespace)}`)
    }
    await Promise.all([
      mkdir(path.join(preProcRoot, namespace), { recursive: true }),
      mkdir(path.join(deployRoot, namespace), { recursive: true }),
    ])
  }

  // ── service: ctx.aliothAuth ────────────────────────────────────────────
  const aliothAuth = {
    /** Register a new user; returns the raw session token (shown once). */
    async register(username: string, password: string): Promise<{ token: string; namespace: string; role: 'admin' | 'user' }> {
      if (!USERNAME_RE.test(username)) {
        throw new Error(`aliothAuth.register: username must match ${config.usernamePattern}`)
      }
      // 新注册密码策略：≥8 位且同时含字母与数字（既有账号不受影响——登录
      // 不校验复杂度，只核对哈希）。
      if (!/^(?=.*[A-Za-z])(?=.*\d).{8,}$/.test(password)) {
        throw new Error('aliothAuth.register: password must be at least 8 characters with at least one letter and one digit')
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
      // 自动为用户创建同名 namespace 工作区（AliothStudio 路径结构）：
      // Pre-Proc/{namespace}/ 与 Deploy/{namespace}/，幂等。
      await ensureWorkspace(user.namespace)
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

    /** Resolved deployment environment; the carrier passes its bind host. */
    environment(hostHint?: string): 'local' | 'production' {
      return resolveEnvironment(config.environment, hostHint)
    },

    /** Workspace dir bootstrap (Pre-Proc/{ns}, Deploy/{ns}); idempotent. */
    ensureWorkspace,

    /**
     * Workspaces visible to an identity: a plain user sees exactly their own
     * namespace; admins span every namespace under the Pre-Proc root. Apps
     * are read from each namespace's Apps/ dir (tolerant of broken files).
     */
    async workspaces(identity: { namespace: string; role: 'admin' | 'user' }): Promise<WorkspaceList> {
      const namespaceDirs = identity.role === 'admin'
        ? await readdir(preProcRoot, { withFileTypes: true }).then(entries =>
          entries.filter(entry => entry.isDirectory() && !entry.name.startsWith('.')).map(entry => entry.name)).catch(() => [])
        : [identity.namespace]
      const workspaces: WorkspaceView[] = []
      for (const namespace of namespaceDirs) {
        if (!NAMESPACE_PATTERN_RE.test(namespace)) {
          continue
        }
        workspaces.push({
          namespace,
          preProcPath: path.join(preProcRoot, namespace),
          deployPath: path.join(deployRoot, namespace),
          apps: await listWorkspaceApps(preProcRoot, namespace),
        })
      }
      workspaces.sort((a, b) => a.namespace.localeCompare(b.namespace))
      return { environment: resolveEnvironment(config.environment), workspaces }
    },
  }

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

  // ── guard: agent/pre-step (enforce mode) ──────────────────────────────
  // The B/S product rule "登录才能用" lands here: the web gate bounces
  // unauthenticated visitors at the UI layer (auth-web-alioth), and this
  // waterfall blocks the agent loop itself (before any model call) when the
  // session carries no bound user identity. Open mode skips it (headless).
  if (effectiveMode === 'enforce') {
    ctx.on('agent/pre-step', async ({ agent }, next) => {
      const user = await aliothAuth.userForSessionId(String(agent.id))
      if (user !== null) {
        return next()
      }
      ctx.logger.warn(`auth-alioth: rejecting agent step — session ${String(agent.id)} is not bound to a user`)
      return { kind: 'reject' }
    })
  }

  ctx.provide('aliothAuth', aliothAuth)

  // ── lifecycle: lazy idempotent init (harness boots without a 'ready'
  //    event — boot() awaits the Loader instead); every DB entry awaits the
  //    same cached readiness promise.
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
          await ensureWorkspace(user.namespace)
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
}
