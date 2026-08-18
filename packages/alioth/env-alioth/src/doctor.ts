/**
 * Read-only environment health checks. Doctor never mutates: it inspects a
 * resolved environment (snapshot + connected client) and reports a structured
 * green/red verdict with one line of evidence per check.
 * @module @dsh-alioth/env-alioth/doctor
 */

import { readFile } from 'node:fs/promises'
import path from 'node:path'
import type { Client } from 'pg'
import type { ModelSnapshot } from './model-source.ts'
import { readStamp } from './bootstrap.ts'

export interface DoctorCheck {
  readonly name: string
  readonly ok: boolean
  readonly detail: string
}

export interface DoctorReport {
  readonly status: 'green' | 'red'
  readonly checks: readonly DoctorCheck[]
}

/** `postgres://user:***@host/db` — credentials never reach a report. */
export function maskUrl(url: string): string {
  return url.replace(/(postgres(?:ql)?:\/\/[^:/@]+:)[^@]+@/, '$1***@')
}

async function checkDatabase(client: Client): Promise<DoctorCheck> {
  const res = await client.query<{ version: string }>('SELECT version()')
  const version = res.rows[0]?.version ?? 'unknown'
  return { name: 'database', ok: true, detail: version.split(',')[0] ?? 'connected' }
}

async function checkRegistrySchema(client: Client): Promise<DoctorCheck> {
  const res = await client.query<{ table_name: string }>(
    "SELECT table_name FROM information_schema.tables WHERE table_schema = 'isahl_meta'",
  )
  const tables = res.rows.map(row => row.table_name)
  const missing = ['meta_collections', 'meta_fields'].filter(table => !tables.includes(table))
  return missing.length === 0
    ? { name: 'isahl-meta', ok: true, detail: `${tables.length} tables incl. meta_collections, meta_fields` }
    : { name: 'isahl-meta', ok: false, detail: `schema present but missing: ${missing.join(', ')}` }
}

async function checkStamp(client: Client, snapshot: ModelSnapshot): Promise<DoctorCheck> {
  const stamp = await readStamp(client)
  if (stamp === null) {
    return { name: 'model-stamp', ok: false, detail: 'no dsh_alioth.model_state row — registry not bootstrapped by this plugin' }
  }
  if (stamp.sourceRef !== snapshot.sourceRef) {
    return {
      name: 'model-stamp',
      ok: false,
      detail: `registry stamped ${stamp.sourceRef.slice(0, 12)} (model ${stamp.modelVersion}), snapshot is ${snapshot.sourceRef.slice(0, 12)} — model drift; pin the stamped source or reset the registry`,
    }
  }
  if (stamp.modelVersion !== snapshot.modelVersion) {
    return {
      name: 'model-stamp',
      ok: false,
      detail: `registry stamped model ${stamp.modelVersion}, snapshot is ${snapshot.modelVersion} — model drift`,
    }
  }
  return { name: 'model-stamp', ok: true, detail: `${snapshot.sourceRef.slice(0, 12)} @ model ${snapshot.modelVersion}` }
}

/** Semantic index + dictionary-snapshot observability (informational checks). */
async function checkSemanticIndex(dataRoot: string): Promise<DoctorCheck> {
  try {
    const meta = JSON.parse(await readFile(path.join(dataRoot, 'semantic', 'meta.json'), 'utf8')) as { count?: number; dimension?: number; model?: string }
    return {
      name: 'semantic-index',
      ok: true,
      detail: `${meta.count ?? 0} entries, ${meta.dimension ?? 0} dims, ${meta.model ?? 'unknown'}`,
    }
  } catch {
    return {
      name: 'semantic-index',
      ok: false,
      detail: 'not built — run mise run alioth:rebuild-semantic (auto-rebuilds on next semantic_search otherwise)',
    }
  }
}

/** Snapshot freshness reminder: dict snapshots ship with the code, not the model tree. */
async function checkDictionarySnapshots(modelDir: string): Promise<DoctorCheck> {
  void modelDir
  return {
    name: 'dictionary-snapshots',
    ok: true,
    detail: 'shipped with the code (skill-alioth/src/data/); model is FROZEN (builtin) — dict snapshots change only with a new plugin release',
  }
}

/**
 * Run all checks. Each check's failure is contained: a throwing check becomes
 * `ok: false` with the error message as evidence, never aborting the report.
 */
export async function runDoctor(client: Client, snapshot: ModelSnapshot, dataRoot?: string): Promise<DoctorReport> {
  const artifacts = snapshot.artifacts
  const checks: DoctorCheck[] = [
    {
      name: 'model-snapshot',
      ok: true,
      detail: `${artifacts.ddlFiles.length} isahl_meta DDL, ${artifacts.skillAdapterFiles.length} adapters, `
        + `${artifacts.artifactSchemaFiles.length} schemas @ ${snapshot.sourceRef.slice(0, 12)} (model ${snapshot.modelVersion})`,
    },
  ]
  for (const run of [checkDatabase, checkRegistrySchema, (c: Client) => checkStamp(c, snapshot)]) {
    try {
      checks.push(await run(client))
    } catch (error) {
      checks.push({ name: 'unknown', ok: false, detail: error instanceof Error ? error.message : String(error) })
    }
  }
  if (dataRoot !== undefined) {
    checks.push(await checkSemanticIndex(dataRoot))
  }
  checks.push(await checkDictionarySnapshots(snapshot.dir))
  return { status: checks.every(check => check.ok) ? 'green' : 'red', checks }
}
