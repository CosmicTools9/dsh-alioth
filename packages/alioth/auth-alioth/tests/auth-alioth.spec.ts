import { describe, expect, it, beforeAll, afterAll } from 'vitest'
import { mkdir, mkdtemp, rm, writeFile } from 'node:fs/promises'
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
let port: number
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

  // Port: bind to 0 → kernel-assigned; the plugin reads config.port before
  // listen, so pick a free port by probing.
  port = 3987 + Math.floor(Math.random() * 500)
  const authPlugin = await ctx.plugin(auth, { port, mode: 'enforce' })
  disposers.push(() => authPlugin.dispose())
}, 120_000)

afterAll(async () => {
  for (const dispose of disposers.reverse()) {
    await dispose().catch(() => {})
  }
  await rm(preProcRoot, { recursive: true, force: true })
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

describe('B/S HTTP surface (real server)', () => {
  const base = (): string => `http://127.0.0.1:${port}`

  it('serves the login page (GET /)', async () => {
    const response = await fetch(`${base()}/`)
    expect(response.status).toBe(200)
    const html = await response.text()
    expect(html).toContain('<form')
    expect(html).toContain('/api/auth/login')
  })

  it('registers via browser form submission (urlencoded)', async () => {
    const form = new URLSearchParams({ username: 'carol', password: 'password-789' })
    const response = await fetch(`${base()}/api/auth/register`, {
      method: 'POST',
      headers: { 'content-type': 'application/x-www-form-urlencoded' },
      body: form.toString(),
    })
    expect(response.status).toBe(201)
    const body = await response.json() as { token: string; namespace: string }
    expect(body.namespace).toBe('U-carol')
    expect(body.token).toMatch(/^[0-9a-f]{64}$/)
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
})
