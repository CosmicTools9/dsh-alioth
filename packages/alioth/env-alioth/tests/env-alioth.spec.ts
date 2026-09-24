import { describe, expect, it, beforeAll, afterAll } from 'vitest'
import { acquirePostgres, type PgHandle, type QueryFn } from '../src/pg.ts'
import { mkdir, mkdtemp, rm, writeFile } from 'node:fs/promises'
import { tmpdir } from 'node:os'
import path from 'node:path'
import { Context } from '@deepseek-ai/cordis'
import { Client } from 'pg'

import {
  extractModelVersion,
  inspectModelArtifacts,
  parseModelSource,
  resolveModelSnapshot,
} from '../src/model-source.ts'
import { bootstrapDatabase, type BootstrapStamp } from '../src/bootstrap.ts'
import { maskUrl } from '../src/doctor.ts'
import { AliothEnv, type AliothEnvInfo, type Config } from '../src/index.ts'
import { createTestDatabase, type TestDatabase } from './test-db.ts'

// ── fixtures ─────────────────────────────────────────────────────────────

const SCHEMA_DDL = `
CREATE TYPE isahl_meta.collection_type AS ENUM ('table', 'view');
CREATE TABLE isahl_meta.meta_collections (
    table_name       text                              NOT NULL,
    collection_type  isahl_meta.collection_type       NOT NULL DEFAULT 'table',
    created_at       timestamptz                       NOT NULL DEFAULT now(),
    PRIMARY KEY (table_name)
);
CREATE TABLE isahl_meta.meta_fields (
    table_name  text  NOT NULL,
    field_name  text  NOT NULL,
    PRIMARY KEY (table_name, field_name)
);
`
const SEED_COLLECTIONS_DDL = `
INSERT INTO isahl_meta.meta_collections (table_name) VALUES ('inventory'), ('demand');
`

const SEED_FIELDS_DDL = `
INSERT INTO isahl_meta.meta_fields (table_name, field_name)
VALUES ('inventory', 'name'), ('inventory', 'qty'), ('demand', 'title');
`

/** A valid-on-real-PG model snapshot fixture. `version` feeds the lib.rs anchor. */
async function makeModelFixture(root: string, version: string): Promise<void> {
  await Promise.all([
    mkdir(path.join(root, 'backend', 'ddl'), { recursive: true }),
    mkdir(path.join(root, 'backend', 'vendor', 'alioth-gen', 'src'), { recursive: true }),
    mkdir(path.join(root, 'skill-adapters'), { recursive: true }),
    mkdir(path.join(root, 'Pre-Proc', 'Alioth', '_schema'), { recursive: true }),
  ])
  await Promise.all([
    // Not isahl_meta: must be excluded from the bootstrap set — its content is
    // deliberately invalid SQL so accidental execution fails the test loudly.
    writeFile(path.join(root, 'backend', 'ddl', '001_app_creator_tables.sql'), 'THIS IS NOT SQL;\n'),
    writeFile(path.join(root, 'backend', 'ddl', '002_isahl_meta_schema.sql'), SCHEMA_DDL),
    writeFile(path.join(root, 'backend', 'ddl', '003_isahl_meta_seed_collections.sql'), SEED_COLLECTIONS_DDL),
    writeFile(path.join(root, 'backend', 'ddl', '004_isahl_meta_seed_fields.sql'), SEED_FIELDS_DDL),
    writeFile(path.join(root, 'skill-adapters', 'alioth-app.yaml'), 'track: app\n'),
    writeFile(path.join(root, 'Pre-Proc', 'Alioth', '_schema', 'app.schema.json'), '{}\n'),
    writeFile(
      path.join(root, 'backend', 'vendor', 'alioth-gen', 'src', 'lib.rs'),
      `pub static ALIOTH_MODEL_VERSION: LazyLock<String> =\n`
      + `    LazyLock::new(|| env::var("MODEL_VERSION").unwrap_or_else(|_| "${version}".to_string()));\n`,
    ),
  ])
}

/** Rewrite only the version anchor of an existing fixture. */
async function setFixtureVersion(root: string, version: string): Promise<void> {
  await writeFile(
    path.join(root, 'backend', 'vendor', 'alioth-gen', 'src', 'lib.rs'),
    `pub static ALIOTH_MODEL_VERSION: LazyLock<String> =\n`
    + `    LazyLock::new(|| env::var("MODEL_VERSION").unwrap_or_else(|_| "${version}".to_string()));\n`,
  )
}

