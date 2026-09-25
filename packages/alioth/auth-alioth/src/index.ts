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
import { existsSync } from 'node:fs'
import { mkdir, readFile, readdir, rename } from 'node:fs/promises'
import { homedir } from 'node:os'
import path from 'node:path'
import { Context } from '@deepseek-ai/cordis'
import z from '@deepseek-ai/schemastery'
import { defineTool, type ToolExecution, type ToolRunContext } from '@deepseek-ai/dsh-tools'
import { hashPassword, verifyPassword } from './password.ts'
import {
  AUTH_SCHEMA, bindSession, bindSessionToUser, deleteExpiredSessions, deleteSession, ensureAuthSchema,
  userForBoundSession,
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

/** Extract the U-<username> namespace from a Pre-Proc workspace path. */
function namespaceFromWorkspacePath(path: string): string | null {
  const ns = /(?:^|\/)Pre-Proc\/(U-[A-Za-z0-9][A-Za-z0-9-]*)(?:\/|$)/.exec(path)?.[1]
  return ns === undefined ? null : ns
}

/** An app workspace path's namespace and app code (`Pre-Proc/{ns}/Apps/{app}`). */
export interface SessionApp {
  readonly namespace: string
  readonly code: string
  readonly dir: string
}

/**
 * Extract namespace + app code from an app workspace path. Deliberately not
 * restricted to `U-` namespaces: the caller's own namespace is checked against
 * this result by whoever serves a read-only surface, so the matcher stays a
 * plain shape check (any namespace, exactly one Apps level, single segment).
 */
function appFromWorkspacePath(workspacePath: string): { namespace: string; code: string } | null {
  const match = /(?:^|\/)Pre-Proc\/([^/]+)\/Apps\/([^/]+)\/?$/.exec(workspacePath)
  const namespace = match?.[1]
  const code = match?.[2]
  return namespace === undefined || code === undefined ? null : { namespace, code }
}

/** Read-only structural face of the harness workspace registry (absent in non-web trees). */
interface WorkspaceRegistryLike {
  list(): ReadonlyArray<{ path: string; sessionIds: ReadonlyArray<string> }>
}

function workspaceRegistryOf(ctx: Context): WorkspaceRegistryLike | undefined {
  try {
    const value = (ctx.get as (name: string) => unknown).call(ctx, 'workspaceRegistry')
    return typeof value === 'object' && value !== null && typeof (value as WorkspaceRegistryLike).list === 'function'
      ? value as WorkspaceRegistryLike
      : undefined
  } catch {
    return undefined
  }
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

/**
 * App-workspace dir-name rule, identical to the app.json `code` pattern, so a
 * workspace name is always a legal app code. One path segment by construction:
 * a workspace can be renamed inside its level and has no level to move to.
 */
const APP_NAME_PATTERN_RE = /^[a-zA-Z0-9][a-zA-Z0-9-]*$/

/**
 * The app workspace every account starts with. Without it a brand-new account's
 * namespace holds no apps, so the console picker — which offers exactly the
 * namespace's apps — would have nothing to select and the composer would stay
 * unusable until the user created something elsewhere.
 */
export const DEFAULT_APP_WORKSPACE = 'default'

/** One Apps/ entry: the app.json code/name when readable, the dir name otherwise. */
async function readAppEntry(appsRoot: string, dir: string): Promise<WorkspaceApp> {
  try {
    const parsed = JSON.parse(await readFile(path.join(appsRoot, dir, 'app.json'), 'utf8')) as Record<string, unknown>
    return { code: typeof parsed.code === 'string' ? parsed.code : dir, name: typeof parsed.name === 'string' ? parsed.name : '' }
  } catch {
    return { code: dir, name: '' }
  }
}

/** App entries under one namespace's Apps/ dir (tolerant: broken app.json → code only). */
async function listWorkspaceApps(preProcRoot: string, namespace: string): Promise<WorkspaceApp[]> {
  const appsRoot = path.join(preProcRoot, namespace, 'Apps')
  const dirs = await readdir(appsRoot, { withFileTypes: true }).then(entries =>
    entries.filter(entry => entry.isDirectory() && !entry.name.startsWith('.')).map(entry => entry.name)).catch(() => [])
  const apps: WorkspaceApp[] = []
  for (const dir of dirs) {
    apps.push(await readAppEntry(appsRoot, dir))
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
  /**
   * Resolve an account id back to its account. Entitlement surfaces keyed by a
   * human-readable name (the operator's L2 authorization list) need this direction.
   */
  userById(id: string): Promise<{ id: string; username: string; namespace: string; role: 'admin' | 'user' } | null>
  logout(token: string | null): Promise<void>
  authorizeNamespace(exec: ToolExecution, namespace: string): Promise<boolean>
  bind(token: string, sessionId: string): Promise<void>
  userForSessionId(sessionId: string): Promise<{ namespace: string; role: 'admin' | 'user' } | null>
  /**
   * The app workspace a session is scoped to (`Pre-Proc/{ns}/Apps/{app}`), for
   * read-only console surfaces. `null` when the session is not in an app
   * workspace. Callers MUST still check the namespace against their own
   * identity — this answers "where is this session", not "may I look there".
   */
  appForSession(sessionId: string): SessionApp | null
  /** Resolved workspace mode ('standard' | 'unlimited'). */
  workspaceMode(): 'standard' | 'unlimited'
  /** Create the user's namespace workspace dirs (Pre-Proc/{ns}, Deploy/{ns}, Apps/default). Idempotent. */
  ensureWorkspace(namespace: string): Promise<void>
  /**
   * Create a custom workspace (unlimited mode only): validates the namespace
   * (U- prefix is reserved for user workspaces), auto-creates the
   * AliothStudio path structure, returns the workspace view.
   */
  createWorkspace(namespace: string): Promise<WorkspaceView>
  /**
   * Create one app workspace at `Pre-Proc/{ns}/Apps/{name}` (工作区 = 应用).
   * Single-segment names only; refuses an existing name.
   */
  createApp(namespace: string, name: string): Promise<WorkspaceApp>
  /**
   * Rename an app workspace in place (`Apps/{from}` → `Apps/{to}`). Same level,
   * single-segment names — a workspace is renamed, never moved between levels.
   */
  renameApp(namespace: string, from: string, to: string): Promise<WorkspaceApp>
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

/**
 * The signed-in account of the dispatch currently being processed.
 *
 * Loaded dynamically: `@deepseek-ai/dsh-client-connection` is a **web-profile**
 * package. A headless tree must boot without it, so a missing module means "no
 * account in scope", never a failure.
 * @returns the account string, or null outside a dispatch / without the package.
 */
async function connectionAccount(): Promise<string | null> {
  try {
    const mod = await import('@deepseek-ai/dsh-client-connection')
    return mod.currentConnectionAccount()
  } catch {
    return null
  }
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
   * Deploy/{namespace}/ (deployment artifacts), plus the default app
   * workspace so a fresh account can start working immediately. Idempotent;
   * the namespace pattern is the path-traversal safety boundary. Shared by
   * registration, login, admin bootstrap, and the service surface.
   */
  async function ensureWorkspace(namespace: string): Promise<void> {
    if (!NAMESPACE_PATTERN_RE.test(namespace)) {
      throw new Error(`aliothAuth.ensureWorkspace: invalid namespace ${JSON.stringify(namespace)}`)
    }
    await Promise.all([
      mkdir(path.join(preProcRoot, namespace), { recursive: true }),
      mkdir(path.join(deployRoot, namespace), { recursive: true }),
      mkdir(path.join(preProcRoot, namespace, 'Apps', DEFAULT_APP_WORKSPACE), { recursive: true }),
    ])
  }

  /**
   * Absolute dir of one app workspace: exactly one name segment under the
   * namespace's single Apps/ level. Both the namespace and the name are
   * pattern-checked, so no input can escape the level (工作区不能层级移动).
   * @param namespace - The owning `U-<username>` namespace.
   * @param name - The app workspace name (app.json code shape).
   * @returns The absolute app dir.
   */
  function appDir(namespace: string, name: string): string {
    if (!NAMESPACE_PATTERN_RE.test(namespace)) {
      throw new Error(`aliothAuth: invalid namespace ${JSON.stringify(namespace)}`)
    }
    if (!APP_NAME_PATTERN_RE.test(name)) {
      throw new Error(
        `aliothAuth: invalid app name ${JSON.stringify(name)} (expected ^[a-zA-Z0-9][a-zA-Z0-9-]*$`
        + ' — one path segment; a workspace can be renamed but never moved between levels)',
      )
    }
    return path.join(preProcRoot, namespace, 'Apps', name)
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
      // Lane 0: the server-side binding written when the session was created
      // (the only lane that needs no client participation).
      const serverBound = await userForBoundSession(ctx, sessionId)
      if (serverBound !== null) {
        const bound = await userById(ctx, serverBound)
        if (bound !== null) return { namespace: bound.namespace, role: bound.role }
      }
      const result = await ctx.aliothEnv.sql<{ user_id: string }>(
        `SELECT user_id FROM ${AUTH_SCHEMA}.sessions WHERE session_id = $1 AND expires_at > now() LIMIT 1`,
        [sessionId],
      )
      if (result.rows[0] !== undefined) {
        const bound = await userById(ctx, result.rows[0].user_id)
        if (bound !== null) return { namespace: bound.namespace, role: bound.role }
      }
      // Non-invasive fallback: a session created through the workspace flow
      // belongs to the workspace's namespace (AppCreator pickers lock every
      // account to its own U- namespace). Deriving the identity from the
      // session's workspace path removes the need for client-side binding.
      const owner = workspaceRegistryOf(ctx)?.list().find(workspace => workspace.sessionIds.includes(sessionId))
      if (owner === undefined) return null
      const namespace = namespaceFromWorkspacePath(owner.path)
      if (namespace === null) return null
      const user = await userByNamespace(ctx, namespace)
      return user === null ? null : { namespace: user.namespace, role: user.role }
    },

    /** Account for an id (see the interface). */
    async userById(id: string) {
      const user = await userById(ctx, id)
      return user === null ? null : { id: user.id, username: user.username, namespace: user.namespace, role: user.role }
    },

    /** The app workspace a session is scoped to (read-only; see the interface). */
    appForSession(sessionId: string): SessionApp | null {
      if (sessionId.trim() === '') return null
      const owner = workspaceRegistryOf(ctx)?.list().find(workspace => workspace.sessionIds.includes(sessionId))
      if (owner === undefined) return null
      const app = appFromWorkspacePath(owner.path)
      return app === null ? null : { namespace: app.namespace, code: app.code, dir: owner.path }
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
     * Create one app workspace — the app management page (and the directory
     * flow's create affordance) is the calling gesture; the console's
     * workspace entry itself only names the choice. The new directory
     * sits at `Pre-Proc/{ns}/Apps/{name}`, the level the console's picker
     * lists and a session roots at; its app contract artifacts are generated
     * later by the pipeline (this only provisions the workspace). Refuses an
     * existing name: a workspace is created, never silently adopted.
     * @param namespace - The owning `U-<username>` namespace.
     * @param name - New app workspace name (app.json `code` shape).
     * @returns The created entry.
     */
    async createApp(namespace: string, name: string): Promise<WorkspaceApp> {
      const dir = appDir(namespace, name)
      const appsRoot = path.join(preProcRoot, namespace, 'Apps')
      await mkdir(appsRoot, { recursive: true })
      try {
        await mkdir(dir)
      } catch (error) {
        if ((error as NodeJS.ErrnoException).code === 'EEXIST') {
          throw new Error(`aliothAuth.createApp: ${namespace}/Apps/${name} already exists`)
        }
        throw error
      }
      return await readAppEntry(appsRoot, name)
    },

    /**
     * Rename one app workspace in place: `Apps/{from}` → `Apps/{to}`. Both
     * names are single segments of the same level, so the rename cannot move
     * the workspace in the hierarchy (工作区可以改名，但不能进行层级移动). The
     * app's `Prototypes/Apps/` dir follows when it exists, so the tree keeps
     * one app under one name.
     * @param namespace - The owning `U-<username>` namespace.
     * @param from - Current app workspace name.
     * @param to - New app workspace name.
     * @returns The renamed entry (under its new code).
     */
    async renameApp(namespace: string, from: string, to: string): Promise<WorkspaceApp> {
      const appsRoot = path.join(preProcRoot, namespace, 'Apps')
      if (from === to) {
        return await readAppEntry(appsRoot, from)
      }
      const target = appDir(namespace, to)
      // POSIX rename replaces an existing EMPTY target directory, so an
      // existing name has to be refused up front instead of relying on EEXIST.
      if (existsSync(target)) {
        throw new Error(`aliothAuth.renameApp: ${namespace}/Apps/${to} already exists`)
      }
      try {
        await rename(appDir(namespace, from), target)
      } catch (error) {
        const code = (error as NodeJS.ErrnoException).code
        if (code === 'ENOENT') {
          throw new Error(`aliothAuth.renameApp: ${namespace}/Apps/${from} does not exist`)
        }
        if (code === 'EEXIST' || code === 'ENOTEMPTY') {
          throw new Error(`aliothAuth.renameApp: ${namespace}/Apps/${to} already exists`)
        }
        throw error
      }
      const prototypesRoot = path.join(preProcRoot, namespace, 'Prototypes', 'Apps')
      await rename(path.join(prototypesRoot, from), path.join(prototypesRoot, to)).catch(() => undefined)
      return await readAppEntry(appsRoot, to)
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

  /**
   * Bind an agent session to the signed-in account of the dispatch that created it.
   * Runs inside an HTTP dispatch, so the harness's account scope is in effect;
   * failures are logged and swallowed — a binding problem MUST NOT break session
   * creation (identity then simply stays unbound, which the guard reports).
   * @param sessionId - the freshly created agent session.
   */
  async function bindSessionFromConnection(sessionId: string): Promise<boolean> {
    try {
      const account = await connectionAccount()
      if (account === null || account === '' || sessionId === '') {
        return false
      }
      // The account the web gate publishes is the caller's NAMESPACE
      // (auth-web-alioth resolves the HttpOnly cookie to `user.namespace`), so
      // the lookup is by namespace first; a username is accepted as well, so the
      // binding does not depend on which of the two a future resolver hands over.
      const user = (await userByNamespace(ctx, account)) ?? (await userByUsername(ctx, account))
      if (user === null) {
        return false
      }
      await bindSessionToUser(ctx, sessionId, user.id)
      return true
    } catch (error) {
      ctx.logger.warn(`auth-alioth: could not bind session ${sessionId} to its connection account: ${error instanceof Error ? error.message : String(error)}`)
      return false
    }
  }

  // Identity is written server-side at session creation. The browser gate script
  // used to be the only carrier; it tracks a REST path the harness no longer uses,
  // so a lost binding silently degraded to path-derived identity (observed on m2:
  // a session landed in another account's namespace). This listener is the
  // transport-independent carrier; the client script stays as a redundant path.
  ctx.on('session/created', (session) => {
    void bindSessionFromConnection(String(session.id))
  })

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
      const sessionId = String(agent.id)
      let user = await aliothAuth.userForSessionId(sessionId)
      if (user === null && await bindSessionFromConnection(sessionId)) {
        // Backstop for sessions created before the listener could see them
        // (e.g. one created in a dispatch whose scope had already ended).
        user = await aliothAuth.userForSessionId(sessionId)
      }
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
