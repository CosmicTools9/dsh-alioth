import { describe, expect, it, beforeAll, afterAll } from 'vitest'
import { mkdir, mkdtemp, writeFile } from 'node:fs/promises'
import { request as httpRequest } from 'node:http'
import { tmpdir } from 'node:os'
import path from 'node:path'
import { Context } from '@deepseek-ai/cordis'
import SystemPrompt from '@deepseek-ai/dsh-system-prompt'
import ToolRuntime from '@deepseek-ai/dsh-tools'
import WebServer from '@deepseek-ai/dsh-host-webserver'
import * as envAlioth from '@dsh-alioth/env-alioth'
import * as authAlioth from '@dsh-alioth/auth-alioth'
import * as landingAlioth from '@dsh-alioth/landing-alioth'
import * as authWebAlioth from '@dsh-alioth/auth-web-alioth'
import * as billingAlioth from '@dsh-alioth/billing-alioth'
import * as billingWeb from '../src/index.ts'

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
let sessionCookie: string

/** Host/Origin-controlled request — fetch refuses to set those headers. */
function rawRequest(targetPort: number, path: string, init: {
  method?: string
  headers?: Record<string, string>
  body?: string
}): Promise<{ status: number; headers: Record<string, string | string[] | undefined>; body: string }> {
  const { promise, resolve, reject } = Promise.withResolvers<{
    status: number
    headers: Record<string, string | string[] | undefined>
    body: string
  }>()
  const request = httpRequest({
    host: '127.0.0.1',
    port: targetPort,
    path,
    method: init.method ?? 'GET',
    headers: init.headers,
  }, response => {
    const chunks: Buffer[] = []
    response.on('data', chunk => chunks.push(Buffer.from(chunk)))
    response.on('end', () => resolve({
      status: response.statusCode ?? 0,
      headers: response.headers,
      body: Buffer.concat(chunks).toString('utf8'),
    }))
  })
  request.on('error', reject)
  if (init.body !== undefined) request.write(init.body)
  request.end()
  return promise
}

/** Extract the `{ error }` payload of a failing response (narrowed, no cast). */
async function jsonError(response: Response): Promise<string> {
  const body: unknown = await response.json()
  if (typeof body === 'object' && body !== null && 'error' in body && typeof body.error === 'string') {
    return body.error
  }
  throw new Error(`response has no error field: ${JSON.stringify(body)}`)
}

/** Register a fresh account on the composed web origin; returns session + token. */
async function registerUser(username: string): Promise<{ cookie: string; token: string }> {
  const response = await fetch(`http://127.0.0.1:${ctx.webServer.port}/api/auth/register`, {
    method: 'POST',
    headers: { 'content-type': 'application/json' },
    body: JSON.stringify({ username, password: 'password-123' }),
  })
  if (response.status !== 201) {
    throw new Error(`register ${username} failed: ${response.status} ${await response.text()}`)
  }
  const body: unknown = await response.json()
  if (typeof body !== 'object' || body === null || !('token' in body) || typeof body.token !== 'string') {
    throw new Error(`register ${username}: no token in response`)
  }
  return {
    token: body.token,
    cookie: response.headers.getSetCookie().map(cookie => cookie.split(';')[0]).join('; '),
  }
}

beforeAll(async () => {
  const modelDir = await mkdtemp(path.join(tmpdir(), 'billweb-model-'))
  const dataRoot = await mkdtemp(path.join(tmpdir(), 'billweb-data-'))
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
  const webServerPlugin = await ctx.plugin(WebServer, { host: '127.0.0.1', port: 0 })
  disposers.push(() => webServerPlugin.dispose())
  const landing = await ctx.plugin(landingAlioth, {})
  disposers.push(() => landing.dispose())
  const auth = await ctx.plugin(authAlioth, { mode: 'open' })
  disposers.push(() => auth.dispose())
  const authWeb = await ctx.plugin(authWebAlioth, { port: 3960 + Math.floor(Math.random() * 30) })
  disposers.push(() => authWeb.dispose())
  const billing = await ctx.plugin(billingAlioth, {})
  disposers.push(() => billing.dispose())
  const carrier = await ctx.plugin(billingWeb, {})
  disposers.push(() => carrier.dispose())

  // Equal users (no super-admin); keep the session cookie for the
  // cookie-authenticated user-center flow under test.
  const register = await fetch(`http://127.0.0.1:${ctx.webServer.port}/api/auth/register`, {
    method: 'POST',
    headers: { 'content-type': 'application/json' },
    body: JSON.stringify({ username: 'ada', password: 'password-123' }),
  })
  const cookie = register.headers.getSetCookie().find(c => c.startsWith('alioth_session='))
  sessionCookie = cookie!.split(';')[0]!
}, 120_000)

