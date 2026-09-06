import { describe, expect, it, beforeAll, afterAll } from 'vitest'
import { mkdtemp, mkdir, rm, writeFile } from 'node:fs/promises'
import { tmpdir } from 'node:os'
import path from 'node:path'
import { Context } from '@deepseek-ai/cordis'
import SystemPrompt from '@deepseek-ai/dsh-system-prompt'
import ToolRuntime from '@deepseek-ai/dsh-tools'
import { ToolCallId } from '@deepseek-ai/dsh-llm'
import * as envAlioth from '@dsh-alioth/env-alioth'
import * as toolAlioth from '@dsh-alioth/tool-alioth'
import * as auth from '../src/index.ts'
import { hashPassword, verifyPassword } from '../src/password.ts'
import { namespaceFor } from '../src/index.ts'
import type { Session, SessionId } from '@deepseek-ai/dsh-session'
import type { Agent } from '@deepseek-ai/dsh-agent'

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
let preProcRoot: string
let deployRoot: string
let counter = 0

function callTool(name: string, args: unknown, agent?: Agent) {
  return ctx.tools.execute({
    signal: new AbortController().signal,
    callId: ToolCallId(`auth-${++counter}`),
    name,
    arguments: args,
    ...(agent === undefined ? {} : { agent }),
  })
}

/** Fake agent carrying a session id (the identity carrier the guard reads). */
function fakeAgent(sessionId: string): Agent {
  const session = { id: sessionId as SessionId } as Session
  return { id: sessionId as SessionId, session } as unknown as Agent
}

beforeAll(async () => {
  const modelDir = await mkdtemp(path.join(tmpdir(), 'auth-model-'))
  const dataRoot = await mkdtemp(path.join(tmpdir(), 'auth-data-'))
  preProcRoot = await mkdtemp(path.join(tmpdir(), 'auth-preproc-'))
  deployRoot = await mkdtemp(path.join(tmpdir(), 'auth-deploy-'))
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
  const system = await ctx.plugin(SystemPrompt)
  disposers.push(() => system.dispose())
  const tools = await ctx.plugin(ToolRuntime)
  disposers.push(() => tools.dispose())
  const env = await ctx.plugin(envAlioth, { modelSource: modelDir, dataRoot })
  disposers.push(() => env.dispose())
  await ctx.aliothEnv.ready()
  const appTool = await ctx.plugin(toolAlioth, { preProcRoot })
  disposers.push(() => appTool.dispose())

  const authPlugin = await ctx.plugin(auth, { mode: 'enforce', preProcRoot, deployRoot })
  disposers.push(() => authPlugin.dispose())
}, 120_000)

afterAll(async () => {
  for (const dispose of disposers.reverse()) {
    await dispose().catch(() => {})
  }
  await rm(preProcRoot, { recursive: true, force: true })
  await rm(deployRoot, { recursive: true, force: true })
})

describe('password', () => {
  it('hashes and verifies (self-describing scrypt)', async () => {
    const encoded = await hashPassword('correct horse battery staple')
    expect(encoded).toMatch(/^scrypt\$16384\$8\$1\$/)
    expect(await verifyPassword('correct horse battery staple', encoded)).toBe(true)
    expect(await verifyPassword('wrong', encoded)).toBe(false)
  })

  it('rejects malformed encodings', async () => {
    expect(await verifyPassword('x', 'not-a-hash')).toBe(false)
    expect(await verifyPassword('x', 'scrypt$1$2$3$4$5$6$7')).toBe(false)
  })
})

