import { describe, expect, it, vi } from 'vitest'
import { Context } from '@deepseek-ai/cordis'
import * as billing from '../src/index.ts'
import { currentPeriod } from '../src/index.ts'
import { createMemoryLicenseStore } from '../src/license-store.ts'

describe('billing capability (memory provider)', () => {
  it('subscribe → order + current-period bill materialized; idempotent re-subscribe', async () => {
    const ctx = new Context()
    const plugin = await ctx.plugin(billing, {})
    const svc = ctx.aliothBilling

    expect(await svc.getSubscription('u1')).toBeNull()
    const sub = await svc.subscribe('u1')
    expect(sub.status).toBe('active')
    const bills = await svc.bills('u1')
    expect(bills).toHaveLength(1)
    expect(bills[0]!.period).toBe(currentPeriod())
    expect(bills[0]!.amountCents).toBe(139900)
    expect(bills[0]!.status).toBe('unpaid')
    // The commerce unit exists and links back to its receivable.
    const orders = await svc.orders('u1')
    expect(orders).toHaveLength(1)
    expect(orders[0]!.status).toBe('pending')
    expect(orders[0]!.billId).toBe(bills[0]!.id)
    expect(bills[0]!.orderId).toBe(orders[0]!.id)

    // Re-subscribe is idempotent while the subscription lives (no duplicate bill).
    await svc.subscribe('u1')
    expect(await svc.bills('u1')).toHaveLength(1)
    expect(await svc.orders('u1')).toHaveLength(1)
    await plugin.dispose()
  })

  it('cancel is AT PERIOD END: canceling keeps entitlements, the executor ends it', async () => {
    const ctx = new Context()
    const plugin = await ctx.plugin(billing, {})
    const svc = ctx.aliothBilling
    await svc.subscribe('u1')

    await svc.cancel('u1')
    const canceling = await svc.getSubscription('u1')
    expect(canceling?.status).toBe('canceling')
    expect(await svc.planOf('u1')).toBe('L1') // still entitled within the period
    expect(await svc.bills('u1')).toHaveLength(1) // bills survive cancellation

    // The renewal sweep past renewsAt ends it.
    const ended = await svc.runRenewals(new Date((canceling?.renewsAt.getTime() ?? 0) + 1000))
    expect(ended.ended).toBe(1)
    expect((await svc.getSubscription('u1'))?.status).toBe('canceled')
    expect(await svc.planOf('u1')).toBe('L0')
    await plugin.dispose()
  })

  it('renewal rolls an active period: next bill + order, renewsAt advanced', async () => {
    const svc = billing.createMemoryBilling()
    const sub = await svc.subscribe('u1')
    const firstBill = (await svc.bills('u1'))[0]!
    await svc.payBill(firstBill.id, { id: 'u1', role: 'user' })

    const rolled = await svc.runRenewals(new Date(sub.renewsAt.getTime() + 1000))
    expect(rolled.renewed).toBe(1)
    const after = await svc.getSubscription('u1')
    expect(after?.status).toBe('active')
    expect(after!.renewsAt.getTime()).toBeGreaterThan(sub.renewsAt.getTime())
    const bills = await svc.bills('u1')
    expect(bills).toHaveLength(2)
    const nextPeriod = currentPeriod(new Date(sub.renewsAt.getTime() + 1000))
    expect(bills.map(bill => bill.period)).toContain(nextPeriod)
    expect(bills.find(bill => bill.period === nextPeriod)?.status).toBe('unpaid')
  })

  it('pay → invoice request lands in the admin queue → admin issues (receipt number)', async () => {
    const ctx = new Context()
    const plugin = await ctx.plugin(billing, {})
    const svc = ctx.aliothBilling
    const user = { id: 'u2', role: 'user' as const }
    const admin = { id: 'op', role: 'admin' as const }

    await svc.subscribe(user.id)
    const bill = (await svc.bills(user.id))[0]!

    // Unpaid bills cannot be invoiced.
    await expect(svc.requestInvoice(bill.id, user, '抬头', '')).rejects.toThrow(/已支付/)

    // Foreign user cannot pay or invoice.
    await expect(svc.payBill(bill.id, { id: 'eve', role: 'user' })).rejects.toThrow(/not your bill/)

    const paid = await svc.payBill(bill.id, user)
    expect(paid.status).toBe('paid')
    expect((await svc.payBill(bill.id, user)).status).toBe('paid') // idempotent
    // Paying the bill fulfills its order (the commerce unit closes).
    const orders = await svc.orders(user.id)
    expect(orders[0]!.status).toBe('fulfilled')
    expect(orders[0]!.fulfilledAt).not.toBeNull()

    // 申请进入管理员开具队列（无超管不变——queue 是 admin 面）。
    const invoice = await svc.requestInvoice(bill.id, user, '杭州示例科技', '91330100MA27X00000')
    expect(invoice.status).toBe('pending')
    expect(invoice.issuedAt).toBeNull()
    expect(invoice.number).toBeNull()
    await expect(svc.requestInvoice(bill.id, user, '抬头', '')).rejects.toThrow(/已有发票申请/)
    await expect(svc.requestInvoice(bill.id, user, ' ', '')).rejects.toThrow(/抬头不能为空/)

    // 队列只对 admin 可读，且带申请者与账单上下文。
    await expect(svc.pendingInvoices(user)).rejects.toThrow(/admin only/)
    const queue = await svc.pendingInvoices(admin)
    expect(queue).toHaveLength(1)
    expect(queue[0]).toMatchObject({ billId: bill.id, amountCents: 139900, period: currentPeriod() })

    // 开具是 admin 动作：pending → issued（收据级票号），幂等。
    await expect(svc.issueInvoice(invoice.id, user)).rejects.toThrow(/admin only/)
    const issued = await svc.issueInvoice(invoice.id, admin)
    expect(issued.status).toBe('issued')
    expect(issued.number).toMatch(/^INV-\d{4}-\d{2}-[0-9a-f]{8}$/)
    expect(issued.issuedAt).not.toBeNull()
    expect((await svc.issueInvoice(invoice.id, admin)).status).toBe('issued') // idempotent
    expect(await svc.pendingInvoices(admin)).toHaveLength(0)
    await plugin.dispose()
  })
})

