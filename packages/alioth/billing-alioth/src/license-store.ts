/**
 * L2 source-download authorizations: where they live.
 *
 * The ladder's source tier is 商务对接 — a person agrees a window, the deployment
 * records it. This module is that record: the user center's 申请 writes a request
 * row, and the operator's one command stamps `granted_at`/`until`. A granted row
 * beats the operator's pre-launch `ALIOTH_SOURCE_LICENSES` list, since it is the
 * negotiated truth.
 *
 * The store owns its own schema (`dsh_alioth_billing`) and never touches the model
 * registry or the auth schema. Account ids are kept as plain text on purpose: a
 * cross-schema foreign key would make this table un-loadable in a deployment that
 * boots billing without auth.
 * @module @dsh-alioth/billing-alioth/license-store
 */

/**
 * Minimal parameterized-query face (`ctx.aliothEnv.sql` / `PgHandle.query` satisfy it
 * structurally). Rows arrive as `unknown` and are parsed defensively — the store is a
 * durability boundary, so it validates what the database hands back.
 */
export interface SqlFn {
  (text: string, values?: readonly unknown[]): Promise<{ rows: readonly unknown[]; rowCount: number | null }>
}

/** Billing-owned schema for authorizations. */
export const LICENSE_SCHEMA = 'dsh_alioth_billing'

/** Idempotent bootstrap: schema + table (never touches the registry). */
export async function ensureLicenseSchema(sql: SqlFn): Promise<void> {
  await sql(`
    CREATE SCHEMA IF NOT EXISTS ${LICENSE_SCHEMA};
    CREATE TABLE IF NOT EXISTS ${LICENSE_SCHEMA}.source_licenses (
      user_id text PRIMARY KEY,
      requested_at timestamptz NOT NULL DEFAULT now(),
      granted_at timestamptz,
      until timestamptz,
      note text NOT NULL DEFAULT ''
    );
  `)
}

/** One authorization row as the surfaces see it. */
export interface LicenseView {
  readonly userId: string
  /** When the account asked (the request row is created on first read of a grant). */
  readonly requestedAt: Date
  /** Set once the operator grants; null while still pending. */
  readonly grantedAt: Date | null
  /** End of the granted window; null while pending. */
  readonly until: Date | null
  readonly note: string
}

/** The store the billing provider reads and the user center writes. */
export interface LicenseStore {
  /** Current row, or null when this account has never asked and was never granted. */
  read(userId: string): Promise<LicenseView | null>
  /** Record (or keep) a request. Idempotent; a granted row is left untouched. */
  request(userId: string): Promise<LicenseView>
  /** Operator path: stamp the window. Creates the row when there is no request yet. */
  grant(userId: string, until: Date, note?: string): Promise<LicenseView>
  /** Requests awaiting the operator, oldest first. */
  pending(): Promise<readonly LicenseView[]>
}

function asDate(value: unknown): Date | null {
  if (value instanceof Date) return value
  if (typeof value !== 'string') return null
  const parsed = new Date(value)
  return Number.isNaN(parsed.getTime()) ? null : parsed
}

/** Parse one row; `null` when it is not a usable authorization record. */
function toView(row: unknown): LicenseView | null {
  if (typeof row !== 'object' || row === null) return null
  const record = row as Record<string, unknown>
  const userId = record.user_id
  const requestedAt = asDate(record.requested_at)
  if (typeof userId !== 'string' || requestedAt === null) return null
  return {
    userId,
    requestedAt,
    grantedAt: asDate(record.granted_at),
    until: asDate(record.until),
    note: typeof record.note === 'string' ? record.note : '',
  }
}

const SELECT = `SELECT user_id, requested_at, granted_at, until, note FROM ${LICENSE_SCHEMA}.source_licenses`

/**
 * Postgres-backed store. Every call re-reads: licenses are granted out of band, so
 * a cache would serve a stale "not licensed" to someone who just paid.
 * @param sql - deployment query function.
 * @returns the store.
 */
export function createPgLicenseStore(sql: SqlFn): LicenseStore {
  return {
    async read(userId) {
      const result = await sql(`${SELECT} WHERE user_id = $1`, [userId])
      return toView(result.rows[0])
    },

    async request(userId) {
      // `ON CONFLICT DO NOTHING` keeps a granted row (and its window) intact when
      // someone asks again after being granted.
      await sql(
        `INSERT INTO ${LICENSE_SCHEMA}.source_licenses (user_id) VALUES ($1) ON CONFLICT (user_id) DO NOTHING`,
        [userId],
      )
      const result = await sql(`${SELECT} WHERE user_id = $1`, [userId])
      const row = toView(result.rows[0])
      if (row === null) throw new Error('billing: source license row vanished after insert')
      return row
    },

    async grant(userId, until, note = '') {
      await sql(
        `INSERT INTO ${LICENSE_SCHEMA}.source_licenses (user_id, granted_at, until, note)
         VALUES ($1, now(), $2, $3)
         ON CONFLICT (user_id) DO UPDATE SET granted_at = now(), until = EXCLUDED.until, note = EXCLUDED.note`,
        [userId, until, note],
      )
      const result = await sql(`${SELECT} WHERE user_id = $1`, [userId])
      const row = toView(result.rows[0])
      if (row === null) throw new Error('billing: source license row vanished after grant')
      return row
    },

    async pending() {
      const result = await sql(`${SELECT} WHERE granted_at IS NULL ORDER BY requested_at`)
      return result.rows.map(toView).filter((row): row is LicenseView => row !== null)
    },
  }
}

/**
 * In-memory store — tests and any deployment that mounts billing without a
 * database. Grants live and die with the process, which is exactly why the real
 * deployments use the Postgres one.
 * @returns the store.
 */
export function createMemoryLicenseStore(): LicenseStore {
  const rows = new Map<string, LicenseView>()

  const read = (userId: string): LicenseView | null => rows.get(userId) ?? null
  return {
    read: async userId => read(userId),
    async request(userId) {
      const existing = read(userId)
      if (existing !== null) return existing
      const created: LicenseView = { userId, requestedAt: new Date(), grantedAt: null, until: null, note: '' }
      rows.set(userId, created)
      return created
    },
    async grant(userId, until, note = '') {
      const existing = read(userId)
      const granted: LicenseView = {
        userId,
        requestedAt: existing?.requestedAt ?? new Date(),
        grantedAt: new Date(),
        until,
        note,
      }
      rows.set(userId, granted)
      return granted
    },
    pending: async () => [...rows.values()].filter(row => row.grantedAt === null),
  }
}