// ── unit: model source ───────────────────────────────────────────────────

describe('env-alioth parseModelSource', () => {
  it('parses github with ref', () => {
    expect(parseModelSource('github:CosmicTools9/Alioth'))
      .toEqual({ kind: 'github', repo: 'CosmicTools9/Alioth', ref: 'main' })
  })

  it('defaults github ref to main', () => {
    expect(parseModelSource('github:a/b')).toEqual({ kind: 'github', repo: 'a/b', ref: 'main' })
  })

  it('rejects malformed github repos', () => {
    expect(() => parseModelSource('github:not-a-repo')).toThrow('invalid github model source')
    expect(() => parseModelSource('github:a/b@')).toThrow('empty ref')
  })

  it('treats non-github specs as local paths', () => {
    expect(parseModelSource('/abs/path')).toEqual({ kind: 'local', path: '/abs/path' })
    expect(() => parseModelSource('')).toThrow('empty model source')
  })
})

describe('env-alioth model artifacts', () => {
  let root: string

  beforeAll(async () => {
    root = await mkdtemp(path.join(tmpdir(), 'dsh-alioth-model-'))
    await makeModelFixture(root, '10.0.0')
  })

  afterAll(async () => {
    await rm(root, { recursive: true, force: true })
  })

  it('selects only isahl_meta DDL in filename order, plus adapters and schemas', async () => {
    const artifacts = await inspectModelArtifacts(root)
    expect(artifacts.ddlFiles.map(file => path.basename(file))).toEqual([
      '002_isahl_meta_schema.sql',
      '003_isahl_meta_seed_collections.sql',
      '004_isahl_meta_seed_fields.sql',
    ])
    expect(artifacts.skillAdapterFiles).toHaveLength(1)
    expect(artifacts.artifactSchemaFiles).toHaveLength(1)
  })

  it('extracts the model version from vendored lib.rs', async () => {
    await expect(extractModelVersion(root)).resolves.toBe('10.0.0')
  })

  it('falls back to unknown for a missing version anchor', async () => {
    const empty = await mkdtemp(path.join(tmpdir(), 'dsh-alioth-empty-'))
    try {
      await expect(extractModelVersion(empty)).resolves.toBe('unknown')
    } finally {
      await rm(empty, { recursive: true, force: true })
    }
  })

  it('rejects directories without an isahl_meta DDL baseline', async () => {
    const empty = await mkdtemp(path.join(tmpdir(), 'dsh-alioth-notmodel-'))
    try {
      await expect(inspectModelArtifacts(empty)).rejects.toThrow('not an Alioth model snapshot')
    } finally {
      await rm(empty, { recursive: true, force: true })
    }
  })

  it('resolves local snapshots in place with local provenance', async () => {
    const snapshot = await resolveModelSnapshot({ kind: 'local', path: root }, root)
    expect(snapshot.dir).toBe(path.resolve(root))
    expect(snapshot.sourceRef).toBe('local')
    expect(snapshot.modelVersion).toBe('10.0.0')
  })
})

// ── unit: bootstrap semantics against a recording fake client ────────────

interface FakeState {
  isahlSchema: boolean
  stamp: BootstrapStamp | null
  stampTable: boolean
  executedDdl: string[]
  /** Whether the existing registry looks like this plugin's (has `meta_collections`). Defaults true. */
  registryTable?: boolean
  /** Regular tables the catalog counts in `isahl_meta` — a foreign registry's occupancy. Defaults 0. */
  schemaTables?: number
}

/**
 * Answers the exact queries `bootstrapDatabase` issues, recording DDL runs.
 * A bare query function, like the handle it stands in for — the module under test
 * takes a `QueryFn`, never a client object.
 */
