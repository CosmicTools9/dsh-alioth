/**
 * `@dsh-alioth/billing-alioth` — the billing CAPABILITY for the B/S
 * deployment: `ctx.aliothBilling` owns the commerce domain (orders,
 * subscription lifecycle, monthly bills, invoices, L2 authorizations), the
 * single entitlement decision point, the account-keyed usage ledger, and the
 * append-only audit trail.
 *
 * The interface IS the seam the 2026-08-21 note promised ("a backend
 * integration replaces the provider, same interface"). It now has two real
 * adapters at the store level: Postgres (`dsh_alioth_billing`, any deployment
 * with env-alioth) and in-memory (DB-less trees, tests) — see `store.ts`.
 *
 * Payment: the `manual` channel (线下确认) stays the self-serve path; external
 * PSP channels integrate through `applyChannelPayment` — an ext adapter (the
 * NS:Cosmic-Tools `Ext-adapter` face, e.g. wechat-pay) verifies its own
 * signature at its HTTP edge and calls this with the settled reference.
 *
 * Entitlement: `entitlement(actor, capability)` is THE decision point — the
 * source-download gate, the metering budget, and admin surfaces read it
 * instead of each re-deriving plan rules from raw rows.
 * @module @dsh-alioth/billing-alioth
 */

import { randomUUID } from 'node:crypto'
import { Context } from '@deepseek-ai/cordis'
import {
  createMemoryLicenseStore,
  createPgLicenseStore,
  ensureLicenseSchema,
  type LicenseStore,
  type LicenseView,
  type SqlFn,
} from './license-store.ts'
import {
  createMemoryBillingStore,
  createPgBillingStore,
  type AuditEntry,
  type BillRow,
  type BillingStore,
  type InvoiceRow,
  type OrderRow,
  type OrderStatus,
  type SubscriptionRow,
  type UsageMonthView,
} from './store.ts'
import z from '@deepseek-ai/schemastery'

export const name = 'billing-alioth'
export const inject: readonly string[] = []

/** L1 subscription price in CNY cents (confirmed pricing ladder). */
export const L1_AMOUNT_CENTS = 139900

/** Renewal sweep cadence (the executor that turns renewsAt into real work). */
export const RENEWAL_INTERVAL_MS = 15 * 60 * 1000
/** 30-day billing period (matches the L1 ladder's 月付). */
const PERIOD_MS = 30 * 24 * 3600 * 1000

export {
  createMemoryLicenseStore,
  createPgLicenseStore,
  ensureLicenseSchema,
  LICENSE_SCHEMA,
  type LicenseStore,
  type LicenseView,
} from './license-store.ts'
export {
  BILLING_SCHEMA,
  createMemoryBillingStore,
  createPgBillingStore,
  ensureBillingSchema,
  type AuditEntry,
  type BillRow,
  type BillingStore,
  type InvoiceRow,
  type OrderRow,
  type OrderStatus,
  type SubscriptionRow,
  type UsageBucket,
  type UsageMonthView,
} from './store.ts'

export interface Config {
  /**
   * Operator-configured L2 source-download authorizations, `username:YYYY-MM-DD`
   * separated by commas (env `ALIOTH_SOURCE_LICENSES`, injected by the bundle).
   * L2 is 商务对接 — it has no self-serve path, so the deployment writes it down
   * until a back-office owns it. A malformed entry fails loud at mount.
   */
  readonly sourceLicenses?: string
  /**
   * Monthly LLM cost budget (cents) per plan for the metering seam. Absent —
   * the default — means "no budget configured": cost is still metered, but the
   * guard does not judge a cap (it must never mistake "unset" for 0).
   */
  readonly llmMonthlyCostCentsL0?: number
  readonly llmMonthlyCostCentsL1?: number
  /**
   * Optional HTTPS webhook: billing events (bill.created / bill.paid /
   * bill.refunded / license.granted / invoice.issued) POST there fire-and-forget.
   * Failures are logged, never thrown into the billing flow.
   */
  readonly notifyWebhookUrl?: string
}

export const Config: z<Config> = z.object({
  sourceLicenses: z.string().default(''),
  llmMonthlyCostCentsL0: z.number(),
  llmMonthlyCostCentsL1: z.number(),
  notifyWebhookUrl: z.string().default(''),
})

export interface BillingUser {
  readonly id: string
  readonly role: 'admin' | 'user'
}

/** Subscription lifecycle: `canceling` keeps entitlements live until renewsAt. */
export type SubscriptionStatus = SubscriptionRow['status']

export interface Subscription {
  userId: string
  plan: 'L1'
  status: SubscriptionStatus
  startedAt: Date
  renewsAt: Date
  canceledAt: Date | null
}