describe('refund / void / channel settlement (C4 payment seam)', () => {
  it('refund and void are admin-only and drive bill + order together', async () => {
    const svc = billing.createMemoryBilling()
    const user = { id: 'u3', role: 'user' as const }
    const admin = { id: 'op', role: 'admin' as const }
    await svc.subscribe(user.id)
    const bill = (await svc.bills(user.id))[0]!

    await expect(svc.refundBill(bill.id, user)).rejects.toThrow(/admin only/)
    await expect(svc.refundBill(bill.id, admin)).rejects.toThrow(/仅已支付/)
    await svc.payBill(bill.id, user)
    const refunded = await svc.refundBill(bill.id, admin)
    expect(refunded.status).toBe('refunded')
    expect((await svc.orders(user.id))[0]!.status).toBe('refunded')

    // Void works on unpaid bills only; the order is cancelled with it.
    await svc.subscribe(user.id)
    await svc.runRenewals(new Date(Date.now() + 31 * 24 * 3600 * 1000))
    const second = (await svc.bills(user.id)).find(candidate => candidate.status === 'unpaid')!
    const voided = await svc.voidBill(second.id, admin)
    expect(voided.status).toBe('void')
    expect((await svc.orders(user.id)).find(order => order.billId === second.id)?.status).toBe('cancelled')
  })

  it('channel settlement pays the bill when the amount matches, refuses a mismatch', async () => {
    const svc = billing.createMemoryBilling()
    const user = { id: 'u4', role: 'user' as const }
    await svc.subscribe(user.id)
    const bill = (await svc.bills(user.id))[0]!

    await expect(svc.applyChannelPayment('wechat-pay', bill.id, 999, 'bad')).rejects.toThrow(/amount mismatch/)
    const paid = await svc.applyChannelPayment('wechat-pay', bill.id, bill.amountCents, 'trade_no=4200')
    expect(paid.status).toBe('paid')
    expect((await svc.orders(user.id))[0]!.status).toBe('fulfilled')
    // Second callback is an idempotent no-op, not an error.
    expect((await svc.applyChannelPayment('wechat-pay', bill.id, bill.amountCents, 'replay')).status).toBe('paid')
  })

  it('refund is PRO-RATED over the remaining service window (injected clock)', async () => {
    let nowMs = Date.UTC(2026, 9, 1, 12, 0, 0)
    const svc = billing.createMemoryBilling({ now: () => new Date(nowMs) })
    const admin = { id: 'op', role: 'admin' as const }
    const user = { id: 'u7', role: 'user' as const }
    await svc.subscribe(user.id) // 服务窗口 = T0 .. T0+30d
    const bill = (await svc.bills(user.id))[0]!
    await svc.payBill(bill.id, user)

    // 半程退款：剩余 15d / 30d → ⌊139900 / 2⌋；服务随退款终止。
    nowMs += 15 * 24 * 3600 * 1000
    const refunded = await svc.refundBill(bill.id, admin)
    expect(refunded.status).toBe('refunded')
    expect(refunded.refundAmountCents).toBe(69950)
    expect((await svc.orders(user.id))[0]!.status).toBe('refunded')
    expect((await svc.getSubscription(user.id))?.status).toBe('canceled')

    // 重订 + 期末之后退款：剩余为 0 → 折算额为 0（不是全额，也不是猜的数）。
    nowMs += 24 * 3600 * 1000
    const resumed = await svc.subscribe(user.id)
    const nextBill = (await svc.bills(user.id)).find(candidate => candidate.status === 'unpaid')!
    await svc.payBill(nextBill.id, user)
    nowMs = resumed.renewsAt.getTime() + 24 * 3600 * 1000 // 服务窗口已走完
    const expired = await svc.refundBill(nextBill.id, admin)
    expect(expired.refundAmountCents).toBe(0)
  })

  it('reconcile reports tallies and stuck orders (admin only)', async () => {
    const svc = billing.createMemoryBilling()
    const admin = { id: 'op', role: 'admin' as const }
    const user = { id: 'u5', role: 'user' as const }
    await svc.subscribe(user.id)
    await expect(svc.reconcile(user)).rejects.toThrow(/admin only/)
    const report = await svc.reconcile(admin)
    expect(report.bills.total).toBe(1)
    expect(report.orders.pending).toBe(1)
    expect(report.anomalies).toHaveLength(0)
  })
})

