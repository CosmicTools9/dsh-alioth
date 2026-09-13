/**
 * billing-web-alioth — branch coverage for the user-center states the
 * AppCreator-tier services never produce: the admin 开具队列, invoices that are
 * still `pending` (the shipped in-memory provider issues directly), billing
 * calls that fail with non-Errors, and a `webServer` of the wrong shape.
 *
 * `ctx.aliothAuth` / `ctx.aliothBilling` stand-ins mirror exactly the members
 * the carrier reads (src/index.ts: authedUser + viewData); the web server is
 * the real harness service, so every page below is a real HTTP response.
 */
import { describe, expect, it, beforeAll, afterAll } from 'vitest'
import { Context } from '@deepseek-ai/cordis'
import WebServer from '@deepseek-ai/dsh-host-webserver'
import type { Bill, BillingUser, Invoice, PendingInvoice } from '@dsh-alioth/billing-alioth'
import * as billingWeb from '../src/index.ts'

interface BillingStandIn {
  service: Record<string, unknown>
  state: {
    /** Thrown verbatim by every action when set (non-Error failure path). */
    failure: unknown
    invoices: Invoice[]
    pending: PendingInvoice[]
  }
}

/** What the carrier reads off `ctx.aliothAuth`. */
function authStandIn(username: string, role: 'admin' | 'user'): Record<string, unknown> {
  return {
    async userForToken(token: string | null) {
      if (token !== 'stub-token') return null
      return { id: `id-${username}`, username, namespace: `U-${username}`, role }
    },
  }
}

function billingStandIn(): BillingStandIn {
  const state: BillingStandIn['state'] = { failure: null, invoices: [], pending: [] }
  const guard = (): void => { if (state.failure !== null) throw state.failure }
  const service: Record<string, unknown> = {
    async getSubscription() {
      guard()
      return null
    },
    async bills() {
      guard()
      return [] as Bill[]
    },
    async invoices() {
      guard()
      return state.invoices
    },
    async pendingInvoices() {
      guard()
      return state.pending
    },
    async subscribe() {
      guard()
      return { userId: 'id-center-admin', plan: 'L1', status: 'active', startedAt: new Date(), renewsAt: new Date() }
    },
    async cancel() {
      guard()
    },
    async payBill(billId: string) {
      guard()
      return {
        id: billId,
        userId: 'id-center-admin',
        period: '2026-09',
        amountCents: 139900,
        status: 'paid',
        createdAt: new Date(),
        paidAt: new Date(),
      }
    },
    async requestInvoice(billId: string, _actor: BillingUser, title: string): Promise<Invoice> {
      guard()
      return {
        id: `inv-${billId}`,
        billId,
        userId: 'id-center-admin',
        title,
        taxId: '',
        status: 'issued',
        requestedAt: new Date(),
        issuedAt: new Date(),
      }
    },
    async issueInvoice(invoiceId: string) {
      guard()
      return state.invoices.find(invoice => invoice.id === invoiceId)
        ?? state.pending.find(invoice => invoice.id === invoiceId)
        ?? null
    },
  }
  return { service, state }
}

function invoice(overrides: Partial<Invoice> & { id: string }): Invoice {
  return {
    billId: 'bill-1',
    userId: 'id-center-admin',
    title: '杭州示例科技',
    taxId: '91330100MA27X00000',
    status: 'pending',
    requestedAt: new Date('2026-09-01T00:00:00Z'),
    issuedAt: null,
    ...overrides,
  }
}

const disposers: Array<() => Promise<void>> = []
let ctx: Context
let billing: BillingStandIn
const cookie = 'alioth_session=stub-token'

beforeAll(async () => {
  ctx = new Context()
  ctx.provide('aliothAuth')
  ctx.set('aliothAuth', authStandIn('center-admin', 'admin') as never)
  billing = billingStandIn()
  ctx.provide('aliothBilling')
  ctx.set('aliothBilling', billing.service as never)
  const webServer = await ctx.plugin(WebServer, { host: '127.0.0.1', port: 0 })
  disposers.push(() => webServer.dispose())
  const carrier = await ctx.plugin(billingWeb, {})
  disposers.push(() => carrier.dispose())
}, 60_000)

afterAll(async () => {
  for (const dispose of disposers.reverse()) {
    await dispose().catch(() => {})
  }
})

