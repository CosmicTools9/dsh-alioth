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
import {
  createMemoryLicenseStore,
  createPgLicenseStore,
  ensureLicenseSchema,
  type LicenseStore,
  type LicenseView,
  type SqlFn,
} from './license-store.ts'
import z from '@deepseek-ai/schemastery'

export const name = 'billing-alioth'
export const inject: readonly string[] = []

/** L1 subscription price in CNY cents (confirmed pricing ladder). */
export const L1_AMOUNT_CENTS = 139900

export {
  createMemoryLicenseStore,
  createPgLicenseStore,
  ensureLicenseSchema,
  LICENSE_SCHEMA,
  type LicenseStore,
  type LicenseView,
} from './license-store.ts'

export interface Config {
  /**
   * Operator-configured L2 source-download authorizations, `username:YYYY-MM-DD`
   * separated by commas (env `ALIOTH_SOURCE_LICENSES`, injected by the bundle).
   * L2 is 商务对接 — it has no self-serve path, so the deployment writes it down
   * until a back-office owns it. A malformed entry fails loud at mount.
   */
  readonly sourceLicenses?: string
}

export const Config: z<Config> = z.object({
  sourceLicenses: z.string().default(''),
})

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
/** Construction options for the volatile provider. */
export interface MemoryBillingOptions {
  resolveUsername?: (userId: string) => Promise<string | null>
  /** Operator-configured L2 authorizations, keyed by username (a recorded grant wins over this). */
  sourceLicenses?: ReadonlyMap<string, Date>
  /** Where authorizations are recorded; defaults to the in-memory store. */
  licenses?: LicenseStore
}

export function createMemoryBilling(opts: MemoryBillingOptions = {}): AliothBillingService {
  const subscriptions = new Map<string, Subscription>()
  const bills = new Map<string, Bill>()
  const invoices = new Map<string, Invoice>()
  /** Recorded authorizations (user-center requests + operator grants). */
  const licenses: LicenseStore = opts.licenses ?? createMemoryLicenseStore()

  const myBills = (userId: string): Bill[] =>
    [...bills.values()].filter(b => b.userId === userId).sort((a, b) => b.period.localeCompare(a.period))
  const myInvoices = (userId: string): Invoice[] =>
    [...invoices.values()].filter(i => i.userId === userId).sort((a, b) => b.requestedAt.getTime() - a.requestedAt.getTime())

  const service: AliothBillingService = {
    async getSubscription(userId) {
      return subscriptions.get(userId) ?? null
    },

    async sourceLicense(userId) {
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
    },

    async grantSourceLicense(userId, until, note) {
      const row = await licenses.grant(userId, until, note)
      return { userId, until: row.until ?? until, grantedBy: 'grant' }
    },

    async requestSourceLicense(userId) {
      return licenses.request(userId)
    },

    async sourceLicenseRequest(userId) {
      return licenses.read(userId)
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
      if (bill.userId !== actor.id) throw new Error('aliothBilling.payBill: not your bill')
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
      if (bill.userId !== actor.id) throw new Error('aliothBilling.requestInvoice: not your bill')
      if (bill.status !== 'paid') throw new Error('aliothBilling.requestInvoice: 仅已支付账单可申请发票')
      if ([...invoices.values()].some(i => i.billId === billId)) {
        throw new Error('aliothBilling.requestInvoice: 该账单已有发票申请')
      }
      // No super-admin review queue: requesting issues the invoice directly.
      const invoice: Invoice = {
        id: randomUUID(), billId, userId: bill.userId,
        title: title.trim(), taxId: taxId.trim(),
        status: 'issued', requestedAt: new Date(), issuedAt: new Date(),
      }
      invoices.set(invoice.id, invoice)
      return invoice
    },

    async pendingInvoices(_actor): Promise<PendingInvoice[]> {
      // No admin review queue in the no-super-admin product: requests issue
      // directly, so this queue is always empty.
      return []
    },

    async issueInvoice(invoiceId, _actor) {
      // Idempotent self-service issuance (requests already issue directly).
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
 * The license store for this deployment: durable when a database is mounted, and
 * the in-memory one otherwise.
 *
 * Resolution is LAZY and memoized on first use, not decided at mount: cordis mounts
 * plugins asynchronously, so reading `aliothEnv` while billing itself is being
 * applied sees no provider — and locking that in would silently downgrade a
 * deployment to the volatile store, which is how a request lands in memory and
 * vanishes on restart.
 *
 * `env.sql` is a method too: called detached it would lose `this` (its ready()/
 * handle state), so it is wrapped once. The schema is created on first use.
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
  // Authorizations are durable wherever a database exists (the plugin mounts after
  // env-alioth in the bundle); a DB-less tree keeps the in-memory store. The table
  // is created lazily on first use, so mounting never touches the database — and a
  // store that later fails must surface as an error, not as a silent "no license".
  const licenses = durableLicenses(ctx)
  // The pre-backend volatile provider for subscriptions/bills; a backend
  // integration replaces this provide call with a persistent implementation of the
  // same interface, keeping the license store underneath.
  ctx.provide('aliothBilling', createMemoryBilling({ sourceLicenses, resolveUsername, licenses }))
}