function fakeQuery(state: FakeState): QueryFn {
  const run = async (
    sql: string,
    values?: readonly unknown[],
  ): Promise<{ rows: Record<string, unknown>[]; rowCount: number | null }> => {
    if (sql.includes('CREATE SCHEMA IF NOT EXISTS isahl_meta')) {
      // The repair + baseline run: one round trip, one implicit transaction.
      state.executedDdl.push(sql)
      return { rows: [], rowCount: 0 }
    }
    if (sql.includes('FROM pg_namespace')) {
      return { rows: [{ exists: state.isahlSchema }], rowCount: 1 }
    }
    if (sql.includes('count(*)')) {
      return { rows: [{ n: String(state.schemaTables ?? 0) }], rowCount: 1 }
    }
    if (sql.includes('FROM pg_class')) {
      // A registry table cannot exist without its schema — the probe the code makes first.
      const present = state.isahlSchema && state.registryTable !== false
      return { rows: present ? [{ '?column?': 1 }] : [], rowCount: present ? 1 : 0 }
    }
    if (sql.includes('current_database()')) {
      return { rows: [{ name: 'fake_db' }], rowCount: 1 }
    }
    if (sql.includes('CREATE TABLE IF NOT EXISTS dsh_alioth.model_state')) {
      state.stampTable = true
      return { rows: [], rowCount: 0 }
    }
    if (sql.includes('to_regclass')) {
      return { rows: [{ oid: state.stampTable ? 101 : null }], rowCount: 1 }
    }
    if (sql.includes('FROM dsh_alioth.model_state')) {
      if (state.stamp === null) {
        return { rows: [], rowCount: 0 }
      }
      return {
        rows: [{
          model_version: state.stamp.modelVersion,
          source_ref: state.stamp.sourceRef,
          bootstrapped_at: state.stamp.bootstrappedAt,
        }],
        rowCount: 1,
      }
    }
    if (sql.includes('INSERT INTO dsh_alioth.model_state')) {
      state.stamp = {
        modelVersion: String(values?.[0]),
        sourceRef: String(values?.[1]),
        bootstrappedAt: new Date(),
      }
      return { rows: [], rowCount: 1 }
    }
    state.executedDdl.push(sql)
    return { rows: [], rowCount: 0 }
  }
  return run as QueryFn
}

describe('env-alioth bootstrapDatabase', () => {
  let ddlFiles: readonly string[]
  let root: string

  beforeAll(async () => {
    root = await mkdtemp(path.join(tmpdir(), 'dsh-alioth-boot-'))
    await makeModelFixture(root, '10.0.0')
    ddlFiles = (await inspectModelArtifacts(root)).ddlFiles
  })

  afterAll(async () => {
    await rm(root, { recursive: true, force: true })
  })

  it('creates the registry from DDL then stamps, in order', async () => {
    const state: FakeState = { isahlSchema: false, stamp: null, stampTable: false, executedDdl: [] }
    const result = await bootstrapDatabase(fakeQuery(state), ddlFiles, { modelVersion: '10.0.0', sourceRef: 'sha-1' })
    expect(result).toEqual({ created: true, stamped: true })
    // Schema creation precedes the baseline, which ran in filename order — all in one round trip.
    expect(state.executedDdl).toHaveLength(1)
    const repair = state.executedDdl[0] ?? ''
    expect(repair.indexOf('CREATE SCHEMA IF NOT EXISTS isahl_meta')).toBeLessThan(repair.indexOf(SCHEMA_DDL))
    expect(repair.indexOf(SCHEMA_DDL)).toBeLessThan(repair.indexOf(SEED_COLLECTIONS_DDL))
    expect(repair.indexOf(SEED_COLLECTIONS_DDL)).toBeLessThan(repair.indexOf(SEED_FIELDS_DDL))
    expect(state.stamp?.sourceRef).toBe('sha-1')
  })

  it('skips DDL over an existing registry and stays quiet when stamped identically', async () => {
    const state: FakeState = {
      isahlSchema: true,
      stamp: { modelVersion: '10.0.0', sourceRef: 'sha-1', bootstrappedAt: new Date() },
      stampTable: true,
      executedDdl: [],
      registryTable: true,
    }
    const result = await bootstrapDatabase(fakeQuery(state), ddlFiles, { modelVersion: '10.0.0', sourceRef: 'sha-1' })
    expect(result).toEqual({ created: false, stamped: false })
    expect(state.executedDdl).toEqual([])
  })

  it('adopts a foreign registry by stamping it without running DDL', async () => {
    const state: FakeState = { isahlSchema: true, stamp: null, stampTable: false, executedDdl: [], registryTable: true }
    const result = await bootstrapDatabase(fakeQuery(state), ddlFiles, { modelVersion: '10.0.0', sourceRef: 'sha-1' })
    expect(result).toEqual({ created: false, stamped: true })
    expect(state.executedDdl).toEqual([])
  })

  it('refuses an isahl_meta holding someone else\'s tables', async () => {
    // The DDL baseline is load-once, so an existing `isahl_meta` is adopted. If it holds
    // another product's tables, adopting it fails much later as a missing relation — and
    // re-running the baseline over those tables is worse. Refuse up front instead.
    const state: FakeState = { isahlSchema: true, stamp: null, stampTable: false, executedDdl: [], registryTable: false, schemaTables: 3 }
    const err = await bootstrapDatabase(fakeQuery(state), ddlFiles, { modelVersion: '10.0.0', sourceRef: 'sha-1' })
      .then(() => null, error => error)
    expect(err).toBeInstanceOf(Error)
    expect(err?.message).toContain('fake_db')
    expect(err?.message).toContain('meta_collections')
    expect(state.executedDdl).toEqual([])
  })

  it('bootstraps the baseline into an existing isahl_meta that holds no tables of its own', async () => {
    // A model-sample `isahl_meta` (its `devv_*` views only) and a half-created schema both
    // leave this shape behind: the schema exists, the registry does not. Creating the
    // baseline into it restores service without dropping anything the sample owns.
    const state: FakeState = { isahlSchema: true, stamp: null, stampTable: false, executedDdl: [], registryTable: false, schemaTables: 0 }
    const result = await bootstrapDatabase(fakeQuery(state), ddlFiles, { modelVersion: '10.0.0', sourceRef: 'sha-1' })
    expect(result).toEqual({ created: true, stamped: true })
    // The baseline's own non-idempotent objects are cleared first, in the same round trip.
    expect(state.executedDdl).toEqual([
      'CREATE SCHEMA IF NOT EXISTS isahl_meta;\n'
      + 'DROP VIEW IF EXISTS isahl_meta.devv_inherits_union;\n'
      + 'DROP VIEW IF EXISTS isahl_meta.devv_inherits_view;\n'
      + 'DROP TYPE IF EXISTS isahl_meta.collection_type;\n'
      + 'DROP TYPE IF EXISTS isahl_meta.field_category;\n'
      + 'DROP TYPE IF EXISTS isahl_meta.field_data_type;\n'
      + `${SCHEMA_DDL}\n${SEED_COLLECTIONS_DDL}\n${SEED_FIELDS_DDL}`,
    ])
  })

  it('reports drift instead of migrating a mismatched stamp', async () => {
    const state: FakeState = {
      isahlSchema: true,
      stamp: { modelVersion: '10.0.0', sourceRef: 'sha-old', bootstrappedAt: new Date() },
      stampTable: true,
      executedDdl: [],
      registryTable: true,
    }
    const result = await bootstrapDatabase(fakeQuery(state), ddlFiles, { modelVersion: '10.1.0', sourceRef: 'sha-new' })
    expect(result.created).toBe(false)
    expect(result.stamped).toBe(false)
    expect(result.drift).toEqual({
      stamped: { modelVersion: '10.0.0', sourceRef: 'sha-old', bootstrappedAt: state.stamp?.bootstrappedAt },
      current: { modelVersion: '10.1.0', sourceRef: 'sha-new' },
    })
    expect(state.executedDdl).toEqual([])
  })
})

