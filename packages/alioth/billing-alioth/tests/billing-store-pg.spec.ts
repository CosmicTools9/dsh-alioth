/**
 * The Postgres billing store against a throwaway database — the same helper the
 * other DB-backed suites use. The interesting behavior lives in the SQL: the
 * lazy schema bootstrap (first use creates six tables, never the registry),
 * upsert semantics on subscriptions, the usage ledger's additive conflict
 * clause, and the nullable-cost month rollup.
 */
import { describe, expect, it, beforeAll, afterAll } from 'vitest'
import {
  BILLING_SCHEMA,
  createPgBillingStore,
  type BillingStore,
} from '../src/store.ts'
import { acquirePostgres, type PgHandle } from '@dsh-alioth/env-alioth'
import { createTestDatabase, type TestDatabase } from '../../env-alioth/tests/test-db.ts'

describe('postgres billing store', () => {
  let testDb: TestDatabase
  let handle: PgHandle
  let store: BillingStore

  beforeAll(async () => {
    testDb = await createTestDatabase('billingstore')
    handle = await acquirePostgres({ url: testDb.url })
    store = createPgBillingStore(handle.query)
  }, 120_000)

  afterAll(async () => {
    await handle.close().catch(() => {})
    await testDb.dispose()
  })

  it('creates its own tables on first use and never the registry', async () => {
    await store.ensureSchema()
    const tables = await handle.query<{ relname: string }>(
      `SELECT relname FROM pg_class JOIN pg_namespace ON pg_namespace.oid = relnamespace
       WHERE nspname = $1 AND relkind = 'r' ORDER BY relname`,
      [BILLING_SCHEMA],
    )
    expect(tables.rows.map(row => row.relname)).toEqual([
      'audit_log', 'bills', 'invoices', 'orders', 'subscriptions', 'usage_daily',
    ])
    // Second ensure is idempotent.
    await store.ensureSchema()
  })

  it('persists subscriptions with due-period reads and upserts', async () => {
    await handle.query(`DELETE FROM ${BILLING_SCHEMA}.subscriptions`)
    const renewsAt = new Date('2026-11-01T00:00:00.000Z')
    await store.upsertSubscription({
      userId: 'u-pg', plan: 'L1', status: 'active', startedAt: new Date('2026-10-01T00:00:00.000Z'),
      renewsAt, canceledAt: null,
    })
    // Upsert replaces the row (cancel-then-resubscribe reuses the primary key).
    await store.upsertSubscription({
      userId: 'u-pg', plan: 'L1', status: 'canceling', startedAt: new Date('2026-10-01T00:00:00.000Z'),
      renewsAt, canceledAt: new Date('2026-10-20T00:00:00.000Z'),
    })
    const row = await store.getSubscription('u-pg')
    expect(row?.status).toBe('canceling')
    expect(row?.canceledAt?.toISOString()).toBe('2026-10-20T00:00:00.000Z')

    const due = await store.subscriptionsDue(new Date('2026-11-02T00:00:00.000Z'))
    expect(due.map(entry => entry.userId)).toEqual(['u-pg'])
    expect(await store.subscriptionsDue(new Date('2026-10-15T00:00:00.000Z'))).toEqual([])
  })

  it('rolls the usage ledger additively and refuses to fake a cost', async () => {
    await handle.query(`DELETE FROM ${BILLING_SCHEMA}.usage_daily`)
    await store.addUsage({ account: 'U-ada', day: '2026-10-01', model: 'deepseek-chat', tokensIn: 1000, tokensOut: 500, calls: 2, costCents: 0.5 })
    await store.addUsage({ account: 'U-ada', day: '2026-10-01', model: 'deepseek-chat', tokensIn: 500, tokensOut: 100, calls: 1, costCents: 0.25 })
    const priced = await store.usageMonth('U-ada', '2026-10')
    expect(priced).toMatchObject({ tokensIn: 1500, tokensOut: 600, calls: 3, costCents: 0.75 })

    // An unpriced bucket (NULL cost) poisons the month's cost — never 0-faked.
    await store.addUsage({ account: 'U-ada', day: '2026-10-02', model: 'mystery', tokensIn: 10, tokensOut: 10, calls: 1, costCents: null })
    expect((await store.usageMonth('U-ada', '2026-10')).costCents).toBeNull()
    // Other accounts stay isolated.
    expect((await store.usageMonth('U-eve', '2026-10')).calls).toBe(0)
  })

  it('appends audit lines and reads the tail newest-first', async () => {
    await handle.query(`DELETE FROM ${BILLING_SCHEMA}.audit_log`)
    await store.appendAudit({ ts: new Date(), actor: 'op', event: 'bill.paid', target: 'bill:1', evidence: '{}' })
    await store.appendAudit({ ts: new Date(), actor: 'u1', event: 'license.requested', target: 'user:u1', evidence: '' })
    const tail = await store.auditTail(10)
    expect(tail.map(entry => entry.event)).toEqual(['license.requested', 'bill.paid'])
  })

  it('orders and bills round-trip with their lifecycle states', async () => {
    await handle.query(`DELETE FROM ${BILLING_SCHEMA}.orders`)
    await handle.query(`DELETE FROM ${BILLING_SCHEMA}.bills`)
    await store.insertOrder({
      id: 'o1', userId: 'u1', kind: 'subscription-l1', amountCents: 139900, status: 'pending',
      billId: 'b1', note: '', createdAt: new Date(), paidAt: null, fulfilledAt: null,
      serviceStart: new Date('2026-10-01T00:00:00.000Z'), serviceEnd: new Date('2026-10-31T00:00:00.000Z'),
    })
    await store.insertBill({
      id: 'b1', userId: 'u1', period: '2026-10', amountCents: 139900, status: 'unpaid',
      orderId: 'o1', createdAt: new Date(), paidAt: null, refundAmountCents: null,
    })
    const paid = await store.setBillStatus('b1', 'paid', new Date())
    expect(paid?.status).toBe('paid')
    const fulfilled = await store.setOrderStatus('o1', 'fulfilled', new Date(), new Date(), 'trade_no=1')
    expect(fulfilled?.status).toBe('fulfilled')
    expect(fulfilled?.fulfilledAt).not.toBeNull()
    expect((await store.billsFor('u1'))[0]?.status).toBe('paid')
    expect(await store.billForPeriod('u1', '2026-10')).not.toBeNull()
    const refunded = await store.setBillStatus('b1', 'refunded', new Date(), 69950)
    expect(refunded?.status).toBe('refunded')
    expect(refunded?.refundAmountCents).toBe(69950)
  })

  it('assigns invoice numbers through the store round-trip', async () => {
    await handle.query(`DELETE FROM ${BILLING_SCHEMA}.invoices`)
    await store.insertInvoice({
      id: 'inv-1', billId: 'b1', userId: 'u1', title: '抬头', taxId: '', status: 'issued',
      number: 'INV-2026-10-abcdef12', requestedAt: new Date(), issuedAt: new Date(),
    })
    const row = await store.invoiceByBill('b1')
    expect(row?.number).toBe('INV-2026-10-abcdef12')
  })

  it('reads back through a fresh store instance (durability, not a cache)', async () => {
    const fresh = createPgBillingStore(handle.query)
    expect((await fresh.getSubscription('u-pg'))?.status).toBe('canceling')
  })
})
