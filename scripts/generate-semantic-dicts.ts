/**
 * Generate the semantic-mapping library as files, fully offline:
 * - coordinates.json: scene/factor/function codes from the Alioth repo's
 *   `seed-dimensions.sql` (version anchor from `latest.json`)
 * - physical-tables.json: isahl tables + inheritance + root columns from
 *   `002_isahl_tables.sql`
 * - fk-index.json: physical FK references from the **release's** registry sidecar
 *   (`<model source>/isahl_meta-registry.sql`, a `pg_dump --data-only --inserts` copy the model
 *   publish pipeline generates — derived data, never committed to Git, delivered out of band),
 *   read through the same loader the boot uses. Absent sidecar ⇒ the index is not regenerated.
 *
 * Everything comes from the model source (`ALIOTH_REPO` / `ALIOTH_MODEL_SOURCE`), never from a
 * copy vendored in this package: the package carries the structure baseline and the kit
 * (adapters, prototype chain), the model source carries the model's own content.
 *
 * The library ships with the plugin — no dev-database dependency.
 * Usage: ALIOTH_REPO=~/WorkSpace/Alioth node --import tsx scripts/generate-semantic-dicts.ts
 */
import { readFile, writeFile, mkdir, stat } from 'node:fs/promises'
import path from 'node:path'
import { fileURLToPath, pathToFileURL } from 'node:url'
import { sanitizeRegistryDdl } from '@dsh-alioth/env-alioth'

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

/**
 * The registry's column order for a table, read from the vendored structure baseline. A
 * positional `--inserts` row carries no column names, so its values are named by this order —
 * deriving it from the DDL the registry is actually created with means a baseline change cannot
 * silently shift `config` into another field the way a hardcoded index list would.
 */
function registryColumnOrder(baselineSql: string, table: string): string[] {
  const columns: string[] = []
  let inside = false
  for (const line of baselineSql.split('\n')) {
    if (!inside) {
      inside = line.startsWith(`CREATE TABLE isahl_meta.${table} (`)
      continue
    }
    if (line.trim() === ');') {
      break
    }
    const column = /^\s{4}([a-z_][a-z0-9_]*)\s/.exec(line)
    if (column !== null) {
      columns.push(column[1] ?? '')
    }
  }
  return columns
}

/**
 * Split a `--inserts` dump into statements. pg_dump writes one `INSERT` per line, but a value may
 * contain a newline (a description written across lines), so a statement ends at the first `;`
 * outside a string literal instead of at the end of a line. Comment, meta-command and session
 * lines are dropped by the loader before this runs.
 */
function insertStatements(source: string): string[] {
  const statements: string[] = []
  let current = ''
  let inString = false
  for (const line of source.split('\n')) {
    // Comments and blank lines carry no statement — keeping them would fuse a comment banner onto
    // the row that follows it (`-- Data for Name: …` precedes the first INSERT of each table).
    if (!inString && /^\s*(?:--.*)?$/.test(line)) {
      continue
    }
    current += `${line}\n`
    let quotes = 0
    for (const ch of line) {
      if (ch === "'") {
        quotes++
      }
    }
    if (quotes % 2 === 1) {
      inString = !inString
    }
    if (!inString && current.trimEnd().endsWith(';')) {
      statements.push(current.trim())
      current = ''
    }
  }
  return statements
}

/**
 * Parse fk references from the vendored registry sidecar. Both faces of `pg_dump` are read:
 * `--inserts` (positional values, named through `registryColumnOrder`) and `--column-inserts`
 * (the statement names its columns) — values are never taken by a fixed index without that
 * mapping, so a column the dump reorders must not silently shift `config` into another field.
 * @param source - the sidecar's text.
 * @param columnOrder - the `meta_fields` column order from the vendored baseline.
 * @throws when the dump yields no `meta_fields` rows at all — that means the sidecar's face
 *   changed and the index would silently come out empty (entity validation reads it).
 */
export function extractFkIndex(source: string, columnOrder: readonly string[]): Array<[string, string, string, string]> {
  const refs: Array<[string, string, string, string]> = []
  let rows = 0
  for (const statement of insertStatements(sanitizeRegistryDdl(source, 'vendored isahl_meta-registry.sql'))) {
    const match = /^INSERT INTO isahl_meta\.meta_fields\s*(?:\(([^)]*)\)\s*)?VALUES \(([\s\S]*)\)(?:\s+ON CONFLICT DO NOTHING)?;?$/.exec(statement)
    if (match === null) {
      continue
    }
    // A positional `--inserts` row has no column list at all: `splitTopLevel('')` yields one empty
    // entry, which must NOT be mistaken for "the statement names its columns".
    const named = splitTopLevel(match[1] ?? '').map(column => column.trim()).filter(column => column.length > 0)
    const values = splitTopLevel(match[2] ?? '')
    const columns = named.length > 0 ? named : columnOrder
    const row: Record<string, string> = {}
    columns.forEach((column, index) => {
      row[column] = values[index] ?? ''
    })
    rows++
    const table = unquote(row.fk_collection ?? '')
    const name = unquote(row.name ?? '')
    const configRaw = unquote(row.config ?? '')
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
  if (rows === 0) {
    throw new Error(
      'generate-semantic-dicts: no `INSERT INTO isahl_meta.meta_fields … VALUES (…);` row in the '
      + 'vendored registry sidecar — its dump format changed, and an empty fk index would silently '
      + 'weaken entity validation. Refresh it from the model source (its release sidecar).',
    )
  }
  return refs
}

