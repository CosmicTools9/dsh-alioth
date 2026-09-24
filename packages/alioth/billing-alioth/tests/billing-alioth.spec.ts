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