/**
 * An L2 source-download authorization. Deliberately NOT the L1 subscription: the
 * published ladder sells source at L2 (¥4,999 起, 商务对接), so an L1 subscriber
 * must not reach the source package.
 */
export interface SourceLicense {
  readonly userId: string
  /** Inclusive end of the negotiated window. */
  readonly until: Date
  /** `operator-config` = deployment's `ALIOTH_SOURCE_LICENSES`; `grant` = explicit call. */
  readonly grantedBy: 'operator-config' | 'grant'
}

export interface Bill {
  id: string
  userId: string
  /** Billing period, 'YYYY-MM'. */
  period: string
  /** CNY cents (L1 = 139900). */
  amountCents: number
  status: BillRow['status']
  orderId: string | null
  createdAt: Date
  paidAt: Date | null
}

export interface Invoice {
  id: string
  billId: string
  userId: string
  /** 发票抬头 */
  title: string
  /** 纳税人识别号 */
  taxId: string
  status: 'pending' | 'issued'
  /** 收据级票号（税控对接前的占位格式 `INV-YYYYMM-xxxxxxxx`）。 */
  number: string | null
  requestedAt: Date
  issuedAt: Date | null
}

export interface Order {
  id: string
  userId: string
  kind: 'subscription-l1'
  amountCents: number
  status: OrderStatus
  billId: string | null
  note: string
  createdAt: Date
  paidAt: Date | null
  fulfilledAt: Date | null
}

/** The capabilities the single decision point knows. */
export type EntitlementCapability = 'source-download' | 'llm-budget'

export interface EntitlementDecision {
  readonly capability: EntitlementCapability
  readonly allowed: boolean
  /** Effective plan: an active (or canceling, within the period) L1 row. */
  readonly plan: 'L0' | 'L1'
  readonly reason: string
  /** source-download: inclusive end of the licensed window; else null. */
  readonly until: Date | null
  /** llm-budget: monthly cost budget in cents; null = deployment configured none. */
  readonly monthlyCostCents: number | null
}

/** One metering flush entry (day-bucketed; cost null when unpriced). */
export interface UsageFlushEntry {
  readonly day: string
  readonly model: string
  readonly tokensIn: number
  readonly tokensOut: number
  readonly calls: number
  readonly costCents: number | null
}

export interface ReconciliationAnomaly {
  readonly kind: 'order-stuck-pending' | 'bill-paid-without-order' | 'order-without-bill'
  readonly detail: string
}

export interface ReconciliationReport {
  readonly bills: { total: number; unpaid: number; paid: number; void: number; refunded: number }
  readonly orders: { total: number; pending: number; paid: number; fulfilled: number; cancelled: number; refunded: number }
  readonly anomalies: readonly ReconciliationAnomaly[]
}

/** Admin queue row: invoice + requestor + amount context. */
export type PendingInvoice = Invoice & { username?: string; amountCents: number; period: string }

