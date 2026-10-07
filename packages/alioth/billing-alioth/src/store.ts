/**
 * Billing persistence: the `dsh_alioth_billing` schema, owned by this plugin.
 *
 * The 2026-08-21 decision ("no DB modeling here, the backend lands later") is
 * superseded by the 2026-10 entitlement work: subscriptions/bills/invoices were
 * VOLATILE Maps (a restart silently ate paid state), so the domain now has a
 * real store with two adapters — Postgres in any deployment that mounts
 * env-alioth, in-memory for DB-less trees and tests. The two adapters are what
 * makes the billing seam real instead of hypothetical.
 *
 * The schema never touches the model registry (`isahl_meta` / `dsh_alioth`)
 * or auth (`dsh_alioth_auth`); account ids stay plain text on purpose — a
 * cross-schema FK would make these tables unloadable in a deployment that
 * boots billing without auth.
 * @module @dsh-alioth/billing-alioth/store
 */

import type { SqlFn } from './license-store.ts'

export type { SqlFn } from './license-store.ts'

/** Billing-owned schema (authorizations + the commerce domain + audit). */
export const BILLING_SCHEMA = 'dsh_alioth_billing'

/** L1 subscription lifecycle. `canceling` = 期末生效 window (entitlements stay live until renewsAt). */
export type SubscriptionStatus = 'active' | 'canceling' | 'canceled'

/** Order lifecycle: the commerce unit. Paying its bill fulfills the order. */
export type OrderStatus = 'pending' | 'paid' | 'fulfilled' | 'cancelled' | 'refunded'

/** Bill states gain void/refunded so refunds and cancellation settlement exist. */
export type BillStatus = 'unpaid' | 'paid' | 'void' | 'refunded'

export interface SubscriptionRow {
  readonly userId: string
  readonly plan: 'L1'
  readonly status: SubscriptionStatus
  readonly startedAt: Date
  readonly renewsAt: Date
  /** Set when the user cancels; the executor flips status to 'canceled' at renewsAt. */
  readonly canceledAt: Date | null
}

export interface BillRow {
  readonly id: string
  readonly userId: string
  /** Billing period, 'YYYY-MM'. */
  readonly period: string
  /** CNY cents (L1 = 139900). */
  readonly amountCents: number
  readonly status: BillStatus
  readonly orderId: string | null
  readonly createdAt: Date
  readonly paidAt: Date | null
  /** 折算退款额（分）：refundBill 按剩余服务期折算后落账；未退款为 null。 */
  readonly refundAmountCents: number | null
}

export interface OrderRow {
  readonly id: string
  readonly userId: string
  /** What was bought; L1 subscriptions are the only self-serve SKU today. */
  readonly kind: 'subscription-l1'
  readonly amountCents: number
  readonly status: OrderStatus
  readonly billId: string | null
  /** Free-text channel context (e.g. `wechat-pay:trade_no=…` from an ext adapter). */
  readonly note: string
  readonly createdAt: Date
  readonly paidAt: Date | null
  readonly fulfilledAt: Date | null
  /** 该订单覆盖的服务窗口（退款按剩余期折算的依据）；legacy 行为 null。 */
  readonly serviceStart: Date | null
  readonly serviceEnd: Date | null
}

export interface InvoiceRow {
  readonly id: string
  readonly billId: string
  readonly userId: string
  /** 发票抬头 */
  readonly title: string
  /** 纳税人识别号 */
  readonly taxId: string
  readonly status: 'pending' | 'issued'
  /** Receipt-level invoice number (`INV-YYYYMM-xxxxxxxx`); tax-control integration lands later. */
  readonly number: string | null
  readonly requestedAt: Date
  readonly issuedAt: Date | null
}

/** One model-day usage bucket (account-keyed cost ledger, C2 metering). */
export interface UsageBucket {
  readonly account: string
  /** 'YYYY-MM-DD' (UTC). */
  readonly day: string
  readonly model: string
  readonly tokensIn: number
  readonly tokensOut: number
  readonly calls: number
  /** Estimated cost when every model had a price; null otherwise (never 0-faked). */
  readonly costCents: number | null
}

