import { describe, expect, it } from 'vitest'
import { extractFkIndex } from '../scripts/generate-semantic-dicts.ts'

/**
 * `meta_fields`' column order as the vendored structure baseline creates it — the order a
 * positional `--inserts` dump writes its values in.
 */
const COLUMNS = [
  'fk_collection', 'name', 'created_at', 'updated_at', 'created_by_id', 'updated_by_id',
  'category', 'data_type', 'is_required', 'default_value', 'config', 'title',
] as const

/** A pg_dump 18 dump envelope, as the model release writes it. */
function dump(...rows: string[]): string {
  return [
    '--',
    '-- PostgreSQL database dump',
    '--',
    '',
    '\\restrict Token',
    '',
    'SET statement_timeout = 0;',
    "SELECT pg_catalog.set_config('search_path', '', false);",
    '',
    '-- Data for Name: meta_fields; Type: TABLE DATA; Schema: isahl_meta; Owner: -',
    '',
    ...rows,
    '',
    '\\unrestrict Token',
    '',
  ].join('\n')
}

/** A SQL literal the way pg_dump writes one (standard_conforming_strings: only `''` escapes). */
function literal(value: string | number | boolean | null): string {
  if (value === null) {
    return 'NULL'
  }
  if (typeof value === 'number' || typeof value === 'boolean') {
    return String(value)
  }
  return `'${value.replaceAll("'", "''")}'`
}

function positionalRow(config: string, name = 'demand', title = '需求'): string {
  const values: Array<string | number | boolean | null> = [
    'zc_id_contract', name, '2026-09-11 16:43:17.609646+08', '2026-09-11 16:43:17.609646+08',
    1, 1, 'reference', 'm2o', false, '', config, title,
  ]
  return `INSERT INTO isahl_meta.meta_fields VALUES (${values.map(literal).join(', ')}) ON CONFLICT DO NOTHING;`
}

describe('extractFkIndex', () => {
  it('reads a positional INSERT row through the baseline column order', () => {
    const refs = extractFkIndex(
      dump(positionalRow('{"unique": false, "reference_config": {"local_key": "fk_plan", "target_table": "zc_id_plan"}}')),
      COLUMNS,
    )
    expect(refs).toEqual([['zc_id_contract', 'demand', 'zc_id_plan', 'fk_plan']])
  })

  it('skips a junction-only reference (no local_key) and a row without config', () => {
    const refs = extractFkIndex(
      dump(
        positionalRow('{"reference_config": {"target_table": "zc_id_stus-agreement", "junction_table": "zc_id_lifecycle_r_primary-status"}}', 'status'),
        positionalRow('{"unique": false}', 'plain'),
      ),
      COLUMNS,
    )
    expect(refs).toEqual([])
  })

  it('reads a column-named row by column name, not by position', () => {
    const row = 'INSERT INTO isahl_meta.meta_fields (fk_collection, name, config) VALUES '
      + "('zc_id_contract', 'plan', '{\"reference_config\": {\"local_key\": \"fk_plan\", \"target_table\": \"zc_id_plan\"}}');"
    const refs = extractFkIndex(dump(row), COLUMNS)
    expect(refs).toEqual([['zc_id_contract', 'plan', 'zc_id_plan', 'fk_plan']])
  })

  it('keeps a row whose literal spans lines (apostrophe, semicolon, newline)', () => {
    const config = '{"description": "it\'s; a note", "reference_config": {"local_key": "fk_plan", "target_table": "zc_id_plan"}}'
    const refs = extractFkIndex(dump(positionalRow(config, 'demand', '需求\n第二行')), COLUMNS)
    expect(refs).toEqual([['zc_id_contract', 'demand', 'zc_id_plan', 'fk_plan']])
  })

  it('refuses a dump whose rows it cannot recognise instead of indexing zero references', () => {
    const copy = dump('COPY isahl_meta.meta_fields (fk_collection) FROM stdin;')
    expect(() => extractFkIndex(copy, COLUMNS)).toThrow(/COPY dump/)
    const other = dump("INSERT INTO isahl_meta.meta_collections (table_name) VALUES ('zc_id_plan');")
    expect(() => extractFkIndex(other, COLUMNS)).toThrow(/dump format changed/)
  })

  it('indexes a release-sized sidecar without silently reading none', () => {
    // The registry sidecar is derived data delivered outside Git, so this exercises the same shape
    // at scale rather than reading a copy out of the package; check:dicts parses the real one
    // against the model source. A shifted column or an unrecognised statement reads ZERO refs —
    // the silent failure this guards against.
    const rows = Array.from({ length: 1200 }, (_, i) => positionalRow(
      `{"unique": false, "reference_config": {"local_key": "fk_plan", "target_table": "zc_id_plan_${i}"}}`,
      `field_${i}`,
    ))
    const refs = extractFkIndex(dump(...rows), COLUMNS)
    expect(refs.length).toBe(1200)
    for (const ref of refs.slice(0, 50)) {
      expect(ref).toHaveLength(4)
      expect(ref.every(part => typeof part === 'string' && part.length > 0)).toBe(true)
    }
  })
})