async function read(pathStr: string): Promise<string> {
  return readFile(pathStr, 'utf8')
}

/**
 * The release directory inside a model source: `<repo>/<version>` when that directory exists,
 * else the source root (both publication layouts). Exported so the freshness gate resolves the
 * same directory the generator writes from.
 * @param repoDir - model source root.
 */
export async function resolveReleaseDir(repoDir: string): Promise<{ releaseDir: string; version: string; publishedAt: string }> {
  const latest = JSON.parse(await read(path.join(repoDir, 'latest.json'))) as { version: string; published_at: string }
  const versioned = path.join(repoDir, latest.version)
  const releaseDir = await stat(versioned).then(() => versioned, () => repoDir)
  return { releaseDir, version: latest.version, publishedAt: latest.published_at }
}

/** Generate the dictionaries into `targetDir`. Exported for the
 * freshness gate (scripts/check-semantic-dicts.ts regenerates into a temp
 * dir and diffs against the checked-in files). */
export async function generateDicts(
  targetDir: string,
  repoDir = ALIOTH_REPO,
): Promise<{ source: string; fkIndex: boolean }> {
  // 发行物布局有两态：2026-09-21 起上游模型发布线把产物改为**平铺固定路径**——文件落在仓库根，
  // 版本号只由 `latest.json` 与 annotated tag 承载；更早的发行物仍在 `<repo>/<version>/` 下。
  // 两种都认：有版本目录用版本目录，没有回落仓库根。（只认版本目录会让模型一发新版、
  // CI 的 check:dicts 立刻 ENOENT 全红。）
  const { releaseDir, version, publishedAt } = await resolveReleaseDir(repoDir)
  const seeds = await read(path.join(releaseDir, 'seed-dimensions.sql'))
  const tablesDdl = await read(path.join(releaseDir, '002_isahl_tables.sql'))
  // The registry rows are DERIVED data: the model publish pipeline generates the sidecar into its
  // work tree and it is never committed to Git, so a Git clone (CI) legitimately has none. Absent
  // ⇒ no fk index is produced; the caller reports that instead of shipping a stale one.
  const fkSidecar = await readFile(path.join(releaseDir, 'isahl_meta-registry.sql'), 'utf8').then(
    text => text,
    () => undefined,
  )
  const registryBaseline = await read(path.join(VENDOR_DDL, '002_isahl_meta_schema.sql'))

  const scene = extractCodes(seeds, 'zc_id_scene')
  const factor = extractCodes(seeds, 'zc_id_factor')
  const func = extractCodes(seeds, 'zc_id_function')
  const tables = extractTables(tablesDdl)
  const rootColumns = extractRootColumns(tablesDdl)
  const refs = fkSidecar === undefined
    ? undefined
    : extractFkIndex(fkSidecar, registryColumnOrder(registryBaseline, 'meta_fields'))

  await mkdir(targetDir, { recursive: true })
  // The `source` string must NOT depend on whether the sidecar was present: the freshness gate
  // regenerates from whatever model source it has (CI clones Git, which carries no sidecar), and a
  // source-dependent string would make the two dictionaries differ for reasons that are not drift.
  // The sidecar's identity is recorded inside fk-index.json, which is only written when it exists.
  const provenance = { source: `Alioth repo ${version} (${publishedAt})` }
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
  if (refs !== undefined) {
    await writeFile(path.join(targetDir, 'fk-index.json'), JSON.stringify({
      $schema: 'https://dsh-alioth.local/schemas/fk-index.json',
      description: 'Physical FK reference index [table, field, target, local_key] from the release isahl_meta-registry.sql sidecar (derived data, delivered outside Git).',
      ...provenance,
      refs,
    }, null, 1) + '\n')
  }
  console.log(`coordinates: scene=${scene.length} factor=${factor.length} function=${func.length}`)
  console.log(`physical-tables: ${tables.length} tables, ${rootColumns.length} root columns`)
  console.log(refs === undefined
    ? `fk-index: NOT regenerated — no isahl_meta-registry.sql sidecar at ${releaseDir}`
    : `fk-index: ${refs.length} refs`)
  return { source: provenance.source, fkIndex: refs !== undefined }
}

async function main(): Promise<void> {
  const { source, fkIndex } = await generateDicts(DATA_DIR)
  if (!fkIndex) {
    // Probe-and-ignore: the sidecar is derived data delivered out of band, so a clone legitimately
    // has none. The other two dictionaries still regenerate; fk-index.json stays as shipped (its
    // bytes stay anchored — the anchor hashes what is on disk, and `source` records the absence).
    console.warn('generate-semantic-dicts: model source carries no isahl_meta-registry.sql sidecar —')
    console.warn('  fk-index.json left as is; the other two dictionaries were regenerated.')
  }
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