describe('env-alioth doctor maskUrl', () => {
  it('masks credentials but keeps structure', () => {
    expect(maskUrl('postgres://alioth:secret@127.0.0.1:5432/alioth'))
      .toBe('postgres://alioth:***@127.0.0.1:5432/alioth')
    expect(maskUrl('postgresql://u:p%40@h/db')).toBe('postgresql://u:***@h/db')
  })
})

// ── integration: the environment's PostgreSQL, full plugin lifecycle ─────

describe('env-alioth end-to-end (environment PostgreSQL)', () => {
  let modelDir: string
  let dataRoot: string
  let db: TestDatabase

  beforeAll(async () => {
    db = await createTestDatabase('e2e')
    modelDir = await mkdtemp(path.join(tmpdir(), 'dsh-alioth-e2e-model-'))
    dataRoot = await mkdtemp(path.join(tmpdir(), 'dsh-alioth-e2e-data-'))
    await makeModelFixture(modelDir, '10.0.0')
    // A present semantic index keeps the doctor green in these env-focused tests.
    await mkdir(path.join(dataRoot, 'semantic'), { recursive: true })
    await writeFile(path.join(dataRoot, 'semantic', 'meta.json'),
      JSON.stringify({ model: 'fake', entriesHash: 'x', count: 1, dimension: 8 }))
  })

  afterAll(async () => {
    await db.dispose()
    await rm(modelDir, { recursive: true, force: true })
    await rm(dataRoot, { recursive: true, force: true })
  })

  async function boot(): Promise<{ ctx: Context; dispose: () => Promise<void>; info: AliothEnvInfo }> {
    const ctx = new Context()
    const config: Config = { modelSource: modelDir, dataRoot, databaseUrl: db.url }
    const fiber = await ctx.plugin(AliothEnv, config)
    const info = await ctx.aliothEnv.ready()
    return { ctx, dispose: () => fiber.dispose(), info }
  }

  it('bootstraps the registry into a fresh database, seeds land, doctor green', { timeout: 120_000 }, async () => {
    const { ctx, dispose, info } = await boot()
    try {
      expect(info.sourceRef).toBe('local')
      expect(info.modelVersion).toBe('10.0.0')
      expect(info.bootstrap).toEqual({ created: true, stamped: true })
      expect(info.databaseUrl).toBe(db.url)
      const report = await ctx.aliothEnv.doctor()
      expect(report.status).toBe('green')
      expect(report.checks.map(check => check.name)).toEqual(['model-snapshot', 'database', 'isahl-meta', 'model-stamp', 'semantic-index', 'dictionary-snapshots'])
      // Seeds landed and the registry answers queries from a second connection.
      const probe = new Client({ connectionString: info.databaseUrl })
      await probe.connect()
      try {
        const collections = await probe.query<{ table_name: string }>(
          'SELECT table_name FROM isahl_meta.meta_collections ORDER BY table_name',
        )
        expect(collections.rows.map(row => row.table_name)).toEqual(['demand', 'inventory'])
        const fields = await probe.query<{ count: string }>('SELECT count(*)::text AS count FROM isahl_meta.meta_fields')
        expect(fields.rows[0]?.count).toBe('3')
      } finally {
        await probe.end()
      }
    } finally {
      await dispose()
    }
  })

  it('reuses the persisted cluster without re-running DDL', { timeout: 120_000 }, async () => {
    const { ctx, dispose, info } = await boot()
    try {
      expect(info.bootstrap).toEqual({ created: false, stamped: false })
      await expect(ctx.aliothEnv.doctor()).resolves.toMatchObject({ status: 'green' })
    } finally {
      await dispose()
    }
  })

  it('reports drift (doctor red) when the snapshot version moves', { timeout: 120_000 }, async () => {
    await setFixtureVersion(modelDir, '10.1.0')
    const { ctx, dispose, info } = await boot()
    try {
      expect(info.bootstrap.drift).toEqual({
        stamped: { modelVersion: '10.0.0', sourceRef: 'local', bootstrappedAt: expect.any(Date) },
        current: { modelVersion: '10.1.0', sourceRef: 'local' },
      })
      const report = await ctx.aliothEnv.doctor()
      expect(report.status).toBe('red')
      const stamp = report.checks.find(check => check.name === 'model-stamp')
      expect(stamp?.ok).toBe(false)
      expect(stamp?.detail).toContain('model drift')
    } finally {
      await setFixtureVersion(modelDir, '10.0.0')
      await dispose()
    }
  })

  it('resetRegistry drops and re-bootstraps from the current snapshot', { timeout: 120_000 }, async () => {
    const { ctx, dispose } = await boot()
    try {
      await ctx.aliothEnv.resetRegistry()
      // The reset invalidates the memo; the next ready() re-runs the baseline.
      const info = await ctx.aliothEnv.ready()
      expect(info.bootstrap).toEqual({ created: true, stamped: true })
      await expect(ctx.aliothEnv.doctor()).resolves.toMatchObject({ status: 'green' })
    } finally {
      await dispose()
    }
  })
})

