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
import type { Client, QueryResult, QueryResultRow } from 'pg'

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

async function schemaExists(client: Client, schema: string): Promise<boolean> {
  const res = await client.query<{ exists: boolean }>(
    'SELECT exists(SELECT 1 FROM information_schema.schemata WHERE schema_name = $1) AS exists',
    [schema],
  )
  return res.rows[0]?.exists === true
}

/** Read the stamp row; `null` when the table or row is absent. */
export async function readStamp(client: Client): Promise<BootstrapStamp | null> {
  const table = await client.query<{ oid: number | null }>(
    'SELECT to_regclass($1) AS oid',
    [`${STAMP_SCHEMA}.model_state`],
  )
  if (table.rows[0]?.oid == null) {
    return null
  }
  const rows = await client.query<QueryResultRow & { model_version: string; source_ref: string; bootstrapped_at: Date }>(
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
  client: Client,
  ddlFiles: readonly string[],
  current: ModelProvenance,
): Promise<BootstrapResult> {
  let created = false
  if (!await schemaExists(client, REGISTRY_SCHEMA)) {
    await client.query(`CREATE SCHEMA IF NOT EXISTS ${REGISTRY_SCHEMA}`)
    for (const file of ddlFiles) {
      // Simple-query protocol: multi-statement DDL (enums, tables, seeds) in one round trip.
      await client.query(await readFile(file, 'utf8'))
    }
    created = true
  }
  await client.query(STAMP_DDL)
  const stamped = await readStamp(client)
  if (stamped === null) {
    const insert: QueryResult = await client.query(
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