/** One append-only audit line (C6). */
export interface AuditEntry {
  /** ISO instant (written by the store; readers get it back). */
  readonly ts: Date
  /** Who acted: user id, or a system actor like `channel:wechat-pay` / `operator:cli`. */
  readonly actor: string
  readonly event: string
  /** What it acted on ('bill:…', 'user:…', 'license:…'). */
  readonly target: string
  /** JSON-encoded context (small, human-readable). */
  readonly evidence: string
}

/** Month-to-date usage view for one account (metering + budget enforcement). */
export interface UsageMonthView {
  readonly account: string
  /** 'YYYY-MM'. */
  readonly month: string
  readonly tokensIn: number
  readonly tokensOut: number
  readonly calls: number
  /** null when any model-day bucket lacks a cost (price table absent then) — never a fabricated 0. */
  readonly costCents: number | null
}

/** The billing provider reads/writes through this; two adapters satisfy it. */
export interface BillingStore {
  ensureSchema(): Promise<void>
  // subscriptions
  getSubscription(userId: string): Promise<SubscriptionRow | null>
  upsertSubscription(row: SubscriptionRow): Promise<void>
  /** Active/canceling subscriptions whose renewsAt has passed (renewal executor input). */
  subscriptionsDue(now: Date): Promise<SubscriptionRow[]>
  // bills
  insertBill(bill: BillRow): Promise<void>
  billById(id: string): Promise<BillRow | null>
  billsFor(userId: string): Promise<BillRow[]>
  billForPeriod(userId: string, period: string): Promise<BillRow | null>
  setBillStatus(id: string, status: BillStatus, paidAt: Date | null, refundAmountCents?: number | null): Promise<BillRow | null>
  allBills(): Promise<BillRow[]>
  // orders
  insertOrder(order: OrderRow): Promise<void>
  orderById(id: string): Promise<OrderRow | null>
  ordersFor(userId: string): Promise<OrderRow[]>
  allOrders(): Promise<OrderRow[]>
  setOrderStatus(id: string, status: OrderStatus, paidAt: Date | null, fulfilledAt: Date | null, note?: string): Promise<OrderRow | null>
  // invoices
  insertInvoice(invoice: InvoiceRow): Promise<void>
  invoicesFor(userId: string): Promise<InvoiceRow[]>
  invoiceByBill(billId: string): Promise<InvoiceRow | null>
  allInvoices(): Promise<InvoiceRow[]>
  setInvoiceIssued(id: string, number: string): Promise<InvoiceRow | null>
  // usage metering
  addUsage(bucket: UsageBucket): Promise<void>
  usageMonth(account: string, month: string): Promise<UsageMonthView>
  // audit
  appendAudit(entry: AuditEntry): Promise<void>
  auditTail(limit: number): Promise<AuditEntry[]>
}

// ── shared parsing helpers (rows arrive as unknown at the durability seam) ──

export function asDate(value: unknown): Date | null {
  if (value instanceof Date) return value
  if (typeof value !== 'string' && typeof value !== 'number') return null
  const parsed = new Date(value)
  return Number.isNaN(parsed.getTime()) ? null : parsed
}

function str(value: unknown, fallback = ''): string {
  return typeof value === 'string' ? value : fallback
}

function num(value: unknown, fallback = 0): number {
  if (typeof value === 'number') return value
  // pg returns bigint aggregates (SUM over bigint) as strings.
  if (typeof value === 'string' && value !== '' && Number.isFinite(Number(value))) return Number(value)
  return fallback
}

const SUB_STATUSES: readonly SubscriptionStatus[] = ['active', 'canceling', 'canceled']
const BILL_STATUSES: readonly BillStatus[] = ['unpaid', 'paid', 'void', 'refunded']
const ORDER_STATUSES: readonly OrderStatus[] = ['pending', 'paid', 'fulfilled', 'cancelled', 'refunded']