// ── network-gated: real github pull ──────────────────────────────────────

const networkTests = process.env.DSH_ALIOTH_NETWORK_TESTS === '1'

describe.skipIf(!networkTests)('env-alioth github snapshot', () => {
  it('pulls a github distribution and resolves artifacts (historical AppCreator channel; new model channel is CosmicTools9/Alioth, validated via builtin/local)', { timeout: 300_000 }, async () => {
    const cacheRoot = await mkdtemp(path.join(tmpdir(), 'dsh-alioth-gh-'))
    try {
      const snapshot = await resolveModelSnapshot(
        { kind: 'github', repo: 'CosmicTools9/AppCreator', ref: 'main' },
        cacheRoot,
      )
      expect(snapshot.sourceRef).toMatch(/^[0-9a-f]{40}$/)
      expect(snapshot.modelVersion).toMatch(/^\d+\.\d+\.\d+$/)
      expect(snapshot.artifacts.skillAdapterFiles.length).toBeGreaterThan(4)
      // Resolving the pinned SHA hits the cache and reuses the same directory.
      const again = await resolveModelSnapshot(
        { kind: 'github', repo: 'CosmicTools9/AppCreator', ref: snapshot.sourceRef },
        cacheRoot,
      )
      expect(again.dir).toBe(snapshot.dir)
    } finally {
      await rm(cacheRoot, { recursive: true, force: true })
    }
  })
})

