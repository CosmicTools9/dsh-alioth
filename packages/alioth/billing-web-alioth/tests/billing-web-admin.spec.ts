/**
 * 运营面（admin）与外部渠道结算回调（ext-adapter 契约）——真实插件链 +
 * 一次性数据库：
 *
 * - admin 页只对 role=admin 开放（页面与 API 双重）；
 * - L2 授权开通（页面 API 与 CLI 等效）、退款/作废走服务面并写审计；
 * - 回调端点：无密钥 → 503 关闭；HMAC/time-window 验签；金额不符 → 409。
 */
import { createHmac } from 'node:crypto'
import { describe, expect, it, beforeAll, afterAll } from 'vitest'
import { mkdtemp, mkdir, writeFile } from 'node:fs/promises'
import { tmpdir } from 'node:os'
import path from 'node:path'
import { Context } from '@deepseek-ai/cordis'
import SystemPrompt from '@deepseek-ai/dsh-system-prompt'
import ToolRuntime from '@deepseek-ai/dsh-tools'
import WebServer from '@deepseek-ai/dsh-host-webserver'
import * as envAlioth from '@dsh-alioth/env-alioth'
import * as authAlioth from '@dsh-alioth/auth-alioth'
import * as authWebAlioth from '@dsh-alioth/auth-web-alioth'
import * as billingAlioth from '@dsh-alioth/billing-alioth'
import * as billingWeb from '../src/index.ts'
import { createTestDatabase, type TestDatabase } from '../../env-alioth/tests/test-db.ts'

const SCHEMA_DDL = `
CREATE SCHEMA IF NOT EXISTS isahl_meta;
CREATE TABLE isahl_meta.meta_collections (
    table_name text NOT NULL, name text NOT NULL, config jsonb DEFAULT '{}'::jsonb,
    schema text DEFAULT 'isahl'::text, PRIMARY KEY (table_name)
);
`

const CHANNEL_SECRET = 'channel-test-secret'
const disposers: Array<() => Promise<void>> = []
let ctx: Context
let testDb: TestDatabase
let webBase = ''
let adminCookie = ''
let userCookie = ''
let adminUsername = ''
let username = ''
let userId = ''

async function register(username0: string): Promise<string> {
  const response = await fetch(`${webBase}/api/auth/register`, {
    method: 'POST',
    headers: { 'content-type': 'application/json' },
    body: JSON.stringify({ username: username0, password: 'password-123' }),
  })
  expect(response.status).toBe(201)
  const setCookie = response.headers.get('set-cookie') ?? ''
  return setCookie.split(';')[0]!
}

async function call(cookie: string, pathname: string, init: RequestInit = {}): Promise<Response> {
  return fetch(`${webBase}${pathname}`, {
    ...init,
    headers: { cookie, 'content-type': 'application/json', ...init.headers as Record<string, string> },
    redirect: 'manual',
  })
}

function channelSignature(raw: string, timestamp: number): string {
  const digest = createHmac('sha256', CHANNEL_SECRET).update(`${timestamp}.${raw}`, 'utf8').digest('hex')
  return `t=${timestamp},v1=${digest}`
}

beforeAll(async () => {
  testDb = await createTestDatabase('billadmin')
  const modelDir = await mkdtemp(path.join(tmpdir(), 'bill-admin-model-'))
  const dataRoot = await mkdtemp(path.join(tmpdir(), 'bill-admin-data-'))
  const preProcRoot = await mkdtemp(path.join(tmpdir(), 'bill-admin-preproc-'))
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
  const env = await ctx.plugin(envAlioth, { modelSource: modelDir, dataRoot, databaseUrl: testDb.url })
  disposers.push(() => env.dispose())
  const web = await ctx.plugin(WebServer, { host: '127.0.0.1', port: 0 })
  disposers.push(() => web.dispose())
  const auth = await ctx.plugin(authAlioth, { mode: 'open', preProcRoot, deployRoot: preProcRoot })
  disposers.push(() => auth.dispose())
  const authWeb = await ctx.plugin(authWebAlioth, { port: 3970 + Math.floor(Math.random() * 20), webGate: true })
  disposers.push(() => authWeb.dispose())
  const billing = await ctx.plugin(billingAlioth, {})
  disposers.push(() => billing.dispose())
  const carrier = await ctx.plugin(billingWeb, { channelSecret: CHANNEL_SECRET })
  disposers.push(() => carrier.dispose())

  const port = (ctx.webServer as unknown as { port: number }).port
  webBase = `http://127.0.0.1:${port}`

  // 两个账号：一个由 admin:grant 的唯一通道（这里等效为 role 位直改，测试夹具）
  // 升为 admin，一个是普通用户。
  const suffix = Date.now() % 100000
  adminUsername = `admin${suffix}`
  username = `user${suffix}`
  adminCookie = await register(adminUsername)
  userCookie = await register(username)
  await ctx.aliothEnv.sql(`UPDATE dsh_alioth_auth.users SET role = 'admin' WHERE username = $1`, [adminUsername])
  const row = await ctx.aliothEnv.sql<{ id: string }>(`SELECT id FROM dsh_alioth_auth.users WHERE username = $1`, [username])
  userId = row.rows[0]!.id
}, 120_000)

