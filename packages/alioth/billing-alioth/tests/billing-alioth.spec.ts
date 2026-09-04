import { describe, expect, it } from 'vitest'
import { Context } from '@deepseek-ai/cordis'
import * as billing from '../src/index.ts'
import { currentPeriod } from '../src/index.ts'

describe('billing capability (memory provider)', () => {
  it('subscribe → current-period bill materialized; cancel keeps bills', async () => {
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

    // Re-subscribe is idempotent for the current period (no duplicate bill).
    await svc.subscribe('u1')
    expect(await svc.bills('u1')).toHaveLength(1)

    await svc.cancel('u1')
    const canceled = await svc.getSubscription('u1')
    expect(canceled?.status).toBe('canceled')
    expect(await svc.bills('u1')).toHaveLength(1) // bills survive cancellation
    await plugin.dispose()
  })

  it('pay → invoice request → duplicate/guard rails (self-service issuance)', async () => {
    const ctx = new Context()
    const plugin = await ctx.plugin(billing, {})
    const svc = ctx.aliothBilling
    const user = { id: 'u2', role: 'user' as const }

    await svc.subscribe(user.id)
    const bill = (await svc.bills(user.id))[0]!

    // Unpaid bills cannot be invoiced.
    await expect(svc.requestInvoice(bill.id, user, '抬头', '')).rejects.toThrow(/已支付/)

    // Foreign user cannot pay or invoice.
    await expect(svc.payBill(bill.id, { id: 'eve', role: 'user' })).rejects.toThrow(/not your bill/)

    const paid = await svc.payBill(bill.id, user)
    expect(paid.status).toBe('paid')
    expect((await svc.payBill(bill.id, user)).status).toBe('paid') // idempotent

    const invoice = await svc.requestInvoice(bill.id, user, '杭州示例科技', '91330100MA27X00000')
    // Self-service: requesting issues directly — no admin queue exists.
    expect(invoice.status).toBe('issued')
    expect(invoice.issuedAt).not.toBeNull()
    await expect(svc.requestInvoice(bill.id, user, '抬头', '')).rejects.toThrow(/已有发票申请/)
    await expect(svc.requestInvoice(bill.id, user, ' ', '')).rejects.toThrow(/抬头不能为空/)

    // Queue is always empty (no super-admin); issue stays idempotent.
    expect(await svc.pendingInvoices(user)).toHaveLength(0)
    expect((await svc.issueInvoice(invoice.id, user)).status).toBe('issued')
    await plugin.dispose()
  })
})