describe('env-alioth doctor observability', () => {
  it('reports semantic-index as not built and dictionary snapshots', async () => {
    const modelDir = await mkdtemp(path.join(tmpdir(), 'dsh-alioth-obs-model-'))
    const dataRoot = await mkdtemp(path.join(tmpdir(), 'dsh-alioth-obs-data-'))
    await makeModelFixture(modelDir, '10.0.0')
    const db = await createTestDatabase('obs')
    const ctx = new Context()
    const fiber = await ctx.plugin(AliothEnv, { modelSource: modelDir, dataRoot, databaseUrl: db.url })
    try {
      await ctx.aliothEnv.ready()
      const report = await ctx.aliothEnv.doctor()
      const semantic = report.checks.find(check => check.name === 'semantic-index')
      expect(semantic?.ok).toBe(false)
      expect(semantic?.detail).toContain('not built')
      const dicts = report.checks.find(check => check.name === 'dictionary-snapshots')
      expect(dicts?.ok).toBe(true)
      expect(dicts?.detail).toContain('FROZEN')
    } finally {
      await fiber.dispose()
      await db.dispose()
      await rm(modelDir, { recursive: true, force: true })
      await rm(dataRoot, { recursive: true, force: true })
    }
  }, 120_000)

  it('reports a built semantic index', async () => {
    const modelDir = await mkdtemp(path.join(tmpdir(), 'dsh-alioth-obs2-model-'))
    const dataRoot = await mkdtemp(path.join(tmpdir(), 'dsh-alioth-obs2-data-'))
    await makeModelFixture(modelDir, '10.0.0')
    await mkdir(path.join(dataRoot, 'semantic'), { recursive: true })
    await writeFile(path.join(dataRoot, 'semantic', 'meta.json'),
      JSON.stringify({ model: 'fake', entriesHash: 'x', count: 12, dimension: 8 }))
    const db = await createTestDatabase('obs2')
    const ctx = new Context()
    const fiber = await ctx.plugin(AliothEnv, { modelSource: modelDir, dataRoot, databaseUrl: db.url })
    try {
      await ctx.aliothEnv.ready()
      const report = await ctx.aliothEnv.doctor()
      const semantic = report.checks.find(check => check.name === 'semantic-index')
      expect(semantic?.ok).toBe(true)
      expect(semantic?.detail).toContain('12 entries')
    } finally {
      await fiber.dispose()
      await db.dispose()
      await rm(modelDir, { recursive: true, force: true })
      await rm(dataRoot, { recursive: true, force: true })
    }
  }, 120_000)
})

describe('env-alioth PgHandle resilience', () => {
  let db: TestDatabase
  let handle: PgHandle

  beforeAll(async () => {
    db = await createTestDatabase('pghandle')
    handle = await acquirePostgres({ url: db.url })
  }, 120_000)

  afterAll(async () => {
    await handle?.close().catch(() => {})
    await db.dispose()
  })

  it('recovers when the connection dies while the session is idle', async () => {
    // The product case: the AppAgent pipeline runs for minutes between registry
    // queries, so a socket can die of old age with nobody looking. A long-lived
    // session used to hold one bare `Client`, and after that every later query
    // failed with pg's "not queryable" guard until the process restarted.
    const victim = await handle.query<{ pid: number }>('SELECT pg_backend_pid() AS pid')
    const killer = await acquirePostgres({ url: handle.url })
    try {
      // Killed from a *second* connection, so the victim learns about it while idle
      // (an in-flight kill is the ambiguous case pinned by the next test).
      await killer.query('SELECT pg_terminate_backend($1)', [victim.rows[0]?.pid]).catch(() => {})
      await new Promise(resolve => setTimeout(resolve, 250))
      const recovered = await handle.query<{ pid: number }>('SELECT pg_backend_pid() AS pid')
      expect(recovered.rows[0]?.pid).not.toBe(victim.rows[0]?.pid)
      expect((await handle.query<{ ok: number }>('SELECT 1 AS ok')).rows[0]?.ok).toBe(1)
    } finally {
      await killer.close()
    }
  })

  it('never replays a statement that may already have executed', async () => {
    // The reconnect covers only failures where pg refused to *send* the statement.
    // A statement that dies in flight may have executed, and replaying it would
    // double-apply — so it must propagate and the connection must be replaced.
    //
    // The sequence makes a replay observable: nextval is not transactional, so its
    // advance survives the rolled-back batch. Two advances = one replay too many.
    // (Note a "the previous query succeeded, so this one is safe" gate would fail
    // here: that gate is open, yet this batch is exactly the unsafe case.)
    await handle.query('DROP SCHEMA IF EXISTS dsh_alioth_probe CASCADE')
    await handle.query('CREATE SCHEMA dsh_alioth_probe')
    await handle.query('CREATE SEQUENCE dsh_alioth_probe.attempts')
    try {
      await handle
        .query(`CREATE TABLE dsh_alioth_probe.writes AS SELECT nextval('dsh_alioth_probe.attempts') AS attempt
                WHERE false; SELECT pg_terminate_backend(pg_backend_pid())`)
        .catch(() => {})
      const rows = await handle.query<{ last_value: string }>(
        'SELECT last_value::text FROM dsh_alioth_probe.attempts',
      )
      expect(rows.rows[0]?.last_value).toBe('1')
      expect((await handle.query<{ ok: number }>('SELECT 1 AS ok')).rows[0]?.ok).toBe(1)
    } finally {
      await handle.query('DROP SCHEMA IF EXISTS dsh_alioth_probe CASCADE').catch(() => {})
    }
  })
})