export function toSubscription(row: unknown): SubscriptionRow | null {
  if (typeof row !== 'object' || row === null) return null
  const record = row as Record<string, unknown>
  const userId = str(record.user_id)
  const status = str(record.status) as SubscriptionStatus
  const startedAt = asDate(record.started_at)
  const renewsAt = asDate(record.renews_at)
  if (userId === '' || !SUB_STATUSES.includes(status) || startedAt === null || renewsAt === null) return null
  return {
    userId,
    plan: 'L1',
    status,
    startedAt,
    renewsAt,
    canceledAt: asDate(record.canceled_at),
  }
}

export function toBill(row: unknown): BillRow | null {
  if (typeof row !== 'object' || row === null) return null
  const record = row as Record<string, unknown>
  const id = str(record.id)
  const userId = str(record.user_id)
  const period = str(record.period)
  const status = str(record.status) as BillStatus
  const createdAt = asDate(record.created_at)
  if (id === '' || userId === '' || period === '' || !BILL_STATUSES.includes(status) || createdAt === null) return null
  return {
    id,
    userId,
    period,
    amountCents: num(record.amount_cents),
    status,
    orderId: typeof record.order_id === 'string' ? record.order_id : null,
    createdAt,
    paidAt: asDate(record.paid_at),
    refundAmountCents: record.refund_amount_cents === null || record.refund_amount_cents === undefined ? null : num(record.refund_amount_cents),
  }
}

export function toOrder(row: unknown): OrderRow | null {
  if (typeof row !== 'object' || row === null) return null
  const record = row as Record<string, unknown>
  const id = str(record.id)
  const userId = str(record.user_id)
  const status = str(record.status) as OrderStatus
  const createdAt = asDate(record.created_at)
  if (id === '' || userId === '' || !ORDER_STATUSES.includes(status) || createdAt === null) return null
  return {
    id,
    userId,
    kind: 'subscription-l1',
    amountCents: num(record.amount_cents),
    status,
    billId: typeof record.bill_id === 'string' ? record.bill_id : null,
    note: str(record.note),
    createdAt,
    paidAt: asDate(record.paid_at),
    fulfilledAt: asDate(record.fulfilled_at),
    serviceStart: asDate(record.service_start),
    serviceEnd: asDate(record.service_end),
  }
}

export function toInvoice(row: unknown): InvoiceRow | null {
  if (typeof row !== 'object' || row === null) return null
  const record = row as Record<string, unknown>
  const id = str(record.id)
  const billId = str(record.bill_id)
  const userId = str(record.user_id)
  const requestedAt = asDate(record.requested_at)
  if (id === '' || billId === '' || userId === '' || requestedAt === null) return null
  return {
    id,
    billId,
    userId,
    title: str(record.title),
    taxId: str(record.tax_id),
    status: record.status === 'issued' ? 'issued' : 'pending',
    number: typeof record.number === 'string' ? record.number : null,
    requestedAt,
    issuedAt: asDate(record.issued_at),
  }
}

export function toUsageMonth(account: string, month: string, row: unknown): UsageMonthView {
  const record = (typeof row === 'object' && row !== null ? row : {}) as Record<string, unknown>
  const tokensIn = num(record.tokens_in)
  const tokensOut = num(record.tokens_out)
  const calls = num(record.calls)
  const rawCost = record.cost_cents
  return {
    account,
    month,
    tokensIn,
    tokensOut,
    calls,
    // SUM over nullable cost_cents: a single unpriced bucket (NULL) makes the
    // whole month's cost null — the "never 0-fake" rule, applied per month.
    costCents: rawCost === null || rawCost === undefined ? null : num(rawCost),
  }
}

export function toAudit(row: unknown): AuditEntry | null {
  if (typeof row !== 'object' || row === null) return null
  const record = row as Record<string, unknown>
  const actor = str(record.actor)
  const event = str(record.event)
  const ts = asDate(record.ts)
  if (actor === '' || event === '' || ts === null) return null
  return { ts, actor, event, target: str(record.target), evidence: str(record.evidence) }
}