export interface AliothBillingService {
  /** The user's subscription, null when on the free L0 tier. */
  getSubscription(userId: string): Promise<Subscription | null>
  /** Activate (or re-activate) L1: order + current-period bill materialized. Idempotent while active. */
  subscribe(userId: string): Promise<Subscription>
  /** Cancel AT PERIOD END — status `canceling`, entitlements live until renewsAt. */
  cancel(userId: string): Promise<void>
  bills(userId: string): Promise<Bill[]>
  orders(userId: string): Promise<Order[]>
  /** Mark a bill paid (manual 线下确认; the owner — or an admin). Fulfills the linked order. */
  payBill(billId: string, actor: BillingUser): Promise<Bill>
  /** Admin: paid → refunded (the linked order follows). */
  refundBill(billId: string, actor: BillingUser): Promise<Bill>
  /** Admin: unpaid → void (cancellation settlement; the linked order is cancelled). */
  voidBill(billId: string, actor: BillingUser): Promise<Bill>
  /**
   * External channel settlement (ext-adapter contract): the channel's HTTP edge
   * verified its own signature, then calls this with the settled bill reference.
   * Amount must match — a mismatched amount is refused, not silently accepted.
   */
  applyChannelPayment(channel: string, billReference: string, amountCents: number, note: string): Promise<Bill>
  invoices(userId: string): Promise<Invoice[]>
  /** Request an invoice (发票抬头 + 纳税人识别号) for a PAID bill — one per bill, issued immediately. */
  requestInvoice(billId: string, actor: BillingUser, title: string, taxId: string): Promise<Invoice>
  /** Admin queue: all pending invoices (requestor usernames when resolvable). */
  pendingInvoices(actor: BillingUser): Promise<PendingInvoice[]>
  /** Admin action: mark a pending invoice issued (assigns the receipt number). */
  issueInvoice(invoiceId: string, actor: BillingUser): Promise<Invoice>
  /** Admin: every order (the ops back-office view). */
  allOrders(actor: BillingUser): Promise<Order[]>
  /** Admin: append-only audit trail tail, newest first. */
  auditTail(actor: BillingUser, limit?: number): Promise<AuditEntry[]>
  /** Admin: cash/order consistency sweep (stuck orders, paid bills without orders). */
  reconcile(actor: BillingUser): Promise<ReconciliationReport>
  /**
   * THE entitlement decision point. Source-download reads this instead of raw
   * license rows; metering reads the budget instead of guessing plan rules.
   */
  entitlement(actor: BillingUser, capability: EntitlementCapability): Promise<EntitlementDecision>
  /** Effective plan for an account (L1 while active or canceling within the period). */
  planOf(userId: string): Promise<'L0' | 'L1'>
  /**
   * L2 authorization for source download, or null. This — not `getSubscription` —
   * is what the source gate reads, because the ladder prices source as its own
   * tier (商务对接开通).
   */
  sourceLicense(userId: string): Promise<SourceLicense | null>
  /** Operator action: grant/replace an L2 authorization. A back-office or PSP lands here. */
  grantSourceLicense(userId: string, until: Date, note?: string): Promise<SourceLicense>
  /**
   * The account asks for L2 (用户中心「申请」). Idempotent: asking again never
   * shortens or clears a window that was already granted.
   */
  requestSourceLicense(userId: string): Promise<LicenseView>
  /**
   * Read-only view of this account's request/grant row (`null` when it never
   * asked) — the user center renders 未申请 / 申请中 / 已开通至 … from it.
   */
  sourceLicenseRequest(userId: string): Promise<LicenseView | null>
  /** Metering (C2): persist one day-bucket batch for an account. */
  recordUsage(account: string, entries: readonly UsageFlushEntry[]): Promise<void>
  /** Metering: month-to-date usage + cost for one account. */
  usageMonthly(account: string): Promise<UsageMonthView>
  /** Audit seam (C6): one append-only line. Best-effort — failures surface as logs, never break the flow. */
  audit(actor: string, event: string, target?: string, evidence?: string): Promise<void>
  /**
   * Renewal executor: flip expired `canceling` rows to `canceled`, roll active
   * periods (next bill + order, renewsAt advanced). Runs on an interval; public
   * so tests and operators can drive it deterministically.
   */
  runRenewals(now?: Date): Promise<{ renewed: number; ended: number }>
}

declare module '@deepseek-ai/cordis' {
  interface Context {
    aliothBilling: AliothBillingService
  }
}

/** Current billing period as 'YYYY-MM' (UTC — deterministic across TZs). */
export function currentPeriod(now: Date = new Date()): string {
  return now.toISOString().slice(0, 7)
}

/** Receipt-level invoice number for one bill (stable, derived from the invoice id). */
export function invoiceNumberFor(period: string, invoiceId: string): string {
  return `INV-${period}-${invoiceId.replace(/-/g, '').slice(0, 8)}`
}

/** Options for `createBilling` — deployment wiring plus test seam. */
export interface BillingOptions {
  readonly store: BillingStore
  readonly licenses: LicenseStore
  readonly resolveUsername?: ((userId: string) => Promise<string | null>) | undefined
  /** Operator-configured L2 authorizations keyed by username (a recorded grant wins over this). */
  readonly sourceLicenses?: ReadonlyMap<string, Date> | undefined
  readonly llmMonthlyCostCentsL0?: number | undefined
  readonly llmMonthlyCostCentsL1?: number | undefined
  readonly notifyWebhookUrl?: string | undefined
  /** Where to surface background failures (renewals, notifications). */
  readonly onError?: ((message: string) => void) | undefined
}

/**
 * The billing service over a store. All domain rules live HERE (one place) —
 * the store adapters only persist. The single implementation per rule is what
 * keeps 订阅/账单/发票/订单/权益 from drifting apart across callers.
 */
