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
 * Deployment workspace mode: `ALIOTH_WORKSPACE_MODE` (unlimited|standard)
 * wins, then the `workspaceMode` config, default standard. Only 'unlimited'
 * opens the custom workspace browser (every namespace visible to every
 * user); standard fixes the B/S surface to the 应用 view of the user's own
 * namespace. Namespace workspace dirs (Pre-Proc/{ns}, Deploy/{ns}) are
 * created for every user regardless of mode.
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
import { defineTool, type ToolExecution, type ToolRunContext } from '@deepseek-ai/dsh-tools'
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
export const inject = ['aliothEnv', 'tools']

export interface Config {
  /** Guard mode: 'open' keeps unauthenticated calls working (headless); 'enforce' rejects them. */
  readonly mode: 'open' | 'enforce'
  /** Session lifetime in seconds; default 7 days. */
  readonly sessionTtlSeconds?: number
  /** Username charset rule: ^[a-z0-9][a-z0-9-]{2,31}$ (namespaces derive from it). */
  readonly usernamePattern?: string
  /**
   * Workspace mode. 'unlimited' opens 自定义工作区 — the workspace browser
   * shows every namespace (with its Pre-Proc/Deploy paths) to every user;
   * 'standard' (default) fixes everyone to the 应用 view of their own
   * namespace. Env override ALIOTH_WORKSPACE_MODE=unlimited wins.
   */
  readonly workspaceMode?: 'standard' | 'unlimited'
  /** Workspace root for app artifacts; default ALIOTH_PRE_PROC_ROOT ?? ~/.dsh-alioth/Pre-Proc (deployment-owned, never the AliothStudio checkout). */
  readonly preProcRoot?: string
  /** Workspace root for deployment artifacts; default ALIOTH_DEPLOY_ROOT ?? ~/.dsh-alioth/Deploy. */
  readonly deployRoot?: string
}

