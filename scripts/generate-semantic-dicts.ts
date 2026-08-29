/**
 * Generate the semantic-mapping library as files, fully offline:
 * - coordinates.json: scene/factor/function codes from the Alioth repo's
 *   `seed-dimensions.sql` (version anchor from `latest.json`)
 * - physical-tables.json: isahl tables + inheritance + root columns from
 *   `002_isahl_tables.sql`
 * - fk-index.json: physical FK references from the vendored isahl_meta seed
 *   (`env-alioth/vendor/backend/ddl/004_isahl_meta_seed_fields.sql`)
 *
 * The library ships with the plugin — no dev-database dependency.
 * Usage: ALIOTH_REPO=~/WorkSpace/Alioth node --import tsx scripts/generate-semantic-dicts.ts
 */
import { readFile, writeFile, mkdir } from 'node:fs/promises'
import path from 'node:path'
import { fileURLToPath, pathToFileURL } from 'node:url'

const ALIOTH_REPO = process.env.ALIOTH_REPO ?? path.join(process.env.HOME ?? '', 'WorkSpace', 'Alioth')
const SCRIPT_DIR = path.dirname(fileURLToPath(import.meta.url))
const DATA_DIR = path.resolve(SCRIPT_DIR, '..', 'packages', 'alioth', 'skill-alioth', 'src', 'data')
const VENDOR_DDL = path.resolve(SCRIPT_DIR, '..', 'packages', 'alioth', 'env-alioth', 'vendor', 'backend', 'ddl')

/** Split a SQL VALUES tuple on top-level commas (quote-aware; handles commas inside strings). */
function splitTopLevel(value: string): string[] {
  const parts: string[] = []
  let current = ''
  let quote: string | null = null
  for (const ch of value) {
    if (quote !== null) {
      current += ch
      if (ch === quote) quote = null
      continue
    }
    if (ch === "'" || ch === '"') {
      quote = ch
      current += ch
      continue
    }
    if (ch === ',') {
      parts.push(current)
      current = ''
      continue
    }
    current += ch
  }
  parts.push(current)
  return parts
}

function unquote(value: string): string {
  const trimmed = value.trim()
  if (trimmed.length >= 2 && (trimmed.startsWith("'") || trimmed.startsWith('"'))) {
    return trimmed.slice(1, -1).replaceAll("''", "'")
  }
  return trimmed
}

function topLevelName(value: string): string {
  return unquote(value).split('.').at(-1) ?? ''
}

/** Parse coordinate codes from the dimension seed: code is the 10th column of each INSERT row. */
function extractCodes(source: string, table: string): string[] {
  const codes: string[] = []
  const pattern = new RegExp(`INSERT INTO isahl\\.${table}\\s+VALUES\\s*\\(`)
  for (const line of source.split('\n')) {
    const match = pattern.exec(line)
    if (match === null) continue
    const rest = line.slice(match[0].length)
    const closing = rest.lastIndexOf(')')
    if (closing === -1) continue
    const fields = splitTopLevel(rest.slice(0, closing))
    const code = unquote(fields[9] ?? '')
    if (code.length > 0) codes.push(code)
  }
  return [...new Set(codes)].sort()
}

/** Parse [table, parent] pairs from the tables DDL. */
function extractTables(source: string): Array<[string, string]> {
  const tables: Array<[string, string]> = []
  let current: string | null = null
  for (const line of source.split('\n')) {
    const create = /CREATE TABLE (?:IF NOT EXISTS )?(?:isahl\.)?(\S+)/.exec(line)
    if (create !== null) {
      current = unquote(create[1] ?? '')
      tables.push([current, ''])
      continue
    }
    const inherits = /INHERITS\s*\(\s*(?:isahl\.)?([^)]+)\)/.exec(line)
    if (inherits !== null && current !== null) {
      const parents = (inherits[1] ?? '').split(',').map(part => topLevelName(part)).filter(Boolean)
      tables[tables.length - 1] = [current, parents[0] ?? '']
      current = null
    }
  }
  return tables
}

/** Root columns: columns of tables that inherit nothing (root family tables). */
function extractRootColumns(source: string): string[] {
  const roots = new Set<string>()
  const columnSets = new Map<string, string[]>()
  let current: string | null = null
  let columns: string[] = []
  let inherits = false
  for (const line of source.split('\n')) {
    const create = /CREATE TABLE (?:IF NOT EXISTS )?(?:isahl\.)?(\S+)/.exec(line)
    if (create !== null) {
      if (current !== null) columnSets.set(current, columns)
      current = unquote(create[1] ?? '')
      columns = []
      inherits = false
      continue
    }
    if (current === null) continue
    if (INHERITS_RE.test(line)) {
      inherits = true
      continue
    }
    const col = /^\s{4}([a-z_][a-z0-9_]*)\s/.exec(line)
    if (col !== null && !line.trim().startsWith('PRIMARY') && !line.trim().startsWith('UNIQUE') && !line.trim().startsWith('CONSTRAINT')) {
      columns.push(col[1] ?? '')
    }
    if (line.trim() === ');') {
      columnSets.set(current, columns)
      if (!inherits) roots.add(current)
      current = null
    }
  }
  if (current !== null) columnSets.set(current, columns)
  if (!inherits) roots.add(current ?? '')
  const out = new Set<string>()
  for (const root of roots) {
    for (const column of columnSets.get(root) ?? []) out.add(column)
  }
  return [...out].sort()
}