// ── schema ───────────────────────────────────────────────────────────────

/** Idempotent bootstrap: schema + every commerce/usage/audit table. */
export async function ensureBillingSchema(sql: SqlFn): Promise<void> {
  await sql(`
    CREATE SCHEMA IF NOT EXISTS ${BILLING_SCHEMA};
    CREATE TABLE IF NOT EXISTS ${BILLING_SCHEMA}.subscriptions (
      user_id text PRIMARY KEY,
      plan text NOT NULL DEFAULT 'L1',
      status text NOT NULL CHECK (status IN ('active', 'canceling', 'canceled')),
      started_at timestamptz NOT NULL,
      renews_at timestamptz NOT NULL,
      canceled_at timestamptz
    );
    CREATE TABLE IF NOT EXISTS ${BILLING_SCHEMA}.bills (
      id text PRIMARY KEY,
      user_id text NOT NULL,
      period text NOT NULL,
      amount_cents integer NOT NULL,
      status text NOT NULL CHECK (status IN ('unpaid', 'paid', 'void', 'refunded')),
      order_id text,
      created_at timestamptz NOT NULL DEFAULT now(),
      paid_at timestamptz,
      refund_amount_cents integer
    );
    ALTER TABLE ${BILLING_SCHEMA}.bills ADD COLUMN IF NOT EXISTS refund_amount_cents integer;
    CREATE INDEX IF NOT EXISTS bills_user_idx ON ${BILLING_SCHEMA}.bills (user_id);
    CREATE TABLE IF NOT EXISTS ${BILLING_SCHEMA}.orders (
      id text PRIMARY KEY,
      user_id text NOT NULL,
      kind text NOT NULL DEFAULT 'subscription-l1',
      amount_cents integer NOT NULL,
      status text NOT NULL CHECK (status IN ('pending', 'paid', 'fulfilled', 'cancelled', 'refunded')),
      bill_id text,
      note text NOT NULL DEFAULT '',
      created_at timestamptz NOT NULL DEFAULT now(),
      paid_at timestamptz,
      fulfilled_at timestamptz,
      service_start timestamptz,
      service_end timestamptz
    );
    ALTER TABLE ${BILLING_SCHEMA}.orders ADD COLUMN IF NOT EXISTS service_start timestamptz;
    ALTER TABLE ${BILLING_SCHEMA}.orders ADD COLUMN IF NOT EXISTS service_end timestamptz;
    CREATE INDEX IF NOT EXISTS orders_user_idx ON ${BILLING_SCHEMA}.orders (user_id);
    CREATE TABLE IF NOT EXISTS ${BILLING_SCHEMA}.invoices (
      id text PRIMARY KEY,
      bill_id text NOT NULL,
      user_id text NOT NULL,
      title text NOT NULL,
      tax_id text NOT NULL DEFAULT '',
      status text NOT NULL DEFAULT 'pending' CHECK (status IN ('pending', 'issued')),
      number text,
      requested_at timestamptz NOT NULL DEFAULT now(),
      issued_at timestamptz
    );
    CREATE INDEX IF NOT EXISTS invoices_user_idx ON ${BILLING_SCHEMA}.invoices (user_id);
    CREATE TABLE IF NOT EXISTS ${BILLING_SCHEMA}.usage_daily (
      account text NOT NULL,
      day date NOT NULL,
      model text NOT NULL,
      tokens_in bigint NOT NULL DEFAULT 0,
      tokens_out bigint NOT NULL DEFAULT 0,
      calls integer NOT NULL DEFAULT 0,
      cost_cents double precision,
      PRIMARY KEY (account, day, model)
    );
    CREATE TABLE IF NOT EXISTS ${BILLING_SCHEMA}.audit_log (
      seq bigserial PRIMARY KEY,
      ts timestamptz NOT NULL DEFAULT now(),
      actor text NOT NULL,
      event text NOT NULL,
      target text NOT NULL DEFAULT '',
      evidence text NOT NULL DEFAULT ''
    );
  `)
}