export function createBilling(opts: BillingOptions): AliothBillingService {
  const store = opts.store
  const licenses = opts.licenses
  const onError = opts.onError ?? (() => {})

  const audit = async (actor: string, event: string, target = '', evidence = ''): Promise<void> => {
    try {
      await store.appendAudit({ ts: new Date(), actor, event, target, evidence })
    } catch (error) {
      onError(`audit write failed (${event}): ${error instanceof Error ? error.message : String(error)}`)
    }
  }

  const notify = (event: string, payload: Record<string, unknown>): void => {
    const url = opts.notifyWebhookUrl
    if (url === undefined || url === '') return
    void fetch(url, {
      method: 'POST',
      headers: { 'content-type': 'application/json' },
      body: JSON.stringify({ event, ts: new Date().toISOString(), ...payload }),
      signal: AbortSignal.timeout(5_000),
    }).catch(error => {
      onError(`notify webhook failed (${event}): ${error instanceof Error ? error.message : String(error)}`)
    })
  }

  const rowToSubscription = (row: SubscriptionRow): Subscription => ({
    userId: row.userId,
    plan: row.plan,
    status: row.status,
    startedAt: row.startedAt,
    renewsAt: row.renewsAt,
    canceledAt: row.canceledAt,
  })

  const rowToBill = (row: BillRow): Bill => ({ ...row })
  const rowToOrder = (row: OrderRow): Order => ({ ...row })
  const rowToInvoice = (row: InvoiceRow): Invoice => ({ ...row })

  const planOf = async (userId: string): Promise<'L0' | 'L1'> => {
    const row = await store.getSubscription(userId)
    return row !== null && row.status !== 'canceled' && row.renewsAt.getTime() > Date.now() ? 'L1' : 'L0'
  }

  const sourceLicense = async (userId: string): Promise<SourceLicense | null> => {
    // A recorded grant is the negotiated truth and outranks the operator's list.
    const row = await licenses.read(userId)
    if (row !== null && row.grantedAt !== null && row.until !== null) {
      return { userId, until: row.until, grantedBy: 'grant' }
    }
    const configured = opts.sourceLicenses
    if (configured === undefined || configured.size === 0) return null
    const username = await opts.resolveUsername?.(userId) ?? null
    const until = username === null ? undefined : configured.get(username)
    return until === undefined ? null : { userId, until, grantedBy: 'operator-config' }
  }

  /** Create the period's order+bill pair (order is the commerce unit, bill its receivable). */
  const materializePeriod = async (userId: string, period: string): Promise<void> => {
    const existing = await store.billForPeriod(userId, period)
    if (existing !== null && existing.status !== 'void') {
      return
    }
    const orderId = randomUUID()
    const billId = randomUUID()
    await store.insertOrder({
      id: orderId, userId, kind: 'subscription-l1', amountCents: L1_AMOUNT_CENTS,
      status: 'pending', billId, note: '', createdAt: new Date(), paidAt: null, fulfilledAt: null,
    })
    await store.insertBill({
      id: billId, userId, period, amountCents: L1_AMOUNT_CENTS, status: 'unpaid',
      orderId, createdAt: new Date(), paidAt: null,
    })
    await audit(userId, 'bill.created', `bill:${billId}`, JSON.stringify({ period, order: orderId }))
    notify('bill.created', { userId, period, billId, amountCents: L1_AMOUNT_CENTS })
  }

  /** Shared unpaid→paid transition (manual channel + ext channel both land here). */
  const settleBill = async (billId: string, actor: string, note?: string): Promise<Bill> => {
    const row = await store.billById(billId)
    if (row === null) throw new Error('aliothBilling.payBill: bill not found')
    if (row.status === 'paid') return rowToBill(row) // idempotent
    if (row.status !== 'unpaid') {
      throw new Error(`aliothBilling.payBill: bill is ${row.status} and can no longer be paid`)
    }
    const paid = await store.setBillStatus(billId, 'paid', new Date())
    if (paid === null) throw new Error('aliothBilling.payBill: bill vanished while paying')
    if (row.orderId !== null) {
      const order = await store.orderById(row.orderId)
      if (order !== null && (order.status === 'pending' || order.status === 'paid')) {
        await store.setOrderStatus(row.orderId, 'fulfilled', new Date(), new Date(), note)
      }
    }
    await audit(actor, 'bill.paid', `bill:${billId}`, JSON.stringify({ amountCents: row.amountCents, period: row.period, note }))
    notify('bill.paid', { userId: row.userId, billId, amountCents: row.amountCents })
    return rowToBill(paid)
  }

  const service: AliothBillingService = {
    async getSubscription(userId) {
      const row = await store.getSubscription(userId)
      return row === null ? null : rowToSubscription(row)
    },

    async planOf(userId) {
      return planOf(userId)
    },

    async subscribe(userId) {
      const existing = await store.getSubscription(userId)
      if (existing !== null && existing.status === 'canceling') {
        // 撤销取消：re-activate in place (the period was never interrupted).
        const restored = { ...existing, status: 'active' as const, canceledAt: null }
        await store.upsertSubscription(restored)
        await audit(userId, 'subscription.resumed', `user:${userId}`, '')
        return rowToSubscription(restored)
      }
      if (existing !== null && existing.status === 'active') {
        // Idempotent while the subscription lives (no duplicate bill, no new period).
        return rowToSubscription(existing)
      }
      const now = new Date()
      const row: SubscriptionRow = {
        userId,
        plan: 'L1',
        status: 'active',
        startedAt: existing?.status === 'canceled' ? now : existing?.startedAt ?? now,
        renewsAt: new Date(now.getTime() + PERIOD_MS),
        canceledAt: null,
      }
      await store.upsertSubscription(row)
      await audit(userId, 'subscription.started', `user:${userId}`, JSON.stringify({ plan: 'L1' }))
      notify('subscription.started', { userId })
      await materializePeriod(userId, currentPeriod(now))
      return rowToSubscription(row)
    },

    async cancel(userId) {
      const row = await store.getSubscription(userId)
      if (row === null || row.status === 'canceled') return
      // 期末生效: entitlements stay live until renewsAt; the renewal executor
      // flips the row to 'canceled' when the period ends. This is what the UI
      // copy always said — the implementation now matches it.
      await store.upsertSubscription({ ...row, status: 'canceling', canceledAt: row.canceledAt ?? new Date() })
      await audit(userId, 'subscription.cancel-requested', `user:${userId}`, JSON.stringify({ until: row.renewsAt.toISOString() }))
    },

    async runRenewals(now = new Date()) {
      let renewed = 0
      let ended = 0
      for (const row of await store.subscriptionsDue(now)) {
        if (row.status === 'canceling') {
          await store.upsertSubscription({ ...row, status: 'canceled' })
          await audit(row.userId, 'subscription.ended', `user:${row.userId}`, row.renewsAt.toISOString())
          ended += 1
          continue
        }
        await materializePeriod(row.userId, currentPeriod(row.renewsAt))
        await store.upsertSubscription({ ...row, renewsAt: new Date(row.renewsAt.getTime() + PERIOD_MS) })
        renewed += 1
      }
      return { renewed, ended }
    },

    async bills(userId) {
      return (await store.billsFor(userId)).map(rowToBill)
    },

    async orders(userId) {
      return (await store.ordersFor(userId)).map(rowToOrder)
    },

    async allOrders(actor) {
      if (actor.role !== 'admin') throw new Error('aliothBilling.allOrders: admin only')
      return (await store.allOrders()).map(rowToOrder)
    },

    async payBill(billId, actor) {
      const row = await store.billById(billId)
      if (row === null) throw new Error('aliothBilling.payBill: bill not found')
      if (row.userId !== actor.id && actor.role !== 'admin') {
        throw new Error('aliothBilling.payBill: not your bill')
      }
      return settleBill(billId, actor.id)
    },

    async applyChannelPayment(channel, billReference, amountCents, note) {
      if (channel.trim() === '') throw new Error('aliothBilling.applyChannelPayment: channel required')
      const row = await store.billById(billReference)
      if (row === null) throw new Error('aliothBilling.applyChannelPayment: bill not found')
      if (row.amountCents !== amountCents) {
        // A settled amount that disagrees with the receivable is a reconciliation
        // event, not a payment: refuse and let the operator adjudicate.
        throw new Error(
          `aliothBilling.applyChannelPayment: amount mismatch (bill ${row.amountCents} vs channel ${amountCents})`,
        )
      }
      return settleBill(billReference, `channel:${channel}`, note)
    },

    async refundBill(billId, actor) {
      if (actor.role !== 'admin') throw new Error('aliothBilling.refundBill: admin only')
      const row = await store.billById(billId)
      if (row === null) throw new Error('aliothBilling.refundBill: bill not found')
      if (row.status !== 'paid') throw new Error('aliothBilling.refundBill: 仅已支付账单可退款')
      const refunded = await store.setBillStatus(billId, 'refunded', row.paidAt)
      if (row.orderId !== null) {
        await store.setOrderStatus(row.orderId, 'refunded', row.paidAt, null)
      }
      await audit(actor.id, 'bill.refunded', `bill:${billId}`, JSON.stringify({ amountCents: row.amountCents }))
      notify('bill.refunded', { userId: row.userId, billId, amountCents: row.amountCents })
      return rowToBill(refunded ?? row)
    },

    async voidBill(billId, actor) {
      if (actor.role !== 'admin') throw new Error('aliothBilling.voidBill: admin only')
      const row = await store.billById(billId)
      if (row === null) throw new Error('aliothBilling.voidBill: bill not found')
      if (row.status !== 'unpaid') throw new Error('aliothBilling.voidBill: 仅未支付账单可作废')
      const voided = await store.setBillStatus(billId, 'void', null)
      if (row.orderId !== null) {
        await store.setOrderStatus(row.orderId, 'cancelled', null, null)
      }
      await audit(actor.id, 'bill.void', `bill:${billId}`, JSON.stringify({ amountCents: row.amountCents }))
      return rowToBill(voided ?? row)
    },

    async reconcile(actor) {
      if (actor.role !== 'admin') throw new Error('aliothBilling.reconcile: admin only')
      const [bills, orders] = await Promise.all([store.allBills(), store.allOrders()])
      const count = <T extends string>(rows: readonly { status: T }[]): Record<string, number> => {
        const tally: Record<string, number> = {}
        for (const row of rows) tally[row.status] = (tally[row.status] ?? 0) + 1
        return tally
      }
      const billTally = count(bills)
      const orderTally = count(orders)
      const billIds = new Set(bills.map(bill => bill.id))
      const orderIds = new Set(orders.map(order => order.id))
      const dayMs = 24 * 3600 * 1000
      const anomalies: ReconciliationAnomaly[] = []
      for (const order of orders) {
        if (order.status === 'pending' && Date.now() - order.createdAt.getTime() > dayMs) {
          anomalies.push({ kind: 'order-stuck-pending', detail: `order ${order.id} (user ${order.userId}) pending since ${order.createdAt.toISOString()}` })
        }
        if (order.billId !== null && !billIds.has(order.billId)) {
          anomalies.push({ kind: 'order-without-bill', detail: `order ${order.id} references missing bill ${order.billId}` })
        }
      }
      for (const bill of bills) {
        if (bill.status === 'paid' && (bill.orderId === null || !orderIds.has(bill.orderId))) {
          anomalies.push({ kind: 'bill-paid-without-order', detail: `bill ${bill.id} (user ${bill.userId}) paid without a fulfilled order` })
        }
      }
      return {
        bills: {
          total: bills.length,
          unpaid: billTally.unpaid ?? 0,
          paid: billTally.paid ?? 0,
          void: billTally.void ?? 0,
          refunded: billTally.refunded ?? 0,
        },
        orders: {
          total: orders.length,
          pending: orderTally.pending ?? 0,
          paid: orderTally.paid ?? 0,
          fulfilled: orderTally.fulfilled ?? 0,
          cancelled: orderTally.cancelled ?? 0,
          refunded: orderTally.refunded ?? 0,
        },
        anomalies,
      }
    },

    async invoices(userId) {
      return (await store.invoicesFor(userId)).map(rowToInvoice)
    },

    async requestInvoice(billId, actor, title, taxId) {
      if (title.trim() === '') throw new Error('aliothBilling.requestInvoice: 发票抬头不能为空')
      const bill = await store.billById(billId)
      if (bill === null) throw new Error('aliothBilling.requestInvoice: bill not found')
      if (bill.userId !== actor.id && actor.role !== 'admin') {
        throw new Error('aliothBilling.requestInvoice: not your bill')
      }
      if (bill.status !== 'paid') throw new Error('aliothBilling.requestInvoice: 仅已支付账单可申请发票')
      if (await store.invoiceByBill(billId) !== null) {
        throw new Error('aliothBilling.requestInvoice: 该账单已有发票申请')
      }
      // No super-admin review queue: requesting issues the invoice directly,
      // with a receipt-level number (tax-control integration assigns real ones).
      const id = randomUUID()
      const number = invoiceNumberFor(bill.period, id)
      const row: InvoiceRow = {
        id, billId, userId: bill.userId,
        title: title.trim(), taxId: taxId.trim(),
        status: 'issued', number, requestedAt: new Date(), issuedAt: new Date(),
      }
      await store.insertInvoice(row)
      await audit(actor.id, 'invoice.issued', `invoice:${id}`, JSON.stringify({ billId, number }))
      notify('invoice.issued', { userId: bill.userId, billId, number })
      return rowToInvoice(row)
    },

    async pendingInvoices(_actor): Promise<PendingInvoice[]> {
      // No admin review queue in the no-super-admin product: requests issue
      // directly, so this queue is always empty.
      return []
    },

    async issueInvoice(invoiceId, _actor) {
      // Idempotent self-service issuance (requests already issue directly).
      const invoices = await store.allInvoices()
      const invoice = invoices.find(candidate => candidate.id === invoiceId)
      if (invoice === undefined) throw new Error('aliothBilling.issueInvoice: invoice not found')
      if (invoice.status === 'issued') return rowToInvoice(invoice) // idempotent
      const bill = await store.billById(invoice.billId)
      const issued = await store.setInvoiceIssued(invoiceId, invoiceNumberFor(bill?.period ?? currentPeriod(), invoiceId))
      return rowToInvoice(issued ?? invoice)
    },

    async entitlement(actor, capability) {
      if (capability === 'source-download') {
        const license = await sourceLicense(actor.id)
        const until = license?.until ?? null
        const allowed = until !== null && until.getTime() >= Date.now()
        return {
          capability,
          allowed,
          plan: await planOf(actor.id),
          reason: license === null
            ? 'no L2 authorization (商务对接开通)'
            : allowed ? 'L2 authorization active' : 'L2 authorization expired',
          until,
          monthlyCostCents: null,
        }
      }
      if (capability === 'llm-budget') {
        const plan = await planOf(actor.id)
        const configured = plan === 'L1' ? opts.llmMonthlyCostCentsL1 : opts.llmMonthlyCostCentsL0
        return {
          capability,
          allowed: true,
          plan,
          reason: configured === undefined
            ? 'no monthly cost budget configured for this plan (cost is metered, not capped)'
            : `monthly cost budget ${configured} cents (plan ${plan})`,
          until: null,
          monthlyCostCents: configured ?? null,
        }
      }
      throw new Error(`aliothBilling.entitlement: unknown capability ${String(capability)}`)
    },

    async recordUsage(account, entries) {
      for (const entry of entries) {
        await store.addUsage({
          account,
          day: entry.day,
          model: entry.model,
          tokensIn: entry.tokensIn,
          tokensOut: entry.tokensOut,
          calls: entry.calls,
          costCents: entry.costCents,
        })
      }
    },

    async usageMonthly(account) {
      return store.usageMonth(account, currentPeriod())
    },

    async audit(actor, event, target, evidence) {
      await audit(actor, event, target ?? '', evidence ?? '')
    },

    async auditTail(actor, limit = 50) {
      if (actor.role !== 'admin') throw new Error('aliothBilling.auditTail: admin only')
      return store.auditTail(Math.min(Math.max(limit, 1), 500))
    },

    async sourceLicense(userId) {
      return sourceLicense(userId)
    },

    async grantSourceLicense(userId, until, note) {
      const row = await licenses.grant(userId, until, note)
      await audit('operator', 'license.granted', `user:${userId}`, JSON.stringify({ until: until.toISOString(), note: note ?? '' }))
      notify('license.granted', { userId, until: until.toISOString() })
      return { userId, until: row.until ?? until, grantedBy: 'grant' }
    },

    async requestSourceLicense(userId) {
      const row = await licenses.request(userId)
      await audit(userId, 'license.requested', `user:${userId}`)
      return row
    },

    async sourceLicenseRequest(userId) {
      return licenses.read(userId)
    },
  }
  return service
}

