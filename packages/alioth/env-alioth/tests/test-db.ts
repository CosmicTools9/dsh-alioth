/**
 * Per-suite throwaway database on the deployment's PostgreSQL 18.
 *
 * The suite used to inherit a private embedded cluster per `dataRoot`; now that
 * env-alioth talks to the environment's server only, isolation comes from a
 * database the suite creates and drops. It is NEVER the deployment's own
 * database: the registry bootstrap writes `isahl_meta` / `dsh_alioth`, so a
 * shared database would let one suite's reset wipe another's registry (and, on
 * dev, would touch the model-sample database).
 *
 * Admin DSN: `DSH_ALIOTH_TEST_ADMIN_URL`, else the local server's superuser
 * (`postgres://postgres@127.0.0.1:5432/postgres`, trust auth on a dev box).
 * The role needs CREATEDB; CI points this at its `postgres:18` service.
 * @module @dsh-alioth/env-alioth/tests/test-db
 */

import { randomBytes } from 'node:crypto'
import { Client } from 'pg'

const DEFAULT_ADMIN_URL = 'postgres://postgres@127.0.0.1:5432/postgres'

const adminUrl = process.env.DSH_ALIOTH_TEST_ADMIN_URL ?? DEFAULT_ADMIN_URL

export interface TestDatabase {
  /** DSN handed to env-alioth / `acquirePostgres`. */
  readonly url: string
  /** Drop the database (safe to call twice). */
  dispose(): Promise<void>
}

async function withAdmin<T>(fn: (client: Client) => Promise<T>): Promise<T> {
  const client = new Client({ connectionString: adminUrl })
  await client.connect()
  try {
    return await fn(client)
  } finally {
    await client.end()
  }
}

/**
 * Create `<label>`'s database and return its DSN.
 * @param label - short suite name; sanitised into the database name.
 */
export async function createTestDatabase(label: string): Promise<TestDatabase> {
  const suffix = label.replace(/[^A-Za-z0-9]+/g, '_').toLowerCase().slice(0, 24)
  const name = `dsh_alioth_test_${suffix}_${randomBytes(3).toString('hex')}`
  await withAdmin(async client => { await client.query(`CREATE DATABASE "${name}"`) })
  const url = new URL(adminUrl)
  url.pathname = `/${name}`
  return {
    url: url.toString(),
    dispose: async () => {
      await withAdmin(async client => {
        await client.query(`DROP DATABASE IF EXISTS "${name}" WITH (FORCE)`)
      })
    },
  }
}