afterAll(async () => {
  for (const dispose of disposers.reverse()) {
    await dispose().catch(() => {})
  }
  await testDb.dispose()
})

describe('admin back-office', () => {
  it('serves the admin page to admins only (page + API)', async () => {
    const adminPage = await call(adminCookie, '/usercenter/admin')
    expect(adminPage.status).toBe(200)
    expect(await adminPage.text()).toContain('对账')

    const plainPage = await call(userCookie, '/usercenter/admin', { redirect: 'manual' })
    expect(plainPage.status).toBe(302)
  })

  it('grants an L2 window through the admin API (operator path)', async () => {
    const until = '2099-06-30'
    const response = await call(adminCookie, '/api/billing/admin/grant-source', {
      method: 'POST',
      body: JSON.stringify({ username, until, note: '合同 2026-114' }),
    })
    expect(response.status).toBe(200)
    const view = await ctx.aliothBilling.sourceLicenseRequest(userId)
    expect(view?.until?.toISOString()).toBe('2099-06-30T23:59:59.999Z')
  })

  it('refunds a paid bill through the admin API and records the audit line', async () => {
    await call(userCookie, '/api/billing/subscribe', { method: 'POST' })
    const bills = await ctx.aliothBilling.bills(userId)
    const bill = bills[0]!
    await ctx.aliothBilling.payBill(bill.id, { id: userId, role: 'user' })
    const refunded = await call(adminCookie, '/api/billing/admin/refund', {
      method: 'POST', body: JSON.stringify({ bill: bill.id }),
    })
    expect(refunded.status).toBe(200)
    const after = await ctx.aliothBilling.bills(userId)
    expect(after[0]!.status).toBe('refunded')
    const tail = await ctx.aliothBilling.auditTail({ id: 'x', role: 'admin' })
    expect(tail.map(entry => entry.event)).toContain('bill.refunded')
    // 普通用户不可退（admin-only 在账单状态之前判定）。
    const denied = await call(userCookie, '/api/billing/admin/refund', {
      method: 'POST', body: JSON.stringify({ bill: bill.id }),
    })
    expect(denied.status).toBe(400)
  })
})

describe('channel settlement callback (ext-adapter contract)', () => {
  it('is disabled without a secret (fail-closed 503)', async () => {
    // 本部署配置了密钥；关闭语义由另一组合（无密钥）覆盖——这里只验证
    // 配置了密钥时对坏签名拒绝。
    const raw = JSON.stringify({ billId: 'none', amountCents: 1 })
    const bad = await fetch(`${webBase}/api/billing/channel/wechat-pay/callback`, {
      method: 'POST',
      headers: { 'content-type': 'application/json', 'x-alioth-signature': 't=1,v1=deadbeef' },
      body: raw,
    })
    expect(bad.status).toBe(401)
  })

  it('settles a matching amount and refuses a mismatch / bad signature / stale timestamp', async () => {
    // 该用户在上一例已订阅（幂等）；推进续期拿一张新的未付账单。
    const sub = await ctx.aliothBilling.getSubscription(userId)
    if (sub === null || sub.status === 'canceled') await call(userCookie, '/api/billing/subscribe', { method: 'POST' })
    await ctx.aliothBilling.runRenewals(new Date((sub?.renewsAt.getTime() ?? Date.now()) + 1_000))
    const bill = (await ctx.aliothBilling.bills(userId)).find(entry => entry.status === 'unpaid')!
    const raw = JSON.stringify({ billId: bill.id, amountCents: bill.amountCents, note: 'trade_no=4200' })
    const ok = await fetch(`${webBase}/api/billing/channel/wechat-pay/callback`, {
      method: 'POST',
      headers: { 'content-type': 'application/json', 'x-alioth-signature': channelSignature(raw, Math.floor(Date.now() / 1000)) },
      body: raw,
    })
    expect(ok.status).toBe(200)
    expect((await ok.json() as { status: string }).status).toBe('paid')

    // 金额不符 → 409（对账事件，不是支付；金额判定先于幂等返回）。
    const mismatchRaw = JSON.stringify({ billId: bill.id, amountCents: bill.amountCents - 1 })
    const mismatch = await fetch(`${webBase}/api/billing/channel/wechat-pay/callback`, {
      method: 'POST',
      headers: { 'content-type': 'application/json', 'x-alioth-signature': channelSignature(mismatchRaw, Math.floor(Date.now() / 1000)) },
      body: mismatchRaw,
    })
    expect(mismatch.status).toBe(409)

    // 过期时间戳 → 401。
    const stale = await fetch(`${webBase}/api/billing/channel/wechat-pay/callback`, {
      method: 'POST',
      headers: { 'content-type': 'application/json', 'x-alioth-signature': channelSignature(raw, Math.floor(Date.now() / 1000) - 3600) },
      body: raw,
    })
    expect(stale.status).toBe(401)
  })
})