/** Construction options for the volatile provider (tests, DB-less trees). */
export interface MemoryBillingOptions {
  resolveUsername?: ((userId: string) => Promise<string | null>) | undefined
  /** Operator-configured L2 authorizations, keyed by username (a recorded grant wins over this). */
  sourceLicenses?: ReadonlyMap<string, Date> | undefined
  /** Where authorizations are recorded; defaults to the in-memory store. */
  licenses?: LicenseStore | undefined
  llmMonthlyCostCentsL0?: number | undefined
  llmMonthlyCostCentsL1?: number | undefined
  notifyWebhookUrl?: string | undefined
  onError?: ((message: string) => void) | undefined
}

/**
 * In-memory provider: same domain rules on the memory store adapter. Real
 * deployments get the Postgres one through `apply` — that is the point of
 * having two adapters at the store seam.
 */
export function createMemoryBilling(opts: MemoryBillingOptions = {}): AliothBillingService {
  return createBilling({
    store: createMemoryBillingStore(),
    licenses: opts.licenses ?? createMemoryLicenseStore(),
    resolveUsername: opts.resolveUsername,
    sourceLicenses: opts.sourceLicenses,
    llmMonthlyCostCentsL0: opts.llmMonthlyCostCentsL0,
    llmMonthlyCostCentsL1: opts.llmMonthlyCostCentsL1,
    notifyWebhookUrl: opts.notifyWebhookUrl,
    onError: opts.onError,
  })
}