afterAll(async () => {
  for (const dispose of disposers.reverse()) {
    await dispose().catch(() => {})
  }
})

describe('user center (web carrier)', () => {
  const webBase = (): string => `http://127.0.0.1:${ctx.webServer.port}`

  it('bounces unauthenticated visits to /login', async () => {
    const response = await fetch(`${webBase()}/usercenter`, { redirect: 'manual' })
    expect(response.status).toBe(302)
    expect(response.headers.get('location')).toBe('/login')
  })

  it('renders the overview page with account + subscription panels', async () => {
    const response = await fetch(`${webBase()}/usercenter`, { headers: { cookie: sessionCookie } })
    expect(response.status).toBe(200)
    const html = await response.text()
    expect(html).toContain('用户中心')
    expect(html).toContain('ada')
    expect(html).toContain('U-ada')
    expect(html).toContain('L0 社区版')
  })

  it('full loop over the JSON API: subscribe → pay → invoice (issued on request)', async () => {
    const api = (path: string, init: RequestInit = {}): Promise<Response> =>
      fetch(`${webBase()}${path}`, {
        ...init,
        headers: { 'content-type': 'application/json', cookie: sessionCookie, ...init.headers },
      })

    expect((await api('/api/billing/overview')).status).toBe(200)

    const sub = await api('/api/billing/subscribe', { method: 'POST', body: '{}' })
    expect(sub.status).toBe(200)
    const subBody = await sub.json() as { status: string }
    expect(subBody.status).toBe('active')

    const overview = await (await api('/api/billing/overview')).json() as { bills: Array<{ id: string; status: string; amountCents: number }> }
    expect(overview.bills).toHaveLength(1)
    expect(overview.bills[0]!.amountCents).toBe(139900)
    const billId = overview.bills[0]!.id

    const paid = await api('/api/billing/pay', { method: 'POST', body: JSON.stringify({ bill: billId }) })
    expect(paid.status).toBe(200)

    const invoice = await api('/api/billing/invoice', {
      method: 'POST',
      body: JSON.stringify({ bill: billId, title: '杭州示例科技', tax: '91330100MA27X00000' }),
    })
    expect(invoice.status).toBe(200)
    const invBody = await invoice.json() as { id: string; status: string }
    // Self-service: no admin review queue — requesting issues directly.
    expect(invBody.status).toBe('issued')

    const dup = await api('/api/billing/invoice', {
      method: 'POST',
      body: JSON.stringify({ bill: billId, title: 'again', tax: '' }),
    })
    expect(dup.status).toBe(400)
  })

  it('form posts redirect back with a notice banner', async () => {
    const response = await fetch(`${webBase()}/api/billing/cancel`, {
      method: 'POST',
      redirect: 'manual',
      headers: { 'content-type': 'application/x-www-form-urlencoded', cookie: sessionCookie },
      body: '',
    })
    expect(response.status).toBe(302)
    expect(response.headers.get('location')).toContain('/usercenter/subscription?notice=')
  })

  it('renders bills and invoices pages for the subscribed user', async () => {
    const bills = await fetch(`${webBase()}/usercenter/bills`, { headers: { cookie: sessionCookie } })
    expect(bills.status).toBe(200)
    const billsHtml = await bills.text()
    expect(billsHtml).toContain('已支付')
    expect(billsHtml).toContain('¥1,399')

    const invoices = await fetch(`${webBase()}/usercenter/invoices`, { headers: { cookie: sessionCookie } })
    expect(invoices.status).toBe(200)
    const invoicesHtml = await invoices.text()
    expect(invoicesHtml).toContain('杭州示例科技')
    expect(invoicesHtml).toContain('已开具')
    // No super-admin: the issuance queue panel does not render.
    expect(invoicesHtml).not.toContain('开具队列')
  })
})

