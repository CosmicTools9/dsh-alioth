import { describe, expect, it, beforeAll, afterAll } from 'vitest'
import { mkdir, mkdtemp, writeFile } from 'node:fs/promises'
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
