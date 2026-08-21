import { describe, expect, it, beforeAll, afterAll } from 'vitest'
import { mkdtemp, mkdir, rm, writeFile } from 'node:fs/promises'
import { tmpdir } from 'node:os'
import path from 'node:path'
import { Context } from '@deepseek-ai/cordis'
import SystemPrompt from '@deepseek-ai/dsh-system-prompt'
import ToolRuntime from '@deepseek-ai/dsh-tools'
import { CallId } from '@deepseek-ai/dsh-llm'
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
    callId: CallId(`auth-${++counter}`),
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
    expect(result.role).toBe('admin') // first user bootstraps as admin
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
    // bob is a plain user (second registration); alice is admin.
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

  it('allows an admin across namespaces', async () => {
    // alice is admin (first user); bind her token to an agent session.
    const adminSession = 'session-alice-1'
    await ctx.aliothAuth.bind(aliceToken, adminSession)
    const agent = fakeAgent(adminSession)
    const result = await callTool('alioth_app_write', {
      namespace: 'U-bob', code: 'admin-app', name: '管理写入', modules: [{ id: 'm1', name: 'M1' }],
    }, agent)
    if (result.isError) throw new Error(`expected admin write success: ${result.error.message}`)
    expect(result.value).toMatchObject({ code: 'admin-app' })
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

  it('resolves the workspace mode: standard by default, unlimited only when asked', async () => {
    expect(ctx.aliothAuth.workspaceMode()).toBe('standard')

    // ALIOTH_WORKSPACE_MODE env wins (restored afterwards)
    const saved = process.env.ALIOTH_WORKSPACE_MODE
    try {
      process.env.ALIOTH_WORKSPACE_MODE = 'unlimited'
      expect(ctx.aliothAuth.workspaceMode()).toBe('unlimited')
      process.env.ALIOTH_WORKSPACE_MODE = 'standard'
      expect(ctx.aliothAuth.workspaceMode()).toBe('standard')
    } finally {
      if (saved === undefined) { delete process.env.ALIOTH_WORKSPACE_MODE } else { process.env.ALIOTH_WORKSPACE_MODE = saved }
    }
  })

  it('unlimited mode opens every namespace to every user', async () => {
    // erin is a plain user on the main ctx (real DB + workspace dirs).
    await ctx.aliothAuth.register('erin', 'password-123')
    // A second context pinned to unlimited: a plain user sees all workspaces.
    const unlimitedCtx = new Context()
    const system = await unlimitedCtx.plugin(SystemPrompt)
    const tools = await unlimitedCtx.plugin(ToolRuntime)
    const env = await unlimitedCtx.plugin(envAlioth, { modelSource: path.join(tmpdir(), 'unused-model'), dataRoot: path.join(tmpdir(), 'unused-data') })
    try {
      const authPlugin = await unlimitedCtx.plugin(auth, {
        mode: 'open', workspaceMode: 'unlimited', preProcRoot, deployRoot,
      })
      try {
        expect(unlimitedCtx.aliothAuth.workspaceMode()).toBe('unlimited')
        // erin is a plain user (registered on the real-DB main ctx; the
        // unlimited ctx only scans the FS — workspaces needs no database).
        const all = await unlimitedCtx.aliothAuth.workspaces({ namespace: 'U-erin', role: 'user' })
        expect(all.mode).toBe('unlimited')
        expect(all.workspaces.map(ws => ws.namespace)).toEqual(expect.arrayContaining(['U-alice', 'U-carol', 'U-erin']))
        // unlimited carries the workspace paths (自定义工作区 chrome)
        expect(all.workspaces[0]).toMatchObject({ preProcPath: path.join(preProcRoot, 'U-alice') })

        // Custom workspace creation: auto-creates the AliothStudio structure.
        const custom = await unlimitedCtx.aliothAuth.createWorkspace('ProjectA')
        expect(custom).toMatchObject({
          namespace: 'ProjectA',
          preProcPath: path.join(preProcRoot, 'ProjectA'),
          deployPath: path.join(deployRoot, 'ProjectA'),
          apps: [],
        })
        const preStat = await import('node:fs/promises').then(fs => fs.stat(path.join(preProcRoot, 'ProjectA')))
        const deployStat = await import('node:fs/promises').then(fs => fs.stat(path.join(deployRoot, 'ProjectA')))
        expect(preStat.isDirectory()).toBe(true)
        expect(deployStat.isDirectory()).toBe(true)
        // reserved prefix and pattern guards
        await expect(unlimitedCtx.aliothAuth.createWorkspace('U-evil')).rejects.toThrow(/reserved/)
        await expect(unlimitedCtx.aliothAuth.createWorkspace('lower')).rejects.toThrow(/invalid namespace/)
      } finally {
        await authPlugin.dispose()
      }
    } finally {
      await env.dispose()
      await tools.dispose()
      await system.dispose()
    }
  })

  it('standard mode rejects custom workspace creation', async () => {
    await expect(ctx.aliothAuth.createWorkspace('ProjectB')).rejects.toThrow(/disabled/)
  })

  it('backfills workspace dirs for users registered before the feature existed', async () => {
    // Simulate a pre-feature user: wipe the dirs, then a workspaces() read
    // must restore them (lazy backfill on access).
    await rm(path.join(preProcRoot, 'U-isahl'), { recursive: true, force: true })
    await rm(path.join(deployRoot, 'U-isahl'), { recursive: true, force: true })
    const list = await ctx.aliothAuth.workspaces({ namespace: 'U-isahl', role: 'admin' })
    expect(list.workspaces.map(ws => ws.namespace)).toContain('U-isahl')
    const preStat = await import('node:fs/promises').then(fs => fs.stat(path.join(preProcRoot, 'U-isahl')))
    const deployStat = await import('node:fs/promises').then(fs => fs.stat(path.join(deployRoot, 'U-isahl')))
    expect(preStat.isDirectory()).toBe(true)
    expect(deployStat.isDirectory()).toBe(true)
  })

  it('workspaces scopes users to their own namespace in standard mode and admins to all', async () => {
    // erin (already registered above) is a plain user; put an app in her workspace.
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

    const admin = await ctx.aliothAuth.workspaces({ namespace: 'U-alice', role: 'admin' })
    expect(admin.workspaces.map(ws => ws.namespace)).toEqual(expect.arrayContaining(['U-alice', 'U-carol', 'U-erin']))
    expect(admin.workspaces.find(ws => ws.namespace === 'U-erin')?.apps).toEqual([{ code: 'demo-app', name: 'Demo 应用' }])
  })
})