export const Config: z<Config> = z.object({
  mode: z.union(['open', 'enforce'] as const).default('open'),
  sessionTtlSeconds: z.number().default(7 * 24 * 3600),
  usernamePattern: z.string().default('^[a-z0-9][a-z0-9-]{2,31}$'),
  workspaceMode: z.union(['standard', 'unlimited'] as const).default('standard'),
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

/** The B/S workspace browser response: mode decides 工作区 vs 应用 presentation. */
export interface WorkspaceList {
  readonly mode: 'standard' | 'unlimited'
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

/** Default workspace roots — deployment-owned, decoupled from any
 * AliothStudio checkout (integrate by setting ALIOTH_PRE_PROC_ROOT). */
function defaultRoots(): { preProc: string; deploy: string } {
  return {
    preProc: process.env.ALIOTH_PRE_PROC_ROOT ?? path.join(homedir(), '.dsh-alioth', 'Pre-Proc'),
    deploy: process.env.ALIOTH_DEPLOY_ROOT ?? path.join(homedir(), '.dsh-alioth', 'Deploy'),
  }
}

/**
 * Resolve the workspace mode. Precedence: ALIOTH_WORKSPACE_MODE env >
 * config `workspaceMode` > 'standard'. Only 'unlimited' opens the custom
 * workspace browser; everything else is the fixed 应用 view.
 */
export function resolveWorkspaceMode(_configured?: Config['workspaceMode']): 'standard' | 'unlimited' {
  // AppCreator tier only: multi-namespace unlimited belongs to the AppAgent
  // tier whose entry lives elsewhere — env/config are intentionally ignored.
  return 'standard'
}

export interface AliothAuthService {
  register(username: string, password: string): Promise<{ token: string; namespace: string; role: 'admin' | 'user' }>
  login(username: string, password: string): Promise<{ token: string; namespace: string; role: 'admin' | 'user' }>
  userForToken(token: string | null): Promise<{ id: string; username: string; namespace: string; role: 'admin' | 'user' } | null>
  logout(token: string | null): Promise<void>
  authorizeNamespace(exec: ToolExecution, namespace: string): Promise<boolean>
  bind(token: string, sessionId: string): Promise<void>
  userForSessionId(sessionId: string): Promise<{ namespace: string; role: 'admin' | 'user' } | null>
  /** Resolved workspace mode ('standard' | 'unlimited'). */
  workspaceMode(): 'standard' | 'unlimited'
  /** Create the user's namespace workspace dirs (Pre-Proc/{ns}, Deploy/{ns}). Idempotent. */
  ensureWorkspace(namespace: string): Promise<void>
  /**
   * Create a custom workspace (unlimited mode only): validates the namespace
   * (U- prefix is reserved for user workspaces), auto-creates the
   * AliothStudio path structure, returns the workspace view.
   */
  createWorkspace(namespace: string): Promise<WorkspaceView>
  /** Workspaces visible to an identity: unlimited shows every namespace, standard is role-scoped. */
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
      // AppCreator has no super-admin: every registered user is equal and
      // owns exactly their U-<username> namespace.
      const user = {
        id: randomUUID(),
        username,
        passwordHash: await hashPassword(password),
        namespace,
        role: 'user' as const,
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
      return user.namespace === namespace
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

    /** Resolved workspace mode ('standard' | 'unlimited'). */
    workspaceMode(): 'standard' | 'unlimited' {
      return resolveWorkspaceMode(config.workspaceMode)
    },

    /** Workspace dir bootstrap (Pre-Proc/{ns}, Deploy/{ns}); idempotent. */
    ensureWorkspace,

    /**
     * Create a custom workspace — only in unlimited mode (标准模式禁用
     * 自定义工作区). The namespace must match the Alioth contract; the
     * `U-` prefix is reserved for per-user workspaces. Auto-creates the
     * AliothStudio path structure and returns the fresh workspace view.
     */
    async createWorkspace(namespace: string): Promise<WorkspaceView> {
      if (resolveWorkspaceMode(config.workspaceMode) !== 'unlimited') {
        throw new Error('aliothAuth.createWorkspace: custom workspaces are disabled (workspaceMode=standard)')
      }
      if (!NAMESPACE_PATTERN_RE.test(namespace)) {
        throw new Error(`aliothAuth.createWorkspace: invalid namespace ${JSON.stringify(namespace)} (expected ^[A-Z][a-zA-Z0-9-]*$)`)
      }
      if (namespace.startsWith('U-')) {
        throw new Error(`aliothAuth.createWorkspace: ${namespace} is reserved for user workspaces (U- prefix)`)
      }
      await ensureWorkspace(namespace)
      return {
        namespace,
        preProcPath: path.join(preProcRoot, namespace),
        deployPath: path.join(deployRoot, namespace),
        apps: await listWorkspaceApps(preProcRoot, namespace),
      }
    },

    /**
     * Workspaces visible to an identity. 'unlimited' opens 自定义工作区:
     * every namespace under the Pre-Proc root is shown to everyone (with its
     * paths). 'standard' is role-scoped: a plain user sees exactly their own
     * namespace, admins span all. Apps are read from each namespace's Apps/
     * dir (tolerant of broken files).
     */
    async workspaces(identity: { namespace: string; role: 'admin' | 'user' }): Promise<WorkspaceList> {
      const mode = resolveWorkspaceMode(config.workspaceMode)
      // Lazy backfill: users registered before the workspace feature (or with
      // roots changed since) may lack their dirs — guarantee the caller's own
      // workspace exists on every read (idempotent).
      await ensureWorkspace(identity.namespace)
      // AppCreator: standard locks every account to its own namespace; only
      // unlimited mode (AppAgent-style deployments) lists every namespace.
      const namespaceDirs = mode === 'unlimited'
        ? await readdir(preProcRoot, { withFileTypes: true }).then(entries =>
          entries.filter(entry => entry.isDirectory() && !entry.name.startsWith('.')).map(entry => entry.name)).catch(() => [])
        : [identity.namespace]
      // Orphan filter (standard mode): `U-*` dirs without a matching user row
      // are leftovers from deleted accounts or foreign instances — never show
      // them as workspaces. Unlimited keeps the raw view (operator-controlled).
      const userNamespaces = mode === 'standard'
        ? (await ctx.aliothEnv.sql<{ namespace: string }>(`SELECT namespace FROM ${AUTH_SCHEMA}.users`))
          .rows.map(row => row.namespace)
        : []
      const workspaces: WorkspaceView[] = []
      for (const namespace of namespaceDirs) {
        if (!NAMESPACE_PATTERN_RE.test(namespace)) {
          continue
        }
        if (mode === 'standard' && namespace.startsWith('U-') && !userNamespaces.includes(namespace)) {
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
      return { mode, workspaces }
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
      // No super-admin concept: registration is the only user path (the old
      // ALIOTH_ADMIN_* bootstrap was removed with the role privileges).
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

  // ── model surface: alioth_workspace_current ───────────────────────────
  // The B/S product rule "the model works inside the caller's workspace":
  // every namespace-scoped tool must receive the caller's OWN namespace.
  // The example namespaces in tool descriptions (e.g. "Alioth") would
  // otherwise leak into real artifacts — this tool resolves the identity
  // bound to the session and ensures the path structure exists on first use.
  ctx.tools.register(defineTool({
    name: 'alioth_workspace_current',
    description:
      'Resolve the caller\'s own workspace: the namespace bound to the current session '
      + '(U-<username>), the workspace mode, and the AliothStudio path structure '
      + '(Pre-Proc/{namespace}/, Deploy/{namespace}/ — ensured to exist). Call this FIRST '
      + 'before any alioth_* call that takes a namespace and use the returned namespace '
      + 'verbatim — never guess or invent a namespace.',
    parameters: {},
    output: {
      schema: {
        type: 'object',
        additionalProperties: false,
        properties: {
          namespace: { type: 'string', required: true },
          mode: { type: 'string', required: true, enum: ['standard', 'unlimited'] },
          preProcPath: { type: 'string', required: true },
          deployPath: { type: 'string', required: true },
        },
      },
      render: (_args, value) => [{
        type: 'text',
        text: `Caller workspace: ${value.namespace} (mode ${value.mode}) — Pre-Proc/${value.namespace}/, Deploy/${value.namespace}/`,
      }],
    },
    async execute(_args, exec: ToolRunContext) {
      const agentId = exec.agent?.id
      if (agentId === undefined) {
        throw new Error('alioth_workspace_current: no session identity — log in first')
      }
      const user = await aliothAuth.userForSessionId(String(agentId))
      if (user === null) {
        throw new Error('alioth_workspace_current: session is not bound to a user — log in first')
      }
      await ensureWorkspace(user.namespace)
      return {
        namespace: user.namespace,
        mode: aliothAuth.workspaceMode(),
        preProcPath: path.join(preProcRoot, user.namespace),
        deployPath: path.join(deployRoot, user.namespace),
      }
    },
    presentCall: _args => ({
      card: 'generic',
      title: 'Resolve current workspace',
      kind: 'other',
      rawInput: {},
    }),
  }))
}