describe('env-alioth isahl_meta occupancy', () => {
  let ddlFiles: readonly string[]
  let root: string

  beforeAll(async () => {
    root = await mkdtemp(path.join(tmpdir(), 'dsh-alioth-occupancy-'))
    await makeModelFixture(root, '10.0.0')
    ddlFiles = (await inspectModelArtifacts(root)).ddlFiles
  })

  afterAll(async () => {
    await rm(root, { recursive: true, force: true })
  })

  it('creates the registry inside an existing schema that holds no tables of its own', async () => {
    // m2/dev shape: the deployment database carries an `isahl_meta` from the model sample
    // (views only), the registry tables gone. Service must come back without dropping the
    // sample's objects — the baseline is created into the existing schema.
    const db = await createTestDatabase('emptyschema')
    const handle = await acquirePostgres({ url: db.url })
    try {
      await handle.query('CREATE SCHEMA isahl_meta')
      await handle.query('CREATE SCHEMA probe_sample')
      await handle.query('CREATE TABLE probe_sample.thing (id integer)')
      await handle.query('CREATE VIEW isahl_meta.devv_probe AS SELECT id FROM probe_sample.thing')
      const result = await bootstrapDatabase(handle.query, ddlFiles, { modelVersion: '10.0.0', sourceRef: 'local' })
      expect(result).toEqual({ created: true, stamped: true })
      const tables = await handle.query<{ n: string }>(
        `SELECT count(*)::text AS n FROM pg_class c JOIN pg_namespace n ON n.oid = c.relnamespace
          WHERE n.nspname = 'isahl_meta' AND c.relkind IN ('r', 'p')`,
      )
      expect(Number(tables.rows[0]?.n)).toBeGreaterThan(0)
      // The sample's view survived (nothing was dropped to make room).
      await expect(handle.query('SELECT 1 FROM isahl_meta.devv_probe')).resolves.toBeDefined()
    } finally {
      await handle.close()
      await db.dispose()
    }
  })

  it('repairs a registry whose tables were dropped, keeping foreign objects', async () => {
    // The m2/dev shape: the schema survives with the baseline's non-idempotent objects
    // (`CREATE TYPE` has no IF NOT EXISTS) but the registry tables are gone — plus objects
    // from another lineage living in the same schema. The repair clears what the baseline
    // is about to recreate and leaves everything else alone.
    const db = await createTestDatabase('partialregistry')
    const handle = await acquirePostgres({ url: db.url })
    try {
      await handle.query('CREATE SCHEMA isahl_meta')
      await handle.query("CREATE TYPE isahl_meta.collection_type AS ENUM ('stale')")
      await handle.query('CREATE VIEW isahl_meta.devv_inherits_view AS SELECT 1 AS one')
      await handle.query("CREATE TYPE isahl_meta.sample_leftover_type AS ENUM ('sample')")

      const result = await bootstrapDatabase(handle.query, ddlFiles, { modelVersion: '10.0.0', sourceRef: 'local' })
      expect(result).toEqual({ created: true, stamped: true })

      // The registry is the baseline's: table present, enum recreated with the baseline's labels.
      const labels = await handle.query<{ labels: string }>(
        'SELECT enum_range(NULL::isahl_meta.collection_type)::text AS labels',
      )
      expect(labels.rows[0]?.labels).toBe('{table,view}')
      expect((await handle.query('SELECT count(*)::int AS n FROM isahl_meta.meta_collections')).rows[0]?.n).toBe(2)
      // The stale view is gone (the baseline's version, if any, is what the file defines).
      expect(await handle.query("SELECT to_regclass('isahl_meta.devv_inherits_view')::text AS r")).toMatchObject({
        rows: [{ r: null }],
      })
      // Another lineage's object in the same schema is untouched (`to_regclass` only knows
      // relations, so the type is probed through `pg_type`).
      expect((await handle.query(
        `SELECT count(*)::int AS n FROM pg_type t JOIN pg_namespace n ON n.oid = t.typnamespace
          WHERE n.nspname = 'isahl_meta' AND t.typname = 'sample_leftover_type'`,
      )).rows[0]?.n).toBe(1)
    } finally {
      await handle.close()
      await db.dispose()
    }
  })

  it('rolls back the whole repair when one of the baseline\'s objects is depended on', async () => {
    // Drops are RESTRICT: a dependent object must abort the repair instead of being destroyed
    // with it, and the round trip is one transaction — so the database is left exactly as it was.
    const db = await createTestDatabase('dependentleftover')
    const handle = await acquirePostgres({ url: db.url })
    try {
      await handle.query('CREATE SCHEMA isahl_meta')
      await handle.query("CREATE TYPE isahl_meta.collection_type AS ENUM ('stale')")
      await handle.query('CREATE SCHEMA elsewhere')
      await handle.query('CREATE TABLE elsewhere.uses_it (kind isahl_meta.collection_type)')

      const err = await bootstrapDatabase(handle.query, ddlFiles, { modelVersion: '10.0.0', sourceRef: 'local' })
        .then(() => null, error => error)
      expect(err).toBeInstanceOf(Error)
      expect(String(err?.message)).toContain('collection_type')

      // Nothing moved: the enum, its dependent table, and the absent registry all stand.
      expect((await handle.query("SELECT to_regclass('elsewhere.uses_it') IS NOT NULL AS kept")).rows[0]?.kept).toBe(true)
      expect((await handle.query("SELECT enum_range(NULL::isahl_meta.collection_type)::text AS labels")).rows[0]?.labels)
        .toBe('{stale}')
      expect(await handle.query("SELECT to_regclass('isahl_meta.meta_collections')::text AS r")).toMatchObject({
        rows: [{ r: null }],
      })
    } finally {
      await handle.close()
      await db.dispose()
    }
  })

  it('fails loud when isahl_meta holds another product\'s tables', async () => {
    // The baseline is load-once by contract: it is never re-applied over existing tables,
    // and an adopted foreign registry would surface much later as a missing relation.
    const db = await createTestDatabase('foreignregistry')
    const handle = await acquirePostgres({ url: db.url })
    try {
      await handle.query('CREATE SCHEMA isahl_meta')
      await handle.query('CREATE TABLE isahl_meta.their_registry (id integer)')
      const err = await bootstrapDatabase(handle.query, [], { modelVersion: '10.0.0', sourceRef: 'local' })
        .then(() => null, error => error)
      expect(err).toBeInstanceOf(Error)
      expect(err?.message).toContain('meta_collections')
      expect(err?.message).toContain('1 table(s)')
      expect(err?.message).toContain('ALIOTH_DATABASE_URL')
    } finally {
      await handle.close()
      await db.dispose()
    }
  })
})

describe('env-alioth database configuration', () => {
  it('fails loud with the settable places when no URL is configured', async () => {
    // The plugin never provisions a cluster: an unset DSN is a deployment error the
    // operator must see at boot, not a silently auto-started embedded server that
    // competes with the environment's PostgreSQL.
    const started = Date.now()
    const err = await acquirePostgres({ url: '   ' }).then(
      () => null,
      error => error,
    )
    expect(err).toBeInstanceOf(Error)
    expect(err?.message).toContain('no PostgreSQL URL configured')
    expect(err?.message).toContain('ALIOTH_DATABASE_URL')
    expect(err?.message).toContain('~/.dsh-alioth.env')
    expect(err?.message).toContain('Config.databaseUrl')
    expect(Date.now() - started).toBeLessThan(1000)
  })
})
