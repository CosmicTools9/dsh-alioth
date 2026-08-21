/**
 * Sync the vendored isahl_meta registry baseline (env-alioth/vendor/backend/
 * ddl/003|004_isahl_meta_seed_*.sql) from a live AliothStudio registry DB.
 *
 * The seeds are `sync_from_database` artifacts: the isahl_meta rows that
 * describe the current model's physical tables. The authoritative source is
 * the AliothStudio registry DB (the model repo ships only the physical DDL;
 * the registry adds names, categories, inheritance, depth, coordinates).
 * Run whenever the model evolves (tables added/renamed/removed):
 *
 *   DATABASE_URL=postgres://isahl@localhost/aliothstudio_dev node --import tsx scripts/sync-vendor-registry.ts
 *
 * Then refresh the provenance manifest:
 *
 *   pnpm run check:vendor --update
 *
 * The local registry must be re-bootstrapped afterwards so tools validate
 * against the current model (mise run alioth:doctor --reset, then restart
 * the web instance); the semantic index rebuilds automatically on the next
 * search (entries hash changed).
 * @module scripts/sync-vendor-registry
 */

import { writeFile } from 'node:fs/promises'
import { createRequire } from 'node:module'
import path from 'node:path'
import { fileURLToPath, pathToFileURL } from 'node:url'

const SCRIPT_DIR = path.dirname(fileURLToPath(import.meta.url))
const VENDOR_DDL = path.resolve(SCRIPT_DIR, '..', 'packages', 'alioth', 'env-alioth', 'vendor', 'backend', 'ddl')

// `pg` is a dependency of @dsh-alioth/env-alioth (pnpm strict layout — not
// resolvable from root scripts/). Anchor the require at the package so this
// dev tool needs no extra root dependency.
const require = createRequire(path.join(SCRIPT_DIR, '..', 'packages', 'alioth', 'env-alioth', 'package.json'))
interface PgClientLike {
  connect(): Promise<void>
  end(): Promise<void>
  query<T = Record<string, unknown>>(sql: string): Promise<{ rows: T[] }>
}
const { Client } = require('pg') as { Client: new (options: { connectionString: string }) => PgClientLike }

const DATABASE_URL = process.env.DATABASE_URL ?? 'postgres://isahl@localhost/aliothstudio_dev'

/** SQL literal: NULL for null/undefined, ISO for Dates, numbers bare, everything else quoted with '' escaping. */
function lit(value: unknown): string {
  if (value === null || value === undefined) {
    return 'NULL'
  }
  if (value instanceof Date) {
    return `'${value.toISOString()}'`
  }
  if (typeof value === 'number' || typeof value === 'bigint' || typeof value === 'boolean') {
    return String(value)
  }
  return `'${String(value).replace(/'/g, "''")}'`
}

interface CollectionRow {
  table_name: string
  created_at: string
  updated_at: string
  created_by_id: string | null
  updated_by_id: string | null
  name: string
  type: string
  config: string | null
  data_source: string | null
  schema: string | null
  biz_description: string | null
}

interface FieldRow {
  fk_collection: string
  name: string
  created_at: string
  updated_at: string
  created_by_id: string | null
  updated_by_id: string | null
  category: string
  data_type: string
  is_required: boolean
  default_value: string | null
  config: string | null
  title: string
}

async function main(): Promise<void> {
  const client = new Client({ connectionString: DATABASE_URL })
  await client.connect()
  try {
    const collections = (await client.query<CollectionRow>(
      `SELECT table_name, created_at, updated_at, created_by_id, updated_by_id, name, type,
              config::text AS config, data_source, schema, biz_description
         FROM isahl_meta.meta_collections ORDER BY table_name`,
    )).rows
    const fields = (await client.query<FieldRow>(
      `SELECT fk_collection, name, created_at, updated_at, created_by_id, updated_by_id,
              category, data_type, is_required, default_value, config::text AS config, title
         FROM isahl_meta.meta_fields ORDER BY fk_collection, name`,
    )).rows

    if (collections.length === 0) {
      throw new Error(`sync-vendor-registry: no meta_collections at ${DATABASE_URL} — wrong database?`)
    }

    const collectionSql = collections.map(row =>
      `INSERT INTO isahl_meta.meta_collections VALUES (${[
        lit(row.table_name), lit(row.created_at), lit(row.updated_at),
        lit(row.created_by_id), lit(row.updated_by_id), lit(row.name), lit(row.type),
        lit(row.config), lit(row.data_source), lit(row.schema), lit(row.biz_description),
      ].join(', ')}) ON CONFLICT DO NOTHING;`).join('\n')
    const fieldSql = fields.map(row =>
      `INSERT INTO isahl_meta.meta_fields VALUES (${[
        lit(row.fk_collection), lit(row.name), lit(row.created_at), lit(row.updated_at),
        lit(row.created_by_id), lit(row.updated_by_id), lit(row.category), lit(row.data_type),
        lit(row.is_required), lit(row.default_value), lit(row.config), lit(row.title),
      ].join(', ')}) ON CONFLICT DO NOTHING;`).join('\n')

    const header = (count: string): string =>
      `-- dsh-alioth vendored isahl_meta registry baseline — synced from a live AliothStudio\n`
      + `-- registry (${DATABASE_URL.replace(/postgres:\/\/[^@]*@/, 'postgres://***@')}) on\n`
      + `-- ${new Date().toISOString()}. ${count} rows. Regenerate with scripts/sync-vendor-registry.ts.\n`

    await writeFile(path.join(VENDOR_DDL, '003_isahl_meta_seed_collections.sql'),
      `${header(String(collections.length))}${collectionSql}\n`)
    await writeFile(path.join(VENDOR_DDL, '004_isahl_meta_seed_fields.sql'),
      `${header(String(fields.length))}${fieldSql}\n`)

    console.log(`sync-vendor-registry: ${collections.length} collections, ${fields.length} fields`)
    console.log(`written to ${VENDOR_DDL}`)
  } finally {
    await client.end()
  }
}

const isEntry = process.argv[1] !== undefined
  && import.meta.url === pathToFileURL(process.argv[1]).href
if (isEntry) {
  main().catch(error => {
    console.error(`sync-vendor-registry failed: ${error instanceof Error ? error.message : String(error)}`)
    process.exitCode = 1
  })
}
