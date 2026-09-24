/**
 * L2 authorization persistence: the user center's request, the operator's grant,
 * and what the download gate actually reads.
 *
 * The PostgreSQL cases run against a throwaway database (the same helper the other
 * DB-backed suites use), because the interesting behavior lives in the SQL: an
 * idempotent request must not clear a granted window, and a grant must win over the
 * operator's pre-launch env list.
 */
import { describe, expect, it, beforeAll, afterAll } from 'vitest'
import * as billing from '../src/index.ts'
import {
  createMemoryLicenseStore,
  createPgLicenseStore,
  ensureLicenseSchema,
  LICENSE_SCHEMA,
} from '../src/license-store.ts'
import { acquirePostgres, type PgHandle } from '@dsh-alioth/env-alioth'
import { createTestDatabase, type TestDatabase } from '../../env-alioth/tests/test-db.ts'

describe('memory license store', () => {
  it('records a request, keeps it idempotent, and stamps a grant', async () => {
    const store = createMemoryLicenseStore()
    expect(await store.read('id-ada')).toBeNull()

    const requested = await store.request('id-ada')
    expect(requested).toMatchObject({ userId: 'id-ada', grantedAt: null, until: null })
    // Asking twice must not create a second row nor move the request timestamp.
    expect((await store.request('id-ada')).requestedAt).toEqual(requested.requestedAt)

    const until = new Date('2027-12-31T23:59:59.999Z')
    const granted = await store.grant('id-ada', until, '合同 2026-114')
    expect(granted).toMatchObject({ grantedAt: expect.any(Date), until, note: '合同 2026-114' })
    // A request after a grant leaves the window alone.
    expect((await store.request('id-ada')).until).toEqual(until)
    expect(await store.pending()).toEqual([])
  })

  it('lists pending requests oldest first', async () => {
    const store = createMemoryLicenseStore()
    await store.request('id-first')
    await store.request('id-second')
    expect((await store.pending()).map(row => row.userId)).toEqual(['id-first', 'id-second'])
  })
})

describe('postgres license store', () => {
  let testDb: TestDatabase
  let handle: PgHandle
  let store: ReturnType<typeof createPgLicenseStore>

  beforeAll(async () => {
    testDb = await createTestDatabase('billinglicense')
    // A raw connection: the store needs a query function, not the model machinery.
    handle = await acquirePostgres({ url: testDb.url })
    await ensureLicenseSchema(handle.query)
    store = createPgLicenseStore(handle.query)
  }, 120_000)

  afterAll(async () => {
    await handle.close().catch(() => {})
    await testDb.dispose()
  })

  it('creates its own schema and never touches the registry', async () => {
    const schemas = await handle.query<{ nspname: string }>(
      'SELECT nspname FROM pg_namespace WHERE nspname = $1',
      [LICENSE_SCHEMA],
    )
    expect(schemas.rows.map(row => row.nspname)).toEqual([LICENSE_SCHEMA])
  })

  it('records requests, survives a repeat, and grants without losing the request', async () => {
    await handle.query(`DELETE FROM ${LICENSE_SCHEMA}.source_licenses`)
    expect(await store.read('id-ada')).toBeNull()

    const requested = await store.request('id-ada')
    expect(requested.grantedAt).toBeNull()
    const again = await store.request('id-ada')
    expect(again.requestedAt).toEqual(requested.requestedAt)

    const until = new Date('2027-12-31T23:59:59.999Z')
    const granted = await store.grant('id-ada', until, '合同 2026-114')
    expect(granted.until?.toISOString()).toBe(until.toISOString())
    expect(granted.grantedAt).not.toBeNull()

    // The granted window survives another request.
    expect((await store.request('id-ada')).until?.toISOString()).toBe(until.toISOString())
    // A grant without a prior request is allowed (商务对接 can grant proactively).
    const direct = await store.grant('id-grace', until)
    expect(direct.grantedAt).not.toBeNull()
    expect((await store.pending()).map(row => row.userId)).toEqual([])
  })

  it('reads back through a fresh store instance (durability, not a cache)', async () => {
    const fresh = createPgLicenseStore(handle.query)
    expect((await fresh.read('id-ada'))?.until?.toISOString()).toBe('2027-12-31T23:59:59.999Z')
  })
})

describe('the provider reads the store, then the operator list', () => {
  it('prefers a recorded grant over the configured list', async () => {
    const store = createMemoryLicenseStore()
    const svc = billing.createMemoryBilling({
      sourceLicenses: billing.parseSourceLicenses('ada:2026-12-31'),
      resolveUsername: async userId => (userId === 'id-ada' ? 'ada' : null),
      licenses: store,
    })

    // Operator list only: the pre-launch switch still works.
    const fromList = await svc.sourceLicense('id-ada')
    expect(fromList).toMatchObject({ grantedBy: 'operator-config' })
    expect(fromList?.until.toISOString()).toBe('2026-12-31T23:59:59.999Z')

    // A recorded grant (the negotiated truth) outranks it.
    await store.grant('id-ada', new Date('2028-06-30T23:59:59.999Z'), '续签')
    const recorded = await svc.sourceLicense('id-ada')
    expect(recorded?.grantedBy).toBe('grant')
    expect(recorded?.until.toISOString()).toBe('2028-06-30T23:59:59.999Z')

    // And an unlicensed account still gets nothing.
    expect(await svc.sourceLicense('id-eve')).toBeNull()
  })
})
