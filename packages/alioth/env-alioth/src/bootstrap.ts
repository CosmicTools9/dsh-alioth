/**
 * `isahl_meta` bootstrap. The model's DDL baseline (`backend/ddl/*isahl_meta*.sql`)
 * is NOT idempotent — its contract (see the files' headers) is "load only when
 * the `isahl_meta` schema does not exist", and it assumes the loader created
 * the schema first (its `CREATE TYPE` statements target `isahl_meta.*` without
 * any `CREATE SCHEMA`). This module is that loader, plus a `dsh_alioth.model_state`
 * stamp recording which snapshot the registry was bootstrapped from. Upgrades
 * are never applied destructively: a stamp mismatch is reported as drift, not
 * auto-migrated.
 * @module @dsh-alioth/env-alioth/bootstrap
 */

import { readFile } from 'node:fs/promises'
import type { QueryResult, QueryResultRow } from 'pg'
import type { QueryFn } from './pg.ts'

/** The registry schema bootstrapped from the model DDL baseline. */
const REGISTRY_SCHEMA = 'isahl_meta'
/** This plugin's private state schema — never touches `isahl_meta`. */
const STAMP_SCHEMA = 'dsh_alioth'

const STAMP_DDL = `
CREATE SCHEMA IF NOT EXISTS ${STAMP_SCHEMA};
CREATE TABLE IF NOT EXISTS ${STAMP_SCHEMA}.model_state (
    id              integer      PRIMARY KEY DEFAULT 1 CHECK (id = 1),
    model_version   text         NOT NULL,
    source_ref      text         NOT NULL,
    bootstrapped_at timestamptz  NOT NULL DEFAULT now()
);
`

/** What the database says it was bootstrapped from. */
export interface BootstrapStamp {
  readonly modelVersion: string
  readonly sourceRef: string
  readonly bootstrappedAt: Date
}

/** What the current snapshot says. */
export interface ModelProvenance {
  readonly modelVersion: string
  readonly sourceRef: string
}

export interface BootstrapResult {
  /** True when this call created `isahl_meta` by executing the DDL baseline. */
  readonly created: boolean
  /** True when this call wrote the `dsh_alioth` stamp (first adopt). */
  readonly stamped: boolean
  /** Present when an existing stamp does not match the current snapshot. */
  readonly drift?: { readonly stamped: BootstrapStamp; readonly current: ModelProvenance }
}

/**
 * Existence probes read the CATALOG, not `information_schema`: the latter is filtered by the
 * current role's privileges, so an under-privileged deployment role can see a registry's views
 * but not its tables (observed on m2) and would re-run the load-once baseline over a live
 * registry.
 */
async function tableExists(query: QueryFn, schema: string, table: string): Promise<boolean> {
  const result = await query(
    `SELECT 1 FROM pg_class c JOIN pg_namespace n ON n.oid = c.relnamespace
      WHERE n.nspname = $1 AND c.relname = $2 AND c.relkind IN ('r', 'p') LIMIT 1`,
    [schema, table],
  )
  return (result.rowCount ?? 0) > 0
}

/**
 * Objects the baseline itself creates that PostgreSQL cannot create idempotently — `CREATE TYPE`
 * has no `IF NOT EXISTS`, and the two views are plain `CREATE VIEW`. When the registry is absent
 * but the schema survives holding these leftovers (a registry whose tables were dropped, or a
 * partially applied baseline), they must be removed before the baseline runs again.
 *
 * Deliberately RESTRICT, never CASCADE: a dependent object aborts the whole repair and the
 * transaction rolls back, instead of being destroyed along with what it depends on. Keep in sync
 * with `vendor/backend/ddl/002_isahl_meta_schema.sql`; a newly added baseline type/view surfaces
 * as a loud `duplicate_object` rather than passing silently.
 */
const BASELINE_OWNED_REMNANTS = [
  'DROP VIEW IF EXISTS isahl_meta.devv_inherits_union;',
  'DROP VIEW IF EXISTS isahl_meta.devv_inherits_view;',
  'DROP TYPE IF EXISTS isahl_meta.collection_type;',
  'DROP TYPE IF EXISTS isahl_meta.field_category;',
  'DROP TYPE IF EXISTS isahl_meta.field_data_type;',
]

