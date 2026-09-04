import { describe, expect, it, beforeAll, afterAll } from 'vitest'
import { acquirePostgres } from '../src/pg.ts'
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
}

/** Answers the exact queries `bootstrapDatabase` issues, recording DDL runs. */
class FakeClient {
  constructor(private readonly state: FakeState) {}

  async query(sql: string, values?: readonly unknown[]): Promise<{ rows: Record<string, unknown>[]; rowCount: number | null }> {
    if (sql.includes('information_schema.schemata')) {
      return { rows: [{ exists: this.state.isahlSchema }], rowCount: 1 }
    }
    if (sql.includes('CREATE TABLE IF NOT EXISTS dsh_alioth.model_state')) {
      this.state.stampTable = true
      return { rows: [], rowCount: 0 }
    }
    if (sql.includes('to_regclass')) {
      return { rows: [{ oid: this.state.stampTable ? 101 : null }], rowCount: 1 }
    }
    if (sql.includes('FROM dsh_alioth.model_state')) {
      if (this.state.stamp === null) {
        return { rows: [], rowCount: 0 }
      }
      return {
        rows: [{
          model_version: this.state.stamp.modelVersion,
          source_ref: this.state.stamp.sourceRef,
          bootstrapped_at: this.state.stamp.bootstrappedAt,
        }],
        rowCount: 1,
      }
    }
    if (sql.includes('INSERT INTO dsh_alioth.model_state')) {
      this.state.stamp = {
        modelVersion: String(values?.[0]),
        sourceRef: String(values?.[1]),
        bootstrappedAt: new Date(),
      }
      return { rows: [], rowCount: 1 }
    }
    this.state.executedDdl.push(sql)
    return { rows: [], rowCount: 0 }
  }
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
    const result = await bootstrapDatabase(new FakeClient(state) as never, ddlFiles, { modelVersion: '10.0.0', sourceRef: 'sha-1' })
    expect(result).toEqual({ created: true, stamped: true })
    // Schema creation precedes the baseline, which ran in filename order.
    expect(state.executedDdl).toEqual(['CREATE SCHEMA IF NOT EXISTS isahl_meta', SCHEMA_DDL, SEED_COLLECTIONS_DDL, SEED_FIELDS_DDL])
    expect(state.stamp?.sourceRef).toBe('sha-1')
  })

  it('skips DDL over an existing registry and stays quiet when stamped identically', async () => {
    const state: FakeState = {
      isahlSchema: true,
      stamp: { modelVersion: '10.0.0', sourceRef: 'sha-1', bootstrappedAt: new Date() },
      stampTable: true,
      executedDdl: [],
    }
    const result = await bootstrapDatabase(new FakeClient(state) as never, ddlFiles, { modelVersion: '10.0.0', sourceRef: 'sha-1' })
    expect(result).toEqual({ created: false, stamped: false })
    expect(state.executedDdl).toEqual([])
  })

  it('adopts a foreign registry by stamping it without running DDL', async () => {
    const state: FakeState = { isahlSchema: true, stamp: null, stampTable: false, executedDdl: [] }
    const result = await bootstrapDatabase(new FakeClient(state) as never, ddlFiles, { modelVersion: '10.0.0', sourceRef: 'sha-1' })
    expect(result).toEqual({ created: false, stamped: true })
    expect(state.executedDdl).toEqual([])
  })

  it('reports drift instead of migrating a mismatched stamp', async () => {
    const state: FakeState = {
      isahlSchema: true,
      stamp: { modelVersion: '10.0.0', sourceRef: 'sha-old', bootstrappedAt: new Date() },
      stampTable: true,
      executedDdl: [],
    }
    const result = await bootstrapDatabase(new FakeClient(state) as never, ddlFiles, { modelVersion: '10.1.0', sourceRef: 'sha-new' })
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

// ── integration: real embedded PostgreSQL, full plugin lifecycle ─────────

describe('env-alioth embedded end-to-end', () => {
  let modelDir: string
  let dataRoot: string

  beforeAll(async () => {
    modelDir = await mkdtemp(path.join(tmpdir(), 'dsh-alioth-e2e-model-'))
    dataRoot = await mkdtemp(path.join(tmpdir(), 'dsh-alioth-e2e-data-'))
    await makeModelFixture(modelDir, '10.0.0')
    // A present semantic index keeps the doctor green in these env-focused tests.
    await mkdir(path.join(dataRoot, 'semantic'), { recursive: true })
    await writeFile(path.join(dataRoot, 'semantic', 'meta.json'),
      JSON.stringify({ model: 'fake', entriesHash: 'x', count: 1, dimension: 8 }))
  })

  afterAll(async () => {
    await rm(modelDir, { recursive: true, force: true })
    await rm(dataRoot, { recursive: true, force: true })
  })

  async function boot(): Promise<{ ctx: Context; dispose: () => Promise<void>; info: AliothEnvInfo }> {
    const ctx = new Context()
    const config: Config = { modelSource: modelDir, dataRoot }
    const fiber = await ctx.plugin(AliothEnv, config)
    const info = await ctx.aliothEnv.ready()
    return { ctx, dispose: () => fiber.dispose(), info }
  }

  it('bootstraps a fresh embedded cluster, seeds land, doctor green', { timeout: 120_000 }, async () => {
    const { ctx, dispose, info } = await boot()
    try {
      expect(info.sourceRef).toBe('local')
      expect(info.modelVersion).toBe('10.0.0')
      expect(info.bootstrap).toEqual({ created: true, stamped: true })
      expect(info.databaseUrl).toMatch(/^postgres:\/\/alioth:[^@]+@127\.0\.0\.1:\d+\/alioth$/)
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
    const ctx = new Context()
    const fiber = await ctx.plugin(AliothEnv, { modelSource: modelDir, dataRoot })
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
    const ctx = new Context()
    const fiber = await ctx.plugin(AliothEnv, { modelSource: modelDir, dataRoot })
    try {
      await ctx.aliothEnv.ready()
      const report = await ctx.aliothEnv.doctor()
      const semantic = report.checks.find(check => check.name === 'semantic-index')
      expect(semantic?.ok).toBe(true)
      expect(semantic?.detail).toContain('12 entries')
    } finally {
      await fiber.dispose()
      await rm(modelDir, { recursive: true, force: true })
      await rm(dataRoot, { recursive: true, force: true })
    }
  }, 120_000)
})



describe('env-alioth embedded cluster lock', () => {
  it('fails loud (no hang) when the data dir is held by a live postmaster', async () => {
    const root = await mkdtemp(path.join(tmpdir(), 'env-pg-lock-'))
    try {
      // Synthetic postmaster.pid naming a LIVE process (this test runner):
      // the guard must fail fast with the actionable error (regression: a
      // held data dir used to hang forever in stop()).
      await mkdir(path.join(root, 'postgres'), { recursive: true })
      await writeFile(path.join(root, 'postgres', 'postmaster.pid'), `${process.pid}\n`, 'utf8')
      const started = Date.now()
      const err = await acquirePostgres({ dataRoot: root }).then(
        () => null,
        error => error,
      )
      expect(err).toBeInstanceOf(Error)
      expect(err?.message).toContain(`already running (postmaster pid ${process.pid})`)
      expect(err?.message).toContain('ALIOTH_DATA_ROOT')
      expect(Date.now() - started).toBeLessThan(5000)
    } finally {
      await rm(root, { recursive: true, force: true })
    }
  })

  it('ignores a stale postmaster.pid (dead pid)', async () => {
    const root = await mkdtemp(path.join(tmpdir(), 'env-pg-lock-'))
    try {
      await mkdir(path.join(root, 'postgres'), { recursive: true })
      // A dead pid: spawn-and-reap leaves no live process behind.
      const dead = 4194304
      await writeFile(path.join(root, 'postgres', 'postmaster.pid'), `${dead}\n`, 'utf8')
      // The guard must NOT fail on a stale lock — postgres clears it on start.
      // acquireEmbedded proceeds past the guard (it fails later on missing PG
      // binaries config in the unit environment — assert the guard passed by
      // never seeing the lock error).
      const err = await acquirePostgres({ dataRoot: root }).then(
        () => null,
        error => error,
      )
      if (err !== null && err.message.includes('already running (postmaster pid')) {
        throw new Error('stale lock was treated as live')
      }
    } finally {
      await rm(root, { recursive: true, force: true })
    }
  })
})