const INHERITS_RE = /INHERITS\s*\(/

/** Parse fk references from the vendored isahl_meta field seed. */
function extractFkIndex(source: string): Array<[string, string, string, string]> {
  const refs: Array<[string, string, string, string]> = []
  for (const line of source.split('\n')) {
    const match = /INSERT INTO isahl_meta\.meta_fields\s+VALUES\s*\(/.exec(line)
    if (match === null) continue
    const rest = line.slice(match[0].length)
    const closing = rest.lastIndexOf(')')
    if (closing === -1) continue
    const fields = splitTopLevel(rest.slice(0, closing))
    const table = unquote(fields[0] ?? '')
    const name = unquote(fields[1] ?? '')
    const configRaw = unquote(fields[10] ?? '')
    if (table.length === 0 || name.length === 0 || configRaw.length === 0) continue
    try {
      const config = JSON.parse(configRaw) as { reference_config?: { target_table?: string; local_key?: string } }
      const rc = config.reference_config
      if (rc?.local_key !== undefined && rc.local_key.length > 0 && rc.target_table !== undefined) {
        refs.push([table, name, rc.target_table, rc.local_key])
      }
    } catch {
      // malformed config row: skip
    }
  }
  return refs
}

async function read(pathStr: string): Promise<string> {
  return readFile(pathStr, 'utf8')
}

/** Generate the three dictionaries into `targetDir`. Exported for the
 * freshness gate (scripts/check-semantic-dicts.ts regenerates into a temp
 * dir and diffs against the checked-in files). */
export async function generateDicts(targetDir: string, repoDir = ALIOTH_REPO): Promise<{ source: string }> {
  const latest = JSON.parse(await read(path.join(repoDir, 'latest.json'))) as { version: string; published_at: string }
  const versionDir = path.join(repoDir, latest.version)
  const seeds = await read(path.join(versionDir, 'seed-dimensions.sql'))
  const tablesDdl = await read(path.join(versionDir, '002_isahl_tables.sql'))
  const fkSeed = await read(path.join(VENDOR_DDL, '004_isahl_meta_seed_fields.sql'))

  const scene = extractCodes(seeds, 'zc_id_scene')
  const factor = extractCodes(seeds, 'zc_id_factor')
  const func = extractCodes(seeds, 'zc_id_function')
  const tables = extractTables(tablesDdl)
  const rootColumns = extractRootColumns(tablesDdl)
  const refs = extractFkIndex(fkSeed)

  await mkdir(targetDir, { recursive: true })
  const provenance = { source: `Alioth repo ${latest.version} (${latest.published_at}) + vendored isahl_meta seed` }
  await writeFile(path.join(targetDir, 'coordinates.json'), JSON.stringify({
    $schema: 'https://dsh-alioth.local/schemas/coordinates-dict.json',
    description: 'Alioth coordinate dictionaries, generated offline from the Alioth model repo (semantic-mapping library shipped with the plugin).',
    ...provenance,
    scene, factor, function: func,
  }, null, 1) + '\n')
  await writeFile(path.join(targetDir, 'physical-tables.json'), JSON.stringify({
    $schema: 'https://dsh-alioth.local/schemas/physical-tables.json',
    description: 'isahl physical table index [table, parent] + root-family common columns, generated offline from the Alioth model repo.',
    ...provenance,
    root_columns: rootColumns,
    tables,
  }, null, 1) + '\n')
  await writeFile(path.join(targetDir, 'fk-index.json'), JSON.stringify({
    $schema: 'https://dsh-alioth.local/schemas/fk-index.json',
    description: 'Physical FK reference index [table, field, target, local_key] from the vendored isahl_meta seed.',
    ...provenance,
    refs,
  }, null, 1) + '\n')
  console.log(`coordinates: scene=${scene.length} factor=${factor.length} function=${func.length}`)
  console.log(`physical-tables: ${tables.length} tables, ${rootColumns.length} root columns`)
  console.log(`fk-index: ${refs.length} refs`)
  return { source: provenance.source }
}

async function main(): Promise<void> {
  const { source } = await generateDicts(DATA_DIR)
  // Anchor: tamper-evidence for the checked-in library. The freshness gate
  // (check-semantic-dicts.ts) verifies hashes and, when ALIOTH_REPO is set,
  // regenerates and diffs.
  const { createHash } = await import('node:crypto')
  const files: Record<string, string> = {}
  for (const name of ['coordinates.json', 'physical-tables.json', 'fk-index.json']) {
    files[name] = createHash('sha256').update(await read(path.join(DATA_DIR, name))).digest('hex')
  }
  await writeFile(path.join(DATA_DIR, 'anchor.json'), JSON.stringify({
    description: 'Semantic-library anchor: sha256 of the generated dictionaries. Regenerated by scripts/generate-semantic-dicts.ts; verified by scripts/check-semantic-dicts.ts.',
    source,
    files,
  }, null, 2) + '\n')
  console.log(`anchor.json written (3 files hashed)`)
  console.log(`written to ${DATA_DIR}`)
}

// CLI entry only — importing (freshness gate) must not regenerate.
const isEntry = process.argv[1] !== undefined
  && import.meta.url === pathToFileURL(process.argv[1]).href
if (isEntry) {
  main().catch(error => {
    console.error(error)
    process.exitCode = 1
  })
}
