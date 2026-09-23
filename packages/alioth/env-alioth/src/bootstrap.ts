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

async function tableExists(query: QueryFn, schema: string, table: string): Promise<boolean> {
  const result = await query(
    'SELECT 1 FROM information_schema.tables WHERE table_schema = $1 AND table_name = $2 LIMIT 1',
    [schema, table],
  )
  return (result.rowCount ?? 0) > 0
}

async function schemaExists(query: QueryFn, schema: string): Promise<boolean> {
  const res = await query<{ exists: boolean }>(
    'SELECT exists(SELECT 1 FROM information_schema.schemata WHERE schema_name = $1) AS exists',
    [schema],
  )
  return res.rows[0]?.exists === true
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
 * 1. `isahl_meta` absent → create the schema, then execute the DDL baseline
 *    files in filename order (schema first — the baseline assumes it exists).
 * 2. Ensure the `dsh_alioth` stamp exists, writing it on first adoption
 *    (including adoption of a registry bootstrapped by something else).
 * 3. Never re-run DDL over an existing registry; report provenance drift.
 */
export async function bootstrapDatabase(
  query: QueryFn,
  ddlFiles: readonly string[],
  current: ModelProvenance,
): Promise<BootstrapResult> {
  let created = false
  if (!await schemaExists(query, REGISTRY_SCHEMA)) {
    await query(`CREATE SCHEMA IF NOT EXISTS ${REGISTRY_SCHEMA}`)
    for (const file of ddlFiles) {
      // Simple-query protocol: multi-statement DDL (enums, tables, seeds) in one round trip.
      await query(await readFile(file, 'utf8'))
    }
    created = true
  } else if (!await tableExists(query, REGISTRY_SCHEMA, 'meta_collections')) {
    // The baseline is load-once BY CONTRACT, so a same-named schema is adopted, never
    // re-created. Now that the database belongs to the deployment (one server, many
    // databases — see pg.ts) a half-made or foreign `isahl_meta` is reachable, and
    // adopting it would fail much later as a missing-relation error inside a tool call.
    const database = await query<{ name: string }>('SELECT current_database() AS name')
    throw new Error(
      `env-alioth: database "${database.rows[0]?.name ?? '?'}" already has an \`${REGISTRY_SCHEMA}\` schema `
      + `without \`meta_collections\` — it is not this plugin's registry (the DDL baseline is load-once and is `
      + 'never re-applied over an existing schema). Point ALIOTH_DATABASE_URL at a clean database, or drop the '
      + 'partial schema first (`DROP SCHEMA isahl_meta CASCADE`; `mise run alioth:doctor --reset` does that and '
      + 're-bootstraps).',
    )
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