// ── Postgres adapter ─────────────────────────────────────────────────────

const iso = (date: Date | null): string | null => (date === null ? null : date.toISOString())

/**
 * Postgres-backed store. Semantics mirror the in-memory one exactly — the
 * service layer must not know which adapter is underneath.
 */
export function createPgBillingStore(sql: SqlFn): BillingStore {
  // Schema bootstrap is lazy and memoized: every method awaits the same
  // readiness promise, so the first use creates the tables and later ones
  // (and concurrent ones) reuse it. A failed bootstrap resets the memo so the
  // next call retries instead of caching the failure.
  let schemaReady: Promise<void> | undefined
  const ready = (): Promise<void> => {
    schemaReady ??= ensureBillingSchema(sql).catch(error => {
      schemaReady = undefined
      throw error
    })
    return schemaReady
  }
  const raw: BillingStore = {
    async ensureSchema() { await ready() },

    async getSubscription(userId) {
      const result = await sql(
        `SELECT user_id, plan, status, started_at, renews_at, canceled_at FROM ${BILLING_SCHEMA}.subscriptions WHERE user_id = $1`,
        [userId],
      )
      return toSubscription(result.rows[0])
    },

    async upsertSubscription(row) {
      await sql(
        `INSERT INTO ${BILLING_SCHEMA}.subscriptions (user_id, plan, status, started_at, renews_at, canceled_at)
         VALUES ($1, $2, $3, $4, $5, $6)
         ON CONFLICT (user_id) DO UPDATE SET plan = EXCLUDED.plan, status = EXCLUDED.status,
           started_at = EXCLUDED.started_at, renews_at = EXCLUDED.renews_at, canceled_at = EXCLUDED.canceled_at`,
        [row.userId, row.plan, row.status, row.startedAt.toISOString(), row.renewsAt.toISOString(), iso(row.canceledAt)],
      )
    },

    async subscriptionsDue(now) {
      const result = await sql(
        `SELECT user_id, plan, status, started_at, renews_at, canceled_at FROM ${BILLING_SCHEMA}.subscriptions
         WHERE status IN ('active', 'canceling') AND renews_at <= $1 ORDER BY renews_at`,
        [now.toISOString()],
      )
      return result.rows.map(toSubscription).filter((row): row is SubscriptionRow => row !== null)
    },

    async insertBill(bill) {
      await sql(
        `INSERT INTO ${BILLING_SCHEMA}.bills (id, user_id, period, amount_cents, status, order_id, created_at, paid_at, refund_amount_cents)
         VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9)`,
        [bill.id, bill.userId, bill.period, bill.amountCents, bill.status, bill.orderId, bill.createdAt.toISOString(), iso(bill.paidAt), bill.refundAmountCents],
      )
    },

    async billById(id) {
      const result = await sql(`SELECT * FROM ${BILLING_SCHEMA}.bills WHERE id = $1`, [id])
      return toBill(result.rows[0])
    },

    async billsFor(userId) {
      const result = await sql(
        `SELECT * FROM ${BILLING_SCHEMA}.bills WHERE user_id = $1 ORDER BY period DESC, created_at DESC`,
        [userId],
      )
      return result.rows.map(toBill).filter((row): row is BillRow => row !== null)
    },

    async billForPeriod(userId, period) {
      const result = await sql(
        `SELECT * FROM ${BILLING_SCHEMA}.bills WHERE user_id = $1 AND period = $2 ORDER BY created_at DESC LIMIT 1`,
        [userId, period],
      )
      return toBill(result.rows[0])
    },

    async setBillStatus(id, status, paidAt, refundAmountCents) {
      await sql(
        `UPDATE ${BILLING_SCHEMA}.bills SET status = $2, paid_at = $3${refundAmountCents === undefined ? '' : ', refund_amount_cents = $4'} WHERE id = $1`,
        refundAmountCents === undefined ? [id, status, iso(paidAt)] : [id, status, iso(paidAt), refundAmountCents],
      )
      const result = await sql(`SELECT * FROM ${BILLING_SCHEMA}.bills WHERE id = $1`, [id])
      return toBill(result.rows[0])
    },

    async allBills() {
      const result = await sql(`SELECT * FROM ${BILLING_SCHEMA}.bills ORDER BY created_at DESC`)
      return result.rows.map(toBill).filter((row): row is BillRow => row !== null)
    },

    async insertOrder(order) {
      await sql(
        `INSERT INTO ${BILLING_SCHEMA}.orders (id, user_id, kind, amount_cents, status, bill_id, note, created_at, paid_at, fulfilled_at, service_start, service_end)
         VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12)`,
        [order.id, order.userId, order.kind, order.amountCents, order.status, order.billId, order.note,
          order.createdAt.toISOString(), iso(order.paidAt), iso(order.fulfilledAt), iso(order.serviceStart), iso(order.serviceEnd)],
      )
    },

    async orderById(id) {
      const result = await sql(`SELECT * FROM ${BILLING_SCHEMA}.orders WHERE id = $1`, [id])
      return toOrder(result.rows[0])
    },

    async ordersFor(userId) {
      const result = await sql(
        `SELECT * FROM ${BILLING_SCHEMA}.orders WHERE user_id = $1 ORDER BY created_at DESC`,
        [userId],
      )
      return result.rows.map(toOrder).filter((row): row is OrderRow => row !== null)
    },

    async allOrders() {
      const result = await sql(`SELECT * FROM ${BILLING_SCHEMA}.orders ORDER BY created_at DESC`)
      return result.rows.map(toOrder).filter((row): row is OrderRow => row !== null)
    },

    async setOrderStatus(id, status, paidAt, fulfilledAt, note) {
      await sql(
        `UPDATE ${BILLING_SCHEMA}.orders SET status = $2, paid_at = COALESCE($3, paid_at),
         fulfilled_at = COALESCE($4, fulfilled_at)${note === undefined ? '' : ', note = $5'} WHERE id = $1`,
        note === undefined ? [id, status, iso(paidAt), iso(fulfilledAt)] : [id, status, iso(paidAt), iso(fulfilledAt), note],
      )
      const result = await sql(`SELECT * FROM ${BILLING_SCHEMA}.orders WHERE id = $1`, [id])
      return toOrder(result.rows[0])
    },

    async insertInvoice(invoice) {
      await sql(
        `INSERT INTO ${BILLING_SCHEMA}.invoices (id, bill_id, user_id, title, tax_id, status, number, requested_at, issued_at)
         VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9)`,
        [invoice.id, invoice.billId, invoice.userId, invoice.title, invoice.taxId, invoice.status,
          invoice.number, invoice.requestedAt.toISOString(), iso(invoice.issuedAt)],
      )
    },

    async invoicesFor(userId) {
      const result = await sql(
        `SELECT * FROM ${BILLING_SCHEMA}.invoices WHERE user_id = $1 ORDER BY requested_at DESC`,
        [userId],
      )
      return result.rows.map(toInvoice).filter((row): row is InvoiceRow => row !== null)
    },

    async invoiceByBill(billId) {
      const result = await sql(`SELECT * FROM ${BILLING_SCHEMA}.invoices WHERE bill_id = $1`, [billId])
      return toInvoice(result.rows[0])
    },

    async allInvoices() {
      const result = await sql(`SELECT * FROM ${BILLING_SCHEMA}.invoices ORDER BY requested_at DESC`)
      return result.rows.map(toInvoice).filter((row): row is InvoiceRow => row !== null)
    },

    async setInvoiceIssued(id, number) {
      await sql(
        `UPDATE ${BILLING_SCHEMA}.invoices SET status = 'issued', number = $2, issued_at = now() WHERE id = $1`,
        [id, number],
      )
      const result = await sql(`SELECT * FROM ${BILLING_SCHEMA}.invoices WHERE id = $1`, [id])
      return toInvoice(result.rows[0])
    },

    async addUsage(bucket) {
      await sql(
        `INSERT INTO ${BILLING_SCHEMA}.usage_daily (account, day, model, tokens_in, tokens_out, calls, cost_cents)
         VALUES ($1, $2, $3, $4, $5, $6, $7)
         ON CONFLICT (account, day, model) DO UPDATE SET
           tokens_in = ${BILLING_SCHEMA}.usage_daily.tokens_in + EXCLUDED.tokens_in,
           tokens_out = ${BILLING_SCHEMA}.usage_daily.tokens_out + EXCLUDED.tokens_out,
           calls = ${BILLING_SCHEMA}.usage_daily.calls + EXCLUDED.calls,
           cost_cents = COALESCE(${BILLING_SCHEMA}.usage_daily.cost_cents, 0) + COALESCE(EXCLUDED.cost_cents, 0)`,
        [bucket.account, bucket.day, bucket.model, bucket.tokensIn, bucket.tokensOut, bucket.calls, bucket.costCents],
      )
    },

    async usageMonth(account, month) {
      const result = await sql(
        `SELECT SUM(tokens_in) AS "tokens_in", SUM(tokens_out) AS "tokens_out", SUM(calls) AS calls,
                CASE WHEN COUNT(*) = COUNT(cost_cents) THEN SUM(cost_cents) END AS cost_cents
         FROM ${BILLING_SCHEMA}.usage_daily WHERE account = $1 AND to_char(day, 'YYYY-MM') = $2`,
        [account, month],
      )
      return toUsageMonth(account, month, result.rows[0])
    },

    async appendAudit(entry) {
      await sql(
        `INSERT INTO ${BILLING_SCHEMA}.audit_log (ts, actor, event, target, evidence) VALUES ($1, $2, $3, $4, $5)`,
        [entry.ts.toISOString(), entry.actor, entry.event, entry.target, entry.evidence],
      )
    },

    async auditTail(limit) {
      const result = await sql(
        `SELECT ts, actor, event, target, evidence FROM ${BILLING_SCHEMA}.audit_log ORDER BY seq DESC LIMIT $1`,
        [limit],
      )
      return result.rows.map(toAudit).filter((row): row is AuditEntry => row !== null)
    },
  }
  // Every method goes through the memoized schema bootstrap: a fresh deployment
  // creates its tables on first use, never at mount (mounting must not touch
  // the database — the same rule the license store follows).
  return Object.fromEntries(
    Object.entries(raw).map(([method, fn]) => [
      method,
      async (...args: unknown[]) => {
        await ready()
        return await (fn as (...inner: unknown[]) => Promise<unknown>)(...args)
      },
    ]),
  ) as unknown as BillingStore
}