/**
 * Parse the operator's L2 authorization list. Malformed entries throw: a typo in a
 * paid entitlement must not silently downgrade someone to "no license".
 * @param raw - `username:YYYY-MM-DD[,username:YYYY-MM-DD…]`.
 * @returns authorizations keyed by username.
 */
export function parseSourceLicenses(raw: string): ReadonlyMap<string, Date> {
  const licenses = new Map<string, Date>()
  for (const entry of raw.split(',').map(part => part.trim()).filter(part => part !== '')) {
    const separator = entry.lastIndexOf(':')
    const username = separator === -1 ? '' : entry.slice(0, separator).trim()
    const until = separator === -1 ? '' : entry.slice(separator + 1).trim()
    // Date-only, interpreted as end of that day, so a licence given to a date
    // covers that whole day rather than expiring at its midnight start.
    const parsed = /^\d{4}-\d{2}-\d{2}$/.test(until) ? new Date(`${until}T23:59:59.999Z`) : new Date(Number.NaN)
    if (username === '' || Number.isNaN(parsed.getTime())) {
      throw new Error(`aliothBilling: sourceLicenses entry is not "username:YYYY-MM-DD": ${JSON.stringify(entry)}`)
    }
    licenses.set(username, parsed)
  }
  return licenses
}

/** Structural face of the env service (absent in trees without a database). */
interface EnvLike {
  sql: SqlFn
}