/** Regular tables occupying a schema (views/sequences do not count). 0 when the schema is absent. */
async function countTables(query: QueryFn, schema: string): Promise<number> {
  const result = await query<{ n: string }>(
    `SELECT count(*)::text AS n FROM pg_class c JOIN pg_namespace n ON n.oid = c.relnamespace
      WHERE n.nspname = $1 AND c.relkind IN ('r', 'p')`,
    [schema],
  )
  return Number.parseInt(result.rows[0]?.n ?? '0', 10)
}

/** Read the stamp row; `null` when the table or row is absent. */
export async function readStamp(query: QueryFn): Promise<BootstrapStamp | null> {
  const table = await query<{ oid: number | null }>(
    'SELECT to_regclass($1) AS oid',
    [`${STAMP_SCHEMA}.model_state`],
  )
  if (table.rows[0]?.oid == null) {
    return null
  }
  const rows = await query<QueryResultRow & { model_version: string; source_ref: string; bootstrapped_at: Date }>(
    `SELECT model_version, source_ref, bootstrapped_at FROM ${STAMP_SCHEMA}.model_state WHERE id = 1`,
  )
  const row = rows.rows[0]
  if (row === undefined) {
    return null
  }
  return { modelVersion: row.model_version, sourceRef: row.source_ref, bootstrappedAt: new Date(row.bootstrapped_at) }
}

/**
 * Bring the database to a bootstrapped state for the given snapshot:
 * 1. Registry tables absent (schema missing, or an `isahl_meta` holding no tables of its own —
 *    e.g. a model-sample one carrying only its `devv_*` views) → create the schema if needed,
 *    then execute the DDL baseline files in filename order (schema first — the baseline assumes
 *    it exists). An `isahl_meta` holding SOMEONE ELSE'S tables is refused instead.
 * 2. Ensure the `dsh_alioth` stamp exists, writing it on first adoption (including adoption of a
 *    registry bootstrapped by something else).
 * 3. Never re-run DDL over an existing registry; report provenance drift.
 */
export async function bootstrapDatabase(
  query: QueryFn,
  ddlFiles: readonly string[],
  current: ModelProvenance,
): Promise<BootstrapResult> {
  let created = false
  if (!await tableExists(query, REGISTRY_SCHEMA, 'meta_collections')) {
    // The baseline is load-once BY CONTRACT, so it is never re-applied over a registry — but an
    // `isahl_meta` WITHOUT registry tables is not a registry. Two cases:
    //  * regular tables that are not ours — a foreign registry this plugin must not silently
    //    adopt: adopting it surfaces much later as a mystery missing-relation error in a tool.
    //  * no tables of its own — an absent registry. The schema may still exist holding objects
    //    the baseline itself creates (a deployment whose registry tables were dropped, or a
    //    partially applied baseline): recreate them, then run the baseline.
    const occupied = await countTables(query, REGISTRY_SCHEMA)
    if (occupied > 0) {
      const database = await query<{ name: string }>('SELECT current_database() AS name')
      throw new Error(
        `env-alioth: database "${database.rows[0]?.name ?? '?'}" already has an \`${REGISTRY_SCHEMA}\` schema `
        + `holding ${occupied} table(s) but no \`meta_collections\` — it is not this plugin's registry, and the `
        + 'load-once baseline is never re-applied over someone else\'s tables. Point ALIOTH_DATABASE_URL at a '
        + 'database whose `isahl_meta` is empty or this plugin\'s, or drop the foreign schema first.',
      )
    }
    const baseline = (await Promise.all(ddlFiles.map(file => readFile(file, 'utf8')))).join('\n')
    // ONE round trip = one implicit transaction (simple-query protocol): consumers never observe
    // the schema without its views, and any conflict rolls back to the state before the repair.
    await query([
      `CREATE SCHEMA IF NOT EXISTS ${REGISTRY_SCHEMA};`,
      ...BASELINE_OWNED_REMNANTS,
      baseline,
    ].join('\n'))
    created = true
  }
  await query(STAMP_DDL)
  const stamped = await readStamp(query)
  if (stamped === null) {
    const insert: QueryResult = await query(
      `INSERT INTO ${STAMP_SCHEMA}.model_state (id, model_version, source_ref)
       VALUES (1, $1, $2)
       ON CONFLICT (id) DO NOTHING`,
      [current.modelVersion, current.sourceRef],
    )
    return { created, stamped: insert.rowCount === 1 }
  }
  if (stamped.sourceRef !== current.sourceRef || stamped.modelVersion !== current.modelVersion) {
    return { created, stamped: false, drift: { stamped, current } }
  }
  return { created, stamped: false }
}
