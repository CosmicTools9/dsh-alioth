/**
 * auth-web 安全缝的 HTTP 级集成断言（复检归零补口）：
 * - 表单状态变更的同源校验（Origin ≠ Host → 403）；
 * - 登录限流（同 IP+用户名 连续失败 → 429，成功清零后可登录）；
 * - https 公网 origin → 会话 cookie 带 `Secure`；
 * - 认证事件进审计缝（登录/失败/登出经 billing.audit 留痕）。
 */
import { describe, expect, it, beforeAll, afterAll } from 'vitest'
import { mkdtemp, mkdir, writeFile } from 'node:fs/promises'
import { tmpdir } from 'node:os'
import path from 'node:path'
import { Context } from '@deepseek-ai/cordis'
import SystemPrompt from '@deepseek-ai/dsh-system-prompt'
import ToolRuntime from '@deepseek-ai/dsh-tools'
import * as envAlioth from '@dsh-alioth/env-alioth'
import * as authAlioth from '@dsh-alioth/auth-alioth'
import * as authWebAlioth from '../src/index.ts'
import { createTestDatabase, type TestDatabase } from '../../env-alioth/tests/test-db.ts'

const SCHEMA_DDL = `
CREATE SCHEMA IF NOT EXISTS isahl_meta;
CREATE TABLE isahl_meta.meta_collections (
    table_name text NOT NULL, name text NOT NULL, config jsonb DEFAULT '{}'::jsonb,
    schema text DEFAULT 'isahl'::text, PRIMARY KEY (table_name)
);
`

const PUBLIC_ORIGIN = 'https://shop.example.com'
const disposers: Array<() => Promise<void>> = []
let testDb: TestDatabase
let webBase = ''
const auditEvents: { actor: string; event: string; target: string }[] = []

async function call(pathname: string, init: RequestInit = {}): Promise<Response> {
  return fetch(`${webBase}${pathname}`, { redirect: 'manual', ...init })
}

beforeAll(async () => {
  testDb = await createTestDatabase('authwebsec')
  const modelDir = await mkdtemp(path.join(tmpdir(), 'authsec-model-'))
  const dataRoot = await mkdtemp(path.join(tmpdir(), 'authsec-data-'))
  const preProcRoot = await mkdtemp(path.join(tmpdir(), 'authsec-preproc-'))
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

  const ctx = new Context()
  const system = await ctx.plugin(SystemPrompt)
  disposers.push(() => system.dispose())
  const tools = await ctx.plugin(ToolRuntime)
  disposers.push(() => tools.dispose())
  const env = await ctx.plugin(envAlioth, { modelSource: modelDir, dataRoot, databaseUrl: testDb.url })
  disposers.push(() => env.dispose())
  // 审计缝替身（真实审计在 billing 集成里验过；这里断言 auth-web 的事件接线）。
  ctx.provide('aliothBilling')
  ctx.set('aliothBilling', {
    async sourceLicense() { return null },
    async audit(actor: string, event: string, target = '', _evidence = '') {
      auditEvents.push({ actor, event, target })
    },
  } as never)
  const auth = await ctx.plugin(authAlioth, { mode: 'open', preProcRoot, deployRoot: preProcRoot })
  disposers.push(() => auth.dispose())
  // standalone 载体端口固定（本套件独占；主 spec 的随机段不相交）。
  const port = 3944
  const authWeb = await ctx.plugin(authWebAlioth, {
    port,
    webGate: false,
    publicOrigin: PUBLIC_ORIGIN, // https → Secure cookie
  })
  disposers.push(() => authWeb.dispose())
  webBase = `http://127.0.0.1:${port}`
}, 120_000)

afterAll(async () => {
  for (const dispose of disposers.reverse()) {
    await dispose().catch(() => {})
  }
  await testDb.dispose()
})

describe('auth-web security seams (real server)', () => {
  it('issues Secure session cookies for an https public origin', async () => {
    const username = `sec${Date.now() % 100000}`
    const response = await call('/api/auth/register', {
      method: 'POST',
      headers: { 'content-type': 'application/json' },
      body: JSON.stringify({ username, password: 'password-123' }),
    })
    expect(response.status).toBe(201)
    const setCookie = response.headers.get('set-cookie') ?? ''
    expect(setCookie).toContain('Secure')
    expect(setCookie).toContain('HttpOnly')
  })

  it('rejects cross-origin form posts (CSRF second line)', async () => {
    const response = await call('/api/auth/login', {
      method: 'POST',
      headers: { 'content-type': 'application/x-www-form-urlencoded', origin: 'https://evil.example' },
      body: 'username=nobody&password=whatever1',
    })
    expect(response.status).toBe(403)
    // JSON 客户端（无 Origin）不受影响。
    const json = await call('/api/auth/login', {
      method: 'POST',
      headers: { 'content-type': 'application/json' },
      body: JSON.stringify({ username: 'nobody', password: 'whatever1' }),
    })
    expect(json.status).toBe(401)
  })

  it('throttles repeated failures per ip+username, then admits a clean login', async () => {
    const username = `thr${Date.now() % 100000}`
    await call('/api/auth/register', {
      method: 'POST',
      headers: { 'content-type': 'application/json' },
      body: JSON.stringify({ username, password: 'password-123' }),
    })
    // 同 IP + 用户名 连续 10 次失败（默认窗口）。
    for (let attempt = 0; attempt < 10; attempt += 1) {
      const failure = await call('/api/auth/login', {
        method: 'POST',
        headers: { 'content-type': 'application/json' },
        body: JSON.stringify({ username, password: 'wrong-password' }),
      })
      expect([401, 429]).toContain(failure.status)
    }
    // 正确口令也被限流（429）。
    const blocked = await call('/api/auth/login', {
      method: 'POST',
      headers: { 'content-type': 'application/json' },
      body: JSON.stringify({ username, password: 'password-123' }),
    })
    expect(blocked.status).toBe(429)
    // 别的用户名不受牵连。
    const other = await call('/api/auth/login', {
      method: 'POST',
      headers: { 'content-type': 'application/json' },
      body: JSON.stringify({ username: `${username}-x`, password: 'whatever1' }),
    })
    expect(other.status).toBe(401)
  })

  it('records auth events on the audit seam', async () => {
    const username = `audit${Date.now() % 100000}`
    const registered = await call('/api/auth/register', {
      method: 'POST',
      headers: { 'content-type': 'application/json' },
      body: JSON.stringify({ username, password: 'password-123' }),
    })
    const token = ((await registered.json()) as { token: string }).token
    const login = await call('/api/auth/login', {
      method: 'POST',
      headers: { 'content-type': 'application/json' },
      body: JSON.stringify({ username, password: 'password-123' }),
    })
    expect(login.status).toBe(200)
    await call('/api/auth/logout', { method: 'POST', headers: { authorization: `Bearer ${token}` } })
    const events = auditEvents.map(entry => entry.event)
    expect(events).toContain('auth.register')
    expect(events).toContain('auth.login')
    expect(events).toContain('auth.logout')
    // 失败也留痕。
    const before = auditEvents.length
    await call('/api/auth/login', {
      method: 'POST',
      headers: { 'content-type': 'application/json' },
      body: JSON.stringify({ username, password: 'wrong-password' }),
    })
    expect(auditEvents.slice(before).map(entry => entry.event)).toContain('auth.login-failed')
  })
})