function envOf(ctx: Context): EnvLike | undefined {
  try {
    const value = (ctx.get as (name: string) => unknown).call(ctx, 'aliothEnv')
    return typeof value === 'object' && value !== null && typeof (value as EnvLike).sql === 'function'
      ? value as EnvLike
      : undefined
  } catch {
    return undefined
  }
}

/**
 * The billing store for this deployment: durable when a database is mounted,
 * and the in-memory one otherwise.
 *
 * Resolution is LAZY and memoized on first use, not decided at mount: cordis mounts
 * plugins asynchronously, so reading `aliothEnv` while billing itself is being
 * applied sees no provider — and locking that in would silently downgrade a
 * deployment to the volatile store, which is how a request lands in memory and
 * vanishes on restart.
 */
function durableStore(ctx: Context): BillingStore {
  let resolved: BillingStore | undefined
  const store = (): BillingStore => {
    if (resolved !== undefined) return resolved
    const env = envOf(ctx)
    resolved = env === undefined
      ? createMemoryBillingStore()
      : createPgBillingStore((text, values) => env.sql(text, values))
    return resolved
  }
  return new Proxy({} as BillingStore, {
    get(_target, property) {
      const inner = store() as unknown as Record<string, unknown>
      return inner[property as string]
    },
  })
}