describe('admin user center (role=admin branches)', () => {
  const base = (): string => `http://127.0.0.1:${ctx.webServer.port}`

  it('marks the admin account on the overview and counts the pending queue', async () => {
    billing.state.pending = [
      { ...invoice({ id: 'pending-1' }), username: 'center-admin', amountCents: 139900, period: '2026-08' },
    ]
    // The overview counts the account's own not-yet-issued invoices.
    billing.state.invoices = [invoice({ id: 'mine-pending' })]
    const response = await fetch(`${base()}/usercenter`, { headers: { cookie } })
    expect(response.status).toBe(200)
    const html = await response.text()
    expect(html).toContain('center-admin · U-center-admin · 管理员')
    expect(html).toContain('admin（管理员）')
    expect(html).toMatch(/待开具发票<\/dt><dd>1<\/dd>/)
  })

  it('renders the empty admin queue and an empty own-invoice list', async () => {
    billing.state.pending = []
    billing.state.invoices = []
    const html = await (await fetch(`${base()}/usercenter/invoices`, { headers: { cookie } })).text()
    expect(html).toContain('开具队列（管理员）')
    expect(html).toContain('队列为空。')
    expect(html).toContain('暂无发票记录。')
    expect(html).toContain('暂无可开票账单')
  })

  it('lists pending invoices with and without a resolvable username', async () => {
    billing.state.pending = [
      { ...invoice({ id: 'pending-named', taxId: '' }), username: '张会计', amountCents: 139900, period: '2026-08' },
      { ...invoice({ id: 'pending-anon', userId: 'abcdef1234567890' }), amountCents: 499900, period: '2026-09' },
    ]
    const html = await (await fetch(`${base()}/usercenter/invoices`, { headers: { cookie } })).text()
    expect(html).toContain('张会计')
    expect(html).toContain('abcdef12') // userId.slice(0, 8) fallback
    expect(html).toContain('¥1,399')
    expect(html).toContain('¥4,999')
    expect(html).toContain('<td>—</td>') // empty tax id
    expect(html).toContain('91330100MA27X00000')
    expect(html).toContain('action="/api/billing/issue"')
  })

  it('shows the account’s own not-yet-issued invoices', async () => {
    billing.state.invoices = [invoice({ id: 'mine-pending', taxId: '' })]
    const html = await (await fetch(`${base()}/usercenter/invoices`, { headers: { cookie } })).text()
    expect(html).toContain('待开具')
    expect(html).toContain('<td>—</td>')
    expect(html).not.toContain('已开具')
  })
})

describe('billing service failures that are not Errors', () => {
  const base = (): string => `http://127.0.0.1:${ctx.webServer.port}`

  it('surfaces a raw failure string on the JSON and form channels', async () => {
    billing.state.failure = 'aliothBilling.payBill: 支付通道炸了'
    try {
      const json = await fetch(`${base()}/api/billing/pay`, {
        method: 'POST',
        headers: { 'content-type': 'application/json', cookie },
        body: JSON.stringify({ bill: 'bill-1' }),
      })
      expect(json.status).toBe(400)
      const payload: unknown = await json.json()
      expect(payload).toEqual({ error: 'aliothBilling.payBill: 支付通道炸了' })

      const form = await fetch(`${base()}/api/billing/pay`, {
        method: 'POST',
        headers: { 'content-type': 'application/x-www-form-urlencoded', cookie },
        body: new URLSearchParams({ bill: 'bill-1' }).toString(),
        redirect: 'manual',
      })
      expect(form.status).toBe(302)
      expect(decodeURIComponent(String(form.headers.get('location')))).toContain('支付通道炸了')
    } finally {
      billing.state.failure = null
    }
  })
})

describe('web server shape mismatch', () => {
  it('logs and stays unmounted when webServer is not a web server', async () => {
    const logs: string[] = []
    const bogus = new Context()
    bogus.logger.exporter({ levels: { default: 3 }, export: message => { logs.push(JSON.stringify(message)) } })
    bogus.provide('aliothAuth')
    bogus.set('aliothAuth', authStandIn('no-web', 'admin') as never)
    bogus.provide('aliothBilling')
    bogus.set('aliothBilling', billingStandIn().service as never)
    bogus.provide('webServer')

    for (const value of [{}, 'not-a-service']) {
      bogus.set('webServer', value as never)
      const carrier = await bogus.plugin(billingWeb, {})
      await carrier.dispose()
    }
    expect(logs.filter(line => line.includes('shape mismatch'))).toHaveLength(2)
    expect(logs.some(line => line.includes('user center mounted'))).toBe(false)
  }, 30_000)
})