describe('auth service', () => {
  it('registers a user and allocates an isolated namespace', async () => {
    const result = await ctx.aliothAuth.register('alice', 'password-123')
    expect(result.namespace).toBe(namespaceFor('alice'))
    expect(result.role).toBe('user') // every registered user is equal (no super-admin)
    expect(result.token).toMatch(/^[0-9a-f]{64}$/)
  })

  it('rejects duplicate usernames and weak passwords', async () => {
    await expect(ctx.aliothAuth.register('alice', 'another-pass-1')).rejects.toThrow(/already taken/)
    await expect(ctx.aliothAuth.register('bob', 'short')).rejects.toThrow(/at least 8/)
    await expect(ctx.aliothAuth.register('bob2', 'password-only')).rejects.toThrow(/at least one letter and one digit/)
    await expect(ctx.aliothAuth.register('bob3', '12345678')).rejects.toThrow(/at least one letter and one digit/)
    await expect(ctx.aliothAuth.register('BOB', 'password-123')).rejects.toThrow(/must match/)
  })

  it('logs in with valid credentials and resolves the user by token', async () => {
    const login = await ctx.aliothAuth.login('alice', 'password-123')
    const user = await ctx.aliothAuth.userForToken(login.token)
    expect(user).toMatchObject({ username: 'alice', namespace: 'U-alice' })
    await expect(ctx.aliothAuth.login('alice', 'wrong-pass')).rejects.toThrow(/invalid credentials/)
  })

  it('expires sessions and logs out', async () => {
    const login = await ctx.aliothAuth.login('alice', 'password-123')
    await ctx.aliothAuth.logout(login.token)
    expect(await ctx.aliothAuth.userForToken(login.token)).toBeNull()
  })
})

describe('namespace authorization guard', () => {
  let bobSession: string
  let aliceToken: string

  beforeAll(async () => {
    // No super-admin: alice and bob are equal users in their own namespaces.
    const bob = await ctx.aliothAuth.register('bob', 'password-456')
    void bob
    const alice = await ctx.aliothAuth.login('alice', 'password-123')
    aliceToken = alice.token
    // Bind bob's token to a fake agent session id (the guard's identity path).
    const bobLogin = await ctx.aliothAuth.login('bob', 'password-456')
    bobSession = 'session-bob-1'
    await ctx.aliothAuth.bind(bobLogin.token, bobSession)
  })

  it('allows a user to write within their own namespace', async () => {
    const agent = fakeAgent(bobSession)
    const result = await callTool('alioth_app_write', {
      namespace: 'U-bob', code: 'my-app', name: 'Bob 应用', modules: [{ id: 'm1', name: 'M1' }],
    }, agent)
    if (result.isError) throw new Error(`expected write success: ${result.error.message}`)
    expect(result.value).toMatchObject({ code: 'my-app' })
  })

  it('denies a user writing into another namespace', async () => {
    const bound = await ctx.aliothAuth.userForSessionId(bobSession)
    if (bound === null) throw new Error('probe: bob session not bound')
    const agent = fakeAgent(bobSession)
    const result = await callTool('alioth_app_write', {
      namespace: 'U-alice', code: 'sneak', name: '偷写', modules: [{ id: 'm1', name: 'M1' }],
    }, agent)
    if (!result.isError) throw new Error('expected denial')
    expect(result.error.message).toContain('not authorized for namespace U-alice')
    expect(result.error.message).toContain('U-bob')
  })

  it('denies reads into another namespace too (inspect)', async () => {
    const agent = fakeAgent(bobSession)
    const result = await callTool('alioth_app_inspect', {
      namespace: 'U-alice', app: 'anything',
    }, agent)
    if (!result.isError) throw new Error('expected denial')
    expect(result.error.message).toContain('not authorized')
  })

  it('rejects writing into another user namespace (no admin escape)', async () => {
    // Equal users: alice's session cannot act inside U-bob.
    await ctx.aliothAuth.bind(aliceToken, 'session-alice-1')
    const agent = fakeAgent('session-alice-1')
    const result = await callTool('alioth_app_write', {
      namespace: 'U-bob', code: 'sneaky-app', name: '越权写入', modules: [{ id: 'm1', name: 'M1' }],
    }, agent)
    if (!result.isError) throw new Error('expected denial for cross-namespace write')
    expect(result.error.message).toContain('not authorized')
  })

  it('rejects unauthenticated calls in enforce mode', async () => {
    const result = await callTool('alioth_app_write', {
      namespace: 'U-alice', code: 'ghost', name: '幽灵', modules: [{ id: 'm1', name: 'M1' }],
    })
    if (!result.isError) throw new Error('expected denial for unauthenticated call')
    expect(result.error.message).toContain('not authorized')
  })

  it('rejects agent steps for unbound sessions, passes bound ones (enforce)', async () => {
    const rejected = await ctx.waterfall('agent/pre-step', {
      agent: fakeAgent('unbound-session'),
      messages: [],
      turn: 1,
      step: 1,
      signal: new AbortController().signal,
    }, async () => ({ kind: 'enter' as const, messages: [] }))
    expect(rejected.kind).toBe('reject')

    const { token } = await ctx.aliothAuth.register('dave', 'password-123')
    await ctx.aliothAuth.bind(token, 'bound-session')
    const passed = await ctx.waterfall('agent/pre-step', {
      agent: fakeAgent('bound-session'),
      messages: [],
      turn: 1,
      step: 1,
      signal: new AbortController().signal,
    }, async () => ({ kind: 'enter' as const, messages: [] }))
    expect(passed.kind).toBe('enter')
  })
})