/**
 * The license store for this deployment: durable when a database is mounted, and
 * the in-memory one otherwise (same lazy resolution as {@link durableStore}).
 */
function durableLicenses(ctx: Context): LicenseStore {
  let resolved: LicenseStore | undefined
  const store = (): LicenseStore => {
    if (resolved !== undefined) return resolved
    const env = envOf(ctx)
    if (env === undefined) {
      resolved = createMemoryLicenseStore()
      return resolved
    }
    const sql: SqlFn = (text, values) => env.sql(text, values)
    const pg = createPgLicenseStore(sql)
    let schemaReady: Promise<void> | undefined
    const ensureSchema = (): Promise<void> => (schemaReady ??= ensureLicenseSchema(sql))
    resolved = {
      read: async userId => { await ensureSchema(); return pg.read(userId) },
      request: async userId => { await ensureSchema(); return pg.request(userId) },
      grant: async (userId, until, note) => { await ensureSchema(); return pg.grant(userId, until, note) },
      pending: async () => { await ensureSchema(); return pg.pending() },
    }
    return resolved
  }
  return {
    read: userId => store().read(userId),
    request: userId => store().request(userId),
    grant: (userId, until, note) => store().grant(userId, until, note),
    pending: () => store().pending(),
  }
}

export function apply(ctx: Context, config: Config): void {
  const sourceLicenses = parseSourceLicenses(config.sourceLicenses ?? '')
  // The operator's list is keyed by username, so the provider has to resolve an
  // account id back to one. Both halves are optional: a tree without the auth
  // capability simply yields no configured authorization (never a false grant).
  const resolveUsername = async (userId: string): Promise<string | null> => {
    try {
      const auth = (ctx.get as (name: string) => unknown).call(ctx, 'aliothAuth') as {
        userById?: (id: string) => Promise<{ username: string } | null>
      } | undefined
      const user = await auth?.userById?.(userId) ?? null
      return user?.username ?? null
    } catch {
      return null
    }
  }
  // Domain rows are durable wherever a database exists (the plugin mounts after
  // env-alioth in the bundle); a DB-less tree keeps the in-memory store. Tables
  // are created lazily on first use, so mounting never touches the database —
  // and a store that later fails must surface as an error, not as silence.
  const store = durableStore(ctx)
  const licenses = durableLicenses(ctx)
  const service = createBilling({
    store,
    licenses,
    resolveUsername,
    sourceLicenses,
    llmMonthlyCostCentsL0: config.llmMonthlyCostCentsL0,
    llmMonthlyCostCentsL1: config.llmMonthlyCostCentsL1,
    notifyWebhookUrl: config.notifyWebhookUrl === '' ? undefined : config.notifyWebhookUrl,
    onError: message => ctx.logger.warn(`billing-alioth: ${message}`),
  })
  // Renewal executor: the first sweep runs shortly after boot (catching periods
  // that ended while the process was down), then every RENEWAL_INTERVAL_MS.
  const sweep = (): void => {
    void service.runRenewals().catch(error => ctx.logger.warn(
      `billing-alioth: renewal sweep failed: ${error instanceof Error ? error.message : String(error)}`,
    ))
  }
  const boot = setTimeout(sweep, 5_000)
  boot.unref?.()
  const timer = setInterval(sweep, RENEWAL_INTERVAL_MS)
  timer.unref?.()
  ctx.effect(() => () => {
    clearTimeout(boot)
    clearInterval(timer)
  })
  ctx.provide('aliothBilling', service)
}
