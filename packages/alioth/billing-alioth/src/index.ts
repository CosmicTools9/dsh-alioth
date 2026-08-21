/**
 * `@dsh-alioth/billing-alioth` — the billing CAPABILITY CONTRACT for the B/S
 * deployment: `ctx.aliothBilling` (subscription lifecycle, monthly bills,
 * invoice requests/issuance).
 *
 * NO DB modeling here by decision (2026-08-21): the real billing backend
 * lands later. The integration seam IS this service — a future backend
 * plugin provides a persistent implementation (same interface, swap the
 * provider); until then this package ships a VOLATILE in-memory
 * implementation so the user center is fully usable end-to-end (state resets
 * on restart — acceptable for the pre-backend phase, labeled everywhere).
 *
 * Payment boundary: no external payment channel is wired; bills transition
 * unpaid→paid through the carrier's explicit 线下确认 action. Pricing follows
 * the confirmed BP ladder (L1 = ¥1,399/月).
 * @module @dsh-alioth/billing-alioth
 */

import { randomUUID } from 'node:crypto'
import { Context } from '@deepseek-ai/cordis'
import z from '@deepseek-ai/schemastery'

export const name = 'billing-alioth'
export const inject: readonly string[] = []

/** L1 subscription price in CNY cents (confirmed pricing ladder). */
export const L1_AMOUNT_CENTS = 139900

export interface Config {}

export const Config: z<Config> = z.object({})

export interface BillingUser {
  readonly id: string
  readonly role: 'admin' | 'user'
}

export interface Subscription {
  userId: string
  plan: 'L1'
  status: 'active' | 'canceled'
  startedAt: Date
  renewsAt: Date
}

export interface Bill {
  id: string
  userId: string
  /** Billing period, 'YYYY-MM'. */
  period: string
  /** CNY cents (L1 = 139900). */
  amountCents: number
  status: 'unpaid' | 'paid'
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
  requestedAt: Date
  issuedAt: Date | null
}

/** Admin queue row: invoice + requestor + amount context. */
export type PendingInvoice = Invoice & { username?: string; amountCents: number; period: string }