describe('workspace (namespace = user workspace)', () => {
  it('creates the AliothStudio workspace dirs at registration', async () => {
    // carol is a fresh user; her namespace dirs must exist under both roots.
    const result = await ctx.aliothAuth.register('carol', 'password-123')
    expect(result.namespace).toBe('U-carol')
    const preStat = await import('node:fs/promises').then(fs => fs.stat(path.join(preProcRoot, 'U-carol')))
    const deployStat = await import('node:fs/promises').then(fs => fs.stat(path.join(deployRoot, 'U-carol')))
    expect(preStat.isDirectory()).toBe(true)
    expect(deployStat.isDirectory()).toBe(true)
  })

  it('ensureWorkspace is idempotent and rejects unsafe namespaces', async () => {
    await expect(ctx.aliothAuth.ensureWorkspace('U-carol')).resolves.toBeUndefined()
    await expect(ctx.aliothAuth.ensureWorkspace('../evil')).rejects.toThrow(/invalid namespace/)
    await expect(ctx.aliothAuth.ensureWorkspace('lower')).rejects.toThrow(/invalid namespace/)
  })

  it('derives session identity from the workspace path when no binding row exists', async () => {
    // Non-invasive fallback: a session attached to a workspace under
    // Pre-Proc/U-<name> resolves to that namespace's user without any
    // client-side bind call. alice owns U-alice (registered above).
    const registry = {
      list: () => [{
        // Registry paths are real filesystem paths under Pre-Proc/<ns>/Apps.
        path: path.join(preProcRoot, 'Pre-Proc', 'U-alice', 'Apps', 'demo'),
        sessionIds: ['session-fallback-1'],
      }],
    }
    // The test tree mounts no harness workspace service; provide a stub on
    // the real Context (ctx.get reads it back through the service registry).
    const provide = (ctx as unknown as { provide(name: string, value: unknown): void }).provide
    provide.call(ctx, 'workspaceRegistry', registry)
    try {
      const resolved = await ctx.aliothAuth.userForSessionId('session-fallback-1')
      expect(resolved).toMatchObject({ namespace: 'U-alice' })
      // Sessions without any workspace stay unknown (no binding, no path).
      expect(await ctx.aliothAuth.userForSessionId('session-orphan')).toBeNull()
    } finally {
      const dispose = (ctx as unknown as { disposeService?(name: string): void }).disposeService
      if (typeof dispose === 'function') dispose.call(ctx, 'workspaceRegistry')
    }
  })

  it('always resolves standard (AppCreator tier; unlimited belongs to the AppAgent tier)', async () => {
    expect(ctx.aliothAuth.workspaceMode()).toBe('standard')
    // Env/config are intentionally ignored.
    const saved = process.env.ALIOTH_WORKSPACE_MODE
    try {
      process.env.ALIOTH_WORKSPACE_MODE = 'unlimited'
      expect(ctx.aliothAuth.workspaceMode()).toBe('standard')
    } finally {
      if (saved === undefined) { delete process.env.ALIOTH_WORKSPACE_MODE } else { process.env.ALIOTH_WORKSPACE_MODE = saved }
    }
  })

  it('standard mode rejects custom workspace creation', async () => {
    await expect(ctx.aliothAuth.createWorkspace('ProjectB')).rejects.toThrow(/disabled/)
  })

  it('alioth_workspace_current resolves the bound identity and ensures the path structure', async () => {
    // bob's session is bound in the guard describe.
    const result = await callTool('alioth_workspace_current', {}, fakeAgent('session-bob-1'))
    if (result.isError) throw new Error(`expected workspace_current success: ${result.error.message}`)
    expect(result.value).toMatchObject({
      namespace: 'U-bob',
      mode: 'standard',
      preProcPath: path.join(preProcRoot, 'U-bob'),
      deployPath: path.join(deployRoot, 'U-bob'),
    })

    // unbound session → loud error (the model must not guess a namespace)
    const unbound = await callTool('alioth_workspace_current', {}, fakeAgent('no-such-session'))
    if (!unbound.isError) throw new Error('expected workspace_current failure for unbound session')
    expect(unbound.error.message).toContain('not bound to a user')

    // no agent identity at all → loud error
    const noAgent = await callTool('alioth_workspace_current', {})
    if (!noAgent.isError) throw new Error('expected workspace_current failure without identity')
    expect(noAgent.error.message).toContain('no session identity')
  })

  it('backfills workspace dirs for users registered before the feature existed', async () => {
    // Simulate a pre-feature user: wipe the dirs, then a workspaces() read
    // must restore them (lazy backfill on access).
    await rm(path.join(preProcRoot, 'U-carol'), { recursive: true, force: true })
    await rm(path.join(deployRoot, 'U-carol'), { recursive: true, force: true })
    const list = await ctx.aliothAuth.workspaces({ namespace: 'U-carol', role: 'admin' })
    expect(list.workspaces.map(ws => ws.namespace)).toContain('U-carol')
    const preStat = await import('node:fs/promises').then(fs => fs.stat(path.join(preProcRoot, 'U-carol')))
    const deployStat = await import('node:fs/promises').then(fs => fs.stat(path.join(deployRoot, 'U-carol')))
    expect(preStat.isDirectory()).toBe(true)
    expect(deployStat.isDirectory()).toBe(true)
  })

  it('hides orphan U-* dirs without a user row in standard mode', async () => {
    // A stale directory from a deleted account / foreign instance.
    await mkdir(path.join(preProcRoot, 'U-ghost'), { recursive: true })
    await mkdir(path.join(deployRoot, 'U-ghost'), { recursive: true })
    const list = await ctx.aliothAuth.workspaces({ namespace: 'U-carol', role: 'admin' })
    const names = list.workspaces.map(ws => ws.namespace)
    expect(names).not.toContain('U-ghost')
    expect(names).toContain('U-carol')
    await rm(path.join(preProcRoot, 'U-ghost'), { recursive: true, force: true })
    await rm(path.join(deployRoot, 'U-ghost'), { recursive: true, force: true })
  })

  it('workspaces scope every account to its own namespace in standard mode', async () => {
    // erin is a plain user; put an app in her workspace.
    await ctx.aliothAuth.register('erin', 'password-123')
    const appsDir = path.join(preProcRoot, 'U-erin', 'Apps', 'demo-app')
    await mkdir(appsDir, { recursive: true })
    await writeFile(path.join(appsDir, 'app.json'), JSON.stringify({ code: 'demo-app', name: 'Demo 应用' }))

    const own = await ctx.aliothAuth.workspaces({ namespace: 'U-erin', role: 'user' })
    expect(own.mode).toBe('standard')
    expect(own.workspaces.map(ws => ws.namespace)).toEqual(['U-erin'])
    expect(own.workspaces[0]).toMatchObject({
      preProcPath: path.join(preProcRoot, 'U-erin'),
      deployPath: path.join(deployRoot, 'U-erin'),
      apps: [{ code: 'demo-app', name: 'Demo 应用' }],
    })

    // No super-admin: an equal user (role field kept for compatibility) also
    // sees exactly their own namespace — never another account's apps.
    const alice = await ctx.aliothAuth.workspaces({ namespace: 'U-alice', role: 'user' })
    expect(alice.workspaces.map(ws => ws.namespace)).toEqual(['U-alice'])
    const adminRole = await ctx.aliothAuth.workspaces({ namespace: 'U-alice', role: 'admin' })
    expect(adminRole.workspaces.map(ws => ws.namespace)).toEqual(['U-alice'])
  })
})