describe('entitlement decision point (C1)', () => {
  it('source-download follows the L2 license, not the subscription', async () => {
    const svc = billing.createMemoryBilling({
      sourceLicenses: billing.parseSourceLicenses('ada:2020-12-31'),
      resolveUsername: async userId => (userId === 'id-ada' ? 'ada' : null),
    })
    const ada = { id: 'id-ada', role: 'user' as const }
    await svc.subscribe(ada.id) // L1 must NOT unlock source

    const denied = await svc.entitlement(ada, 'source-download')
    expect(denied.allowed).toBe(false) // the configured window is in the past
    expect(denied.reason).toBe('L2 authorization expired')
    expect(denied.plan).toBe('L1')
    await svc.grantSourceLicense(ada.id, new Date('2099-01-01T00:00:00.000Z'))
    const allowed = await svc.entitlement(ada, 'source-download')
    expect(allowed.allowed).toBe(true)
    expect(allowed.until?.toISOString()).toBe('2099-01-01T00:00:00.000Z')
  })

  it('llm-budget reports the configured quota per plan and null when unset', async () => {
    const configured = billing.createMemoryBilling({ llmMonthlyCostCentsL0: 100, llmMonthlyCostCentsL1: 5000 })
    const free = { id: 'free', role: 'user' as const }
    expect((await configured.entitlement(free, 'llm-budget')).monthlyCostCents).toBe(100)
    await configured.subscribe(free.id)
    expect((await configured.entitlement(free, 'llm-budget')).monthlyCostCents).toBe(5000)

    const unset = billing.createMemoryBilling()
    expect((await unset.entitlement(free, 'llm-budget')).monthlyCostCents).toBeNull()
    expect((await unset.entitlement(free, 'llm-budget')).reason).toContain('not capped')
  })
})