describe('user center transport + failure branches (real services)', () => {
  const base = (): string => `http://127.0.0.1:${ctx.webServer.port}`
  let cookie: string
  let token: string
  const call = (path: string, init: RequestInit = {}): Promise<Response> =>
    fetch(`${base()}${path}`, { ...init, headers: { cookie, ...init.headers } })

  beforeAll(async () => {
    ({ cookie, token } = await registerUser('bill-transport'))
  })

  it('bounces anonymous visits to every user-center page', async () => {
    for (const path of ['/usercenter', '/usercenter/subscription', '/usercenter/bills', '/usercenter/invoices']) {
      const response = await fetch(`${base()}${path}`, { redirect: 'manual' })
      expect(response.status).toBe(302)
      expect(response.headers.get('location')).toBe('/login')
    }
  })

  it('walks the subscription page through L0 → L1 → canceled', async () => {
    const fresh = await call('/usercenter/subscription')
    expect(fresh.status).toBe(200)
    const freshHtml = await fresh.text()
    expect(freshHtml).toContain('当前：L0 社区版')
    expect(freshHtml).toContain('（当前套餐）')
    expect(freshHtml).toContain('action="/api/billing/subscribe"')
    expect(freshHtml).not.toContain('取消订阅')

    const subscribed = await call('/api/billing/subscribe', { method: 'POST' })
    expect(subscribed.status).toBe(200)

    const active = await call('/usercenter/subscription')
    const activeHtml = await active.text()
    expect(activeHtml).toContain('当前：L1 订阅版')
    expect(activeHtml).toContain('action="/api/billing/cancel"')
    expect(activeHtml).toContain('取消订阅（期末生效）')

    const overview = await call('/usercenter')
    const overviewHtml = await overview.text()
    expect(overviewHtml).toContain('L1 订阅版')
    expect(overviewHtml).toContain('生效中')
    expect(overviewHtml).toMatch(/下次续期<\/dt><dd>\d{4}-\d{2}-\d{2}<\/dd>/)

    const canceled = await call('/api/billing/cancel', { method: 'POST' })
    expect(canceled.status).toBe(200)
    expect(await canceled.json()).toEqual({ ok: true })
    expect(await (await call('/usercenter')).text()).toContain('已取消')
  })

  it('renders unpaid / paid / empty bill states', async () => {
    const billsUser = await registerUser('bill-states')
    const asBillsUser = (path: string, init: RequestInit = {}): Promise<Response> =>
      fetch(`${base()}${path}`, { ...init, headers: { cookie: billsUser.cookie, ...init.headers } })

    const fresh = await asBillsUser('/usercenter/bills')
    const freshHtml = await fresh.text()
    expect(freshHtml).toContain('暂无账单 — 订阅 L1 后按月生成。')

    await asBillsUser('/api/billing/subscribe', { method: 'POST' })
    const unpaid = await asBillsUser('/usercenter/bills')
    const unpaidHtml = await unpaid.text()
    expect(unpaidHtml).toContain('待支付')
    expect(unpaidHtml).toContain('/api/billing/pay')
    expect(unpaidHtml).toContain('¥1,399')

    const overview = await (await asBillsUser('/api/billing/overview')).json() as { bills: Array<{ id: string }> }
    const billId = overview.bills[0]!.id
    expect((await asBillsUser('/api/billing/pay', {
      method: 'POST',
      body: JSON.stringify({ bill: billId }),
      headers: { 'content-type': 'application/json' },
    })).status).toBe(200)

    const paidHtml = await (await asBillsUser('/usercenter/bills')).text()
    expect(paidHtml).toContain('已支付')
    expect(paidHtml).toContain('申请发票')
  })

  it('renders the invoice form, records invoices, and hides the admin queue', async () => {
    const invoiceUser = await registerUser('bill-invoice')
    const asInvoiceUser = (path: string, init: RequestInit = {}): Promise<Response> =>
      fetch(`${base()}${path}`, { ...init, headers: { cookie: invoiceUser.cookie, ...init.headers } })

    const before = await asInvoiceUser('/usercenter/invoices')
    const beforeHtml = await before.text()
    expect(beforeHtml).toContain('暂无可开票账单')
    expect(beforeHtml).toContain('暂无发票记录。')
    expect(beforeHtml).not.toContain('开具队列')

    await asInvoiceUser('/api/billing/subscribe', { method: 'POST' })
    const overview = await (await asInvoiceUser('/api/billing/overview')).json() as { bills: Array<{ id: string; status: string }> }
    const unpaidBill = overview.bills[0]!
    await asInvoiceUser('/api/billing/pay', {
      method: 'POST',
      headers: { 'content-type': 'application/json' },
      body: JSON.stringify({ bill: unpaidBill.id }),
    })

    const offer = await asInvoiceUser('/usercenter/invoices')
    const offerHtml = await offer.text()
    expect(offerHtml).toContain('action="/api/billing/invoice"')
    expect(offerHtml).toContain(`<option value="${unpaidBill.id}">`)

    const requested = await asInvoiceUser('/api/billing/invoice', {
      method: 'POST',
      headers: { 'content-type': 'application/json' },
      body: JSON.stringify({ bill: unpaidBill.id, title: '无税号公司', tax: '' }),
    })
    expect(requested.status).toBe(200)
    const issuedId = ((await requested.json()) as { id: string }).id

    const afterHtml = await (await asInvoiceUser('/usercenter/invoices')).text()
    expect(afterHtml).toContain('无税号公司')
    expect(afterHtml).toContain('已开具')
    expect(afterHtml).toMatch(/<td>—<\/td>/)

    // Idempotent re-issue of the invoice just requested (the JSON twin).
    const reissued = await asInvoiceUser('/api/billing/issue', {
      method: 'POST',
      headers: { 'content-type': 'application/json' },
      body: JSON.stringify({ invoice: issuedId }),
    })
    expect(reissued.status).toBe(200)
    expect(await reissued.json()).toMatchObject({ id: issuedId, status: 'issued' })
  })

  it('shows notice and error banners from the query feed', async () => {
    const notice = await call('/usercenter?notice=%E6%93%8D%E4%BD%9C%E6%88%90%E5%8A%9F')
    const noticeHtml = await notice.text()
    expect(noticeHtml).toContain('class="banner ok"')
    expect(noticeHtml).toContain('操作成功')

    const failure = await call('/usercenter?error=%E5%A4%B1%E8%B4%A5%E4%BA%86')
    const failureHtml = await failure.text()
    expect(failureHtml).toContain('class="banner error"')
    expect(failureHtml).toContain('失败了')
  })

  it('authenticates the JSON API by bearer token and rejects odd headers', async () => {
    const bearer = await fetch(`${base()}/api/billing/overview`, { headers: { authorization: `Bearer ${token}` } })
    expect(bearer.status).toBe(200)
    expect(await bearer.json()).toMatchObject({ username: 'bill-transport', role: 'user' })

    expect((await fetch(`${base()}/api/billing/overview`, { headers: { authorization: 'Bearer' } })).status).toBe(401)
    expect((await fetch(`${base()}/api/billing/overview`, { headers: { authorization: 'Basic YXJhOng=' } })).status).toBe(401)
    expect((await fetch(`${base()}/api/billing/overview`, { headers: { cookie: 'other=1' } })).status).toBe(401)
  })

  it('answers requests without a body content-type and malformed JSON', async () => {
    const body = 'bill=whatever'
    const noType = await rawRequest(ctx.webServer.port, '/api/billing/pay', {
      method: 'POST',
      headers: { cookie, 'content-length': String(Buffer.byteLength(body)) },
      body,
    })
    expect(noType.status).toBe(400)
    expect(JSON.parse(noType.body)).toEqual({ error: '缺少账单' })

    const empty = await fetch(`${base()}/api/billing/pay`, {
      method: 'POST',
      headers: { 'content-type': 'application/json', cookie },
    })
    expect(empty.status).toBe(400)
    expect(await jsonError(empty)).toBe('缺少账单')

    const malformed = await fetch(`${base()}/api/billing/pay`, {
      method: 'POST',
      headers: { 'content-type': 'application/json', cookie },
      body: '{"bill":',
    })
    // A body that does not parse fails the request instead of silently
    // degrading to {} (which would answer the carrier's 缺少账单 payload).
    expect(malformed.status).toBe(400)
    expect(await malformed.text()).not.toContain('缺少账单')

    const nonString = await fetch(`${base()}/api/billing/pay`, {
      method: 'POST',
      headers: { 'content-type': 'application/json', cookie },
      body: JSON.stringify({ bill: 42 }),
    })
    expect(nonString.status).toBe(400)
    expect(await jsonError(nonString)).toBe('缺少账单')
  })

  it('rejects cross-origin form posts and accepts same-origin ones', async () => {
    const origin = `http://127.0.0.1:${ctx.webServer.port}`
    const crossOrigin = await rawRequest(ctx.webServer.port, '/api/billing/subscribe', {
      method: 'POST',
      headers: { cookie, origin: 'http://evil.example', 'content-type': 'application/x-www-form-urlencoded' },
    })
    expect(crossOrigin.status).toBe(403)
    expect(JSON.parse(crossOrigin.body)).toEqual({ error: 'origin mismatch' })

    const sameOrigin = await rawRequest(ctx.webServer.port, '/api/billing/subscribe', {
      method: 'POST',
      headers: { cookie, origin, host: `127.0.0.1:${ctx.webServer.port}`, 'content-type': 'application/x-www-form-urlencoded' },
    })
    expect(sameOrigin.status).toBe(302)
    expect(String(sameOrigin.headers.location)).toContain('/usercenter/subscription?notice=')
  })

  it('rejects anonymous action posts and unknown billing paths', async () => {
    const anonymous = await fetch(`${base()}/api/billing/subscribe`, { method: 'POST' })
    expect(anonymous.status).toBe(401)
    expect(await jsonError(anonymous)).toBe('unauthorized')

    const unknown = await fetch(`${base()}/api/billing/nope`, { headers: { cookie } })
    expect(unknown.status).toBe(404)
    expect(await jsonError(unknown)).toBe('not found')
  })

  it('validates the invoice + issue payloads on the JSON channel', async () => {
    const missingBill = await fetch(`${base()}/api/billing/invoice`, {
      method: 'POST',
      headers: { 'content-type': 'application/json', cookie },
      body: JSON.stringify({ bill: '', title: 42, tax: 42 }),
    })
    expect(missingBill.status).toBe(400)
    expect(await jsonError(missingBill)).toBe('缺少账单')

    const typedFields = await fetch(`${base()}/api/billing/invoice`, {
      method: 'POST',
      headers: { 'content-type': 'application/json', cookie },
      body: JSON.stringify({ bill: 7, title: 42, tax: { a: 1 } }),
    })
    expect(typedFields.status).toBe(400)
    expect(await jsonError(typedFields)).toBe('缺少账单')

    const missingInvoice = await fetch(`${base()}/api/billing/issue`, {
      method: 'POST',
      headers: { 'content-type': 'application/json', cookie },
      body: JSON.stringify({ invoice: '' }),
    })
    expect(missingInvoice.status).toBe(400)
    expect(await jsonError(missingInvoice)).toBe('缺少发票申请')

    const typedInvoice = await fetch(`${base()}/api/billing/issue`, {
      method: 'POST',
      headers: { 'content-type': 'application/json', cookie },
      body: JSON.stringify({ invoice: 99 }),
    })
    expect(typedInvoice.status).toBe(400)
    expect(await jsonError(typedInvoice)).toBe('缺少发票申请')
  })

  it('redirects a failed form action back with the reason in the query', async () => {
    const response = await fetch(`${base()}/api/billing/pay`, {
      method: 'POST',
      headers: { 'content-type': 'application/x-www-form-urlencoded', cookie },
      body: new URLSearchParams({ bill: 'no-such-bill' }).toString(),
      redirect: 'manual',
    })
    expect(response.status).toBe(302)
    const location = decodeURIComponent(String(response.headers.get('location')))
    expect(location).toContain('/usercenter/bills?error=')
    expect(location).toContain('bill not found')
  })
})