// ── in-memory adapter ────────────────────────────────────────────────────

/**
 * In-memory store — tests and DB-less deployments. Grants/state live and die
 * with the process, which is exactly why real deployments use the Postgres one.
 */
export function createMemoryBillingStore(): BillingStore {
  const subscriptions = new Map<string, SubscriptionRow>()
  const bills = new Map<string, BillRow>()
  const orders = new Map<string, OrderRow>()
  const invoices = new Map<string, InvoiceRow>()
  const usage = new Map<string, UsageBucket>()
  const audit: AuditEntry[] = []

  const usageKey = (account: string, day: string, model: string): string => `${account}\u0000${day}\u0000${model}`
  const sortDesc = <T>(rows: T[], at: (row: T) => Date): T[] => [...rows].sort((a, b) => at(b).getTime() - at(a).getTime())

  return {
    async ensureSchema() { /* nothing to create */ },

    async getSubscription(userId) { return subscriptions.get(userId) ?? null },
    async upsertSubscription(row) { subscriptions.set(row.userId, row) },
    async subscriptionsDue(now) {
      return [...subscriptions.values()]
        .filter(row => row.status !== 'canceled' && row.renewsAt.getTime() <= now.getTime())
        .sort((a, b) => a.renewsAt.getTime() - b.renewsAt.getTime())
    },

    async insertBill(bill) { bills.set(bill.id, bill) },
    async billById(id) { return bills.get(id) ?? null },
    async billsFor(userId) {
      return sortDesc([...bills.values()].filter(bill => bill.userId === userId), bill => bill.createdAt)
    },
    async billForPeriod(userId, period) {
      return sortDesc([...bills.values()].filter(bill => bill.userId === userId && bill.period === period),
        bill => bill.createdAt)[0] ?? null
    },
    async setBillStatus(id, status, paidAt, refundAmountCents) {
      const existing = bills.get(id)
      if (existing === undefined) return null
      const updated: BillRow = {
        ...existing,
        status,
        paidAt: paidAt ?? existing.paidAt,
        refundAmountCents: refundAmountCents === undefined ? existing.refundAmountCents : refundAmountCents,
      }
      bills.set(id, updated)
      return updated
    },
    async allBills() { return sortDesc([...bills.values()], bill => bill.createdAt) },

    async insertOrder(order) { orders.set(order.id, order) },
    async orderById(id) { return orders.get(id) ?? null },
    async ordersFor(userId) {
      return sortDesc([...orders.values()].filter(order => order.userId === userId), order => order.createdAt)
    },
    async allOrders() { return sortDesc([...orders.values()], order => order.createdAt) },
    async setOrderStatus(id, status, paidAt, fulfilledAt, note) {
      const existing = orders.get(id)
      if (existing === undefined) return null
      const updated: OrderRow = {
        ...existing,
        status,
        paidAt: paidAt ?? existing.paidAt,
        fulfilledAt: fulfilledAt ?? existing.fulfilledAt,
        note: note ?? existing.note,
      }
      orders.set(id, updated)
      return updated
    },

    async insertInvoice(invoice) { invoices.set(invoice.id, invoice) },
    async invoicesFor(userId) {
      return sortDesc([...invoices.values()].filter(invoice => invoice.userId === userId), invoice => invoice.requestedAt)
    },
    async invoiceByBill(billId) {
      return [...invoices.values()].find(invoice => invoice.billId === billId) ?? null
    },
    async allInvoices() { return sortDesc([...invoices.values()], invoice => invoice.requestedAt) },
    async setInvoiceIssued(id, number) {
      const existing = invoices.get(id)
      if (existing === undefined) return null
      const updated: InvoiceRow = { ...existing, status: 'issued', number, issuedAt: new Date() }
      invoices.set(id, updated)
      return updated
    },

    async addUsage(bucket) {
      const key = usageKey(bucket.account, bucket.day, bucket.model)
      const existing = usage.get(key)
      const costNull = bucket.costCents === null || (existing !== undefined && existing.costCents === null)
      usage.set(key, {
        ...bucket,
        tokensIn: bucket.tokensIn + (existing?.tokensIn ?? 0),
        tokensOut: bucket.tokensOut + (existing?.tokensOut ?? 0),
        calls: bucket.calls + (existing?.calls ?? 0),
        costCents: costNull ? null : (existing?.costCents ?? 0) + (bucket.costCents ?? 0),
      })
    },
    async usageMonth(account, month) {
      const rows = [...usage.values()].filter(row => row.account === account && row.day.startsWith(`${month}-`))
      const tokensIn = rows.reduce((sum, row) => sum + row.tokensIn, 0)
      const tokensOut = rows.reduce((sum, row) => sum + row.tokensOut, 0)
      const calls = rows.reduce((sum, row) => sum + row.calls, 0)
      const costNull = rows.some(row => row.costCents === null)
      return {
        account, month, tokensIn, tokensOut, calls,
        costCents: costNull ? null : rows.reduce((sum, row) => sum + (row.costCents ?? 0), 0),
      }
    },

    async appendAudit(entry) { audit.push(entry) },
    async auditTail(limit) { return [...audit].reverse().slice(0, limit) },
  }
}