describe('metering ledger + audit trail (C2/C6)', () => {
  it('usage accumulates per account/day/model; a missing price poisons the month cost', async () => {
    const svc = billing.createMemoryBilling()
    await svc.recordUsage('U-ada', [
      { day: '2026-10-01', model: 'deepseek-chat', tokensIn: 1000, tokensOut: 500, calls: 2, costCents: 0.5 },
      { day: '2026-10-02', model: 'deepseek-chat', tokensIn: 2000, tokensOut: 100, calls: 1, costCents: 0.5 },
    ])
    const month = await svc.usageMonthly('U-ada')
    expect(month.tokensIn).toBe(3000)
    expect(month.tokensOut).toBe(600)
    expect(month.calls).toBe(3)
    expect(month.costCents).toBe(1)

    await svc.recordUsage('U-ada', [
      { day: '2026-10-03', model: 'mystery-model', tokensIn: 10, tokensOut: 10, calls: 1, costCents: null },
    ])
    expect((await svc.usageMonthly('U-ada')).costCents).toBeNull() // never 0-faked
  })

  it('audit is append-only and its tail is admin-readable', async () => {
    const svc = billing.createMemoryBilling()
    const admin = { id: 'op', role: 'admin' as const }
    const user = { id: 'u6', role: 'user' as const }
    await svc.subscribe(user.id)
    const bill = (await svc.bills(user.id))[0]!
    await svc.payBill(bill.id, user)
    await expect(svc.auditTail(user)).rejects.toThrow(/admin only/)
    const tail = await svc.auditTail(admin)
    const events = tail.map(entry => entry.event)
    expect(events).toContain('subscription.started')
    expect(events).toContain('bill.created')
    expect(events).toContain('bill.paid')
  })
})

describe('notifications (optional webhook)', () => {
  it('POSTs billing events fire-and-forget; failures never break the flow', async () => {
    const { createServer } = await import('node:http')
    const received: { event: string }[] = []
    const server = createServer((request, response) => {
      let body = ''
      request.on('data', chunk => { body += String(chunk) })
      request.on('end', () => {
        received.push({ event: (JSON.parse(body || '{}') as { event: string }).event })
        response.end('ok')
      })
    })
    await new Promise<void>(resolve => server.listen(0, '127.0.0.1', resolve))
    const port = (server.address() as { port: number }).port
    try {
      const svc = billing.createMemoryBilling({ notifyWebhookUrl: `http://127.0.0.1:${port}/hook` })
      await svc.subscribe('u7')
      const bill = (await svc.bills('u7'))[0]!
      await svc.payBill(bill.id, { id: 'u7', role: 'user' })
      await new Promise(resolve => setTimeout(resolve, 50))
      expect(received.map(entry => entry.event)).toEqual(['subscription.started', 'bill.created', 'bill.paid'])
    } finally {
      server.close()
    }
  })

  it('an unreachable webhook is logged, never thrown', async () => {
    const onError = vi.fn()
    const svc = billing.createMemoryBilling({ notifyWebhookUrl: 'http://127.0.0.1:9/nope', onError })
    await svc.subscribe('u8') // resolves without throwing
    await new Promise(resolve => setTimeout(resolve, 20))
    expect(onError).toHaveBeenCalled()
  })
})

