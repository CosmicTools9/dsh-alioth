/**
 * Iron-rule gate: no SQL in this workspace names a relation in the model's own `isahl` schema.
 *
 * `isahl` is the Alioth model's data space (its several hundred physical tables, shared with the
 * NS:Alioth deployment). dsh-alioth consumes the model — it never writes there, and it does not
 * read model tables either: everything it needs from the model arrives as published content (the
 * physical DDL, the dimension seeds, the registry rows) or as the dict snapshots generated from
 * them. That rule used to be discipline only; this gate turns it into a mechanical one, because
 * the schema name is exactly the kind of literal that survives review and breaks in production.
 *
 * Named schemas that merely share the prefix are fine (`isahl_meta` is this plugin's registry,
 * `isahl_auth`/`isahl_audit`/`isahl_knowledge` belong to other deployments).
 *
 * @module tests/iron-rule-no-isahl-sql
 */
import { readdirSync, readFileSync, statSync } from 'node:fs'
import path from 'node:path'
import { fileURLToPath } from 'node:url'
import { describe, expect, it } from 'vitest'

const ROOT = path.resolve(path.dirname(fileURLToPath(import.meta.url)), '..')

/** A statement naming `isahl.<relation>` (quoted or bare; `isahl_meta` et al. do not match). */
const ISAHL_SQL = /\b(?:FROM|JOIN|INTO|UPDATE|TABLE|TRUNCATE)\s+"?isahl"?\s*\./gi

/** Every `.ts` under `dir`, skipping vendored trees and test fixtures (they may build sample schemas). */
function sources(dir: string): string[] {
  const out: string[] = []
  for (const entry of readdirSync(dir)) {
    if (entry === 'node_modules' || entry === 'vendor' || entry === 'dist' || entry === 'lib') {
      continue
    }
    const full = path.join(dir, entry)
    if (statSync(full).isDirectory()) {
      out.push(...sources(full))
    } else if (entry.endsWith('.ts') && !entry.endsWith('.spec.ts')) {
      out.push(full)
    }
  }
  return out
}

describe('iron rule: no SQL against schema isahl', () => {
  it('never names isahl.<relation> in any shipped source', () => {
    const scanned = [
      ...sources(path.join(ROOT, 'packages', 'alioth')),
      ...sources(path.join(ROOT, 'scripts')),
    ]
    // A broken walk would otherwise pass vacuously.
    expect(scanned.length).toBeGreaterThan(50)
    const offenders: string[] = []
    for (const file of scanned) {
      for (const match of readFileSync(file, 'utf8').matchAll(ISAHL_SQL)) {
        offenders.push(`${path.relative(ROOT, file)}: ${match[0]}`)
      }
    }
    expect(offenders).toEqual([])
  })
})