export interface AliothBillingService {
  /** The user's subscription, null when on the free L0 tier. */
  getSubscription(userId: string): Promise<Subscription | null>
  /** Activate (or re-activate) L1 and materialize the current period's bill. */
  subscribe(userId: string): Promise<Subscription>
  /** Cancel at period end — keeps the subscription row and past bills. */
  cancel(userId: string): Promise<void>
  bills(userId: string): Promise<Bill[]>
  /** Mark a bill paid (offline confirmation; the future PSP lands here). Own bill or admin. */
  payBill(billId: string, actor: BillingUser): Promise<Bill>
  invoices(userId: string): Promise<Invoice[]>
  /** Request an invoice (发票抬头 + 纳税人识别号) for a PAID bill — one per bill. */
  requestInvoice(billId: string, actor: BillingUser, title: string, taxId: string): Promise<Invoice>
  /** Admin queue: all pending invoices (requestor usernames when resolvable). */
  pendingInvoices(actor: BillingUser): Promise<PendingInvoice[]>
  /** Admin action: mark a pending invoice issued. */
  issueInvoice(invoiceId: string, actor: BillingUser): Promise<Invoice>
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

/**
 * VOLATILE in-memory implementation — the pre-backend stopgap. State lives
 * in Maps inside this closure and resets on process restart. The backend
 * integration replaces exactly this provider (same interface).
 */
export function createMemoryBilling(opts: { resolveUsername?: (userId: string) => Promise<string | null> } = {}): AliothBillingService {
  const subscriptions = new Map<string, Subscription>()
  const bills = new Map<string, Bill>()
  const invoices = new Map<string, Invoice>()

  const myBills = (userId: string): Bill[] =>
    [...bills.values()].filter(b => b.userId === userId).sort((a, b) => b.period.localeCompare(a.period))
  const myInvoices = (userId: string): Invoice[] =>
    [...invoices.values()].filter(i => i.userId === userId).sort((a, b) => b.requestedAt.getTime() - a.requestedAt.getTime())

  const service: AliothBillingService = {
    async getSubscription(userId) {
      return subscriptions.get(userId) ?? null
    },

    async subscribe(userId) {
      const existing = subscriptions.get(userId)
      const sub: Subscription = {
        userId,
        plan: 'L1',
        status: 'active',
        startedAt: existing?.startedAt ?? new Date(),
        renewsAt: new Date(Date.now() + 30 * 24 * 3600 * 1000),
      }
      subscriptions.set(userId, sub)
      const period = currentPeriod()
      if (!myBills(userId).some(b => b.period === period)) {
        const bill: Bill = {
          id: randomUUID(), userId, period, amountCents: L1_AMOUNT_CENTS,
          status: 'unpaid', createdAt: new Date(), paidAt: null,
        }
        bills.set(bill.id, bill)
      }
      return sub
    },

    async cancel(userId) {
      const sub = subscriptions.get(userId)
      if (sub !== undefined) subscriptions.set(userId, { ...sub, status: 'canceled' })
    },

    async bills(userId) {
      return myBills(userId)
    },

    async payBill(billId, actor) {
      const bill = bills.get(billId)
      if (bill === undefined) throw new Error('aliothBilling.payBill: bill not found')
      if (bill.userId !== actor.id && actor.role !== 'admin') throw new Error('aliothBilling.payBill: not your bill')
      if (bill.status === 'paid') return bill // idempotent
      const paid: Bill = { ...bill, status: 'paid', paidAt: new Date() }
      bills.set(billId, paid)
      return paid
    },

    async invoices(userId) {
      return myInvoices(userId)
    },

    async requestInvoice(billId, actor, title, taxId) {
      if (title.trim() === '') throw new Error('aliothBilling.requestInvoice: 发票抬头不能为空')
      const bill = bills.get(billId)
      if (bill === undefined) throw new Error('aliothBilling.requestInvoice: bill not found')
      if (bill.userId !== actor.id && actor.role !== 'admin') throw new Error('aliothBilling.requestInvoice: not your bill')
      if (bill.status !== 'paid') throw new Error('aliothBilling.requestInvoice: 仅已支付账单可申请发票')
      if ([...invoices.values()].some(i => i.billId === billId)) {
        throw new Error('aliothBilling.requestInvoice: 该账单已有发票申请')
      }
      const invoice: Invoice = {
        id: randomUUID(), billId, userId: bill.userId,
        title: title.trim(), taxId: taxId.trim(),
        status: 'pending', requestedAt: new Date(), issuedAt: null,
      }
      invoices.set(invoice.id, invoice)
      return invoice
    },

    async pendingInvoices(actor) {
      if (actor.role !== 'admin') throw new Error('aliothBilling.pendingInvoices: admin only')
      const rows: PendingInvoice[] = []
      for (const invoice of invoices.values()) {
        if (invoice.status !== 'pending') continue
        const bill = bills.get(invoice.billId)
        if (bill === undefined) continue
        const username = opts.resolveUsername === undefined ? undefined : await opts.resolveUsername(invoice.userId)
        rows.push({
          ...invoice, amountCents: bill.amountCents, period: bill.period,
          ...(username === undefined || username === null ? {} : { username }),
        })
      }
      return rows.sort((a, b) => a.requestedAt.getTime() - b.requestedAt.getTime())
    },

    async issueInvoice(invoiceId, actor) {
      if (actor.role !== 'admin') throw new Error('aliothBilling.issueInvoice: admin only')
      const invoice = invoices.get(invoiceId)
      if (invoice === undefined) throw new Error('aliothBilling.issueInvoice: invoice not found')
      if (invoice.status === 'issued') return invoice // idempotent
      const issued: Invoice = { ...invoice, status: 'issued', issuedAt: new Date() }
      invoices.set(invoiceId, issued)
      return issued
    },
  }
  return service
}


export function apply(ctx: Context, _config: Config): void {
  void _config
  // The pre-backend volatile provider. A backend integration replaces this
  // provide call with a persistent implementation of the same interface.
  ctx.provide('aliothBilling', createMemoryBilling())
}