describe('L2 source authorizations', () => {
  it('parses the operator list, failing loud on a malformed entry', () => {
    const parsed = billing.parseSourceLicenses('ada:2026-12-31, grace:2027-06-30 ')
    expect([...parsed.keys()]).toEqual(['ada', 'grace'])
    // Date-only means the whole day, not its midnight start.
    expect(parsed.get('ada')?.toISOString()).toBe('2026-12-31T23:59:59.999Z')

    expect(billing.parseSourceLicenses('')).toEqual(new Map())
    expect(() => billing.parseSourceLicenses('ada')).toThrow(/username:YYYY-MM-DD/)
    expect(() => billing.parseSourceLicenses('ada:soon')).toThrow(/username:YYYY-MM-DD/)
  })

  it('serves the configured authorization and lets an explicit grant outrank it', async () => {
    // The provider resolves user ids to usernames through the harness account store
    // (the plugin wires that up); here the lookup is injected directly.
    const svc = billing.createMemoryBilling({
      sourceLicenses: billing.parseSourceLicenses('ada:2026-12-31'),
      resolveUsername: async userId => (userId === 'id-ada' ? 'ada' : null),
    })

    expect((await svc.sourceLicense('id-ada'))?.until.toISOString()).toBe('2026-12-31T23:59:59.999Z')
    expect((await svc.sourceLicense('id-ada'))?.grantedBy).toBe('operator-config')
    expect(await svc.sourceLicense('id-eve')).toBeNull()

    // An L1 subscription is not an input: it must not produce a source authorization.
    await svc.subscribe('id-eve')
    expect(await svc.sourceLicense('id-eve')).toBeNull()

    const granted = await svc.grantSourceLicense('id-eve', new Date('2027-01-15T00:00:00.000Z'))
    expect(granted).toMatchObject({ grantedBy: 'grant' })
    expect((await svc.sourceLicense('id-eve'))?.until.toISOString()).toBe('2027-01-15T00:00:00.000Z')
  })
})

describe('L2 authorizations resolve through the auth capability', () => {
  it('maps an account id to the operator list, and grants nothing without it', async () => {
    // Without the auth capability the configured list cannot be matched — and that
    // must never be read as a grant.
    const bare = new Context()
    const barePlugin = await bare.plugin(billing, { sourceLicenses: 'ada:2026-12-31' })
    expect(await bare.aliothBilling.sourceLicense('id-ada')).toBeNull()
    await barePlugin.dispose()

    // With it, the same list resolves by the account's username.
    const ctx = new Context()
    ctx.provide('aliothAuth')
    ctx.set('aliothAuth', {
      userById: async (id: string) => (id === 'id-ada' ? { username: 'ada' } : null),
    } as never)
    const plugin = await ctx.plugin(billing, { sourceLicenses: 'ada:2026-12-31' })

    expect((await ctx.aliothBilling.sourceLicense('id-ada'))?.until.toISOString()).toBe('2026-12-31T23:59:59.999Z')
    expect((await ctx.aliothBilling.sourceLicense('id-ada'))?.grantedBy).toBe('operator-config')
    expect(await ctx.aliothBilling.sourceLicense('id-unknown')).toBeNull()
    await plugin.dispose()
  })
})

describe('durable billing store (Postgres adapter)', () => {
  it('is exercised in billing-store-pg.spec.ts against a throwaway database', () => {
    // Kept DB-less here so `pnpm test` works on every machine; the PG path has
    // its own spec following the env-alioth throwaway-database helper.
    expect(true).toBe(true)
  })
})

describe('license store (memory path)', () => {
  it('request → grant → pending pipeline', async () => {
    const licenses = createMemoryLicenseStore()
    const before = await licenses.request('u10')
    expect(before.grantedAt).toBeNull()
    expect(await licenses.pending()).toHaveLength(1)
    const granted = await licenses.grant('u10', new Date('2027-03-01T00:00:00.000Z'), '合同 2026-115')
    expect(granted.until?.toISOString()).toBe('2027-03-01T00:00:00.000Z')
    expect(await licenses.pending()).toHaveLength(0)
    // Re-request never shortens a granted window.
    expect((await licenses.request('u10')).until?.toISOString()).toBe('2027-03-01T00:00:00.000Z')
  })
})
