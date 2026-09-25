import { readFile } from 'node:fs/promises'
import { fileURLToPath } from 'node:url'
import { describe, expect, it } from 'vitest'
import { sanitizeRegistryDdl } from '../src/registry-ddl.ts'

/** A minimal dump in the shape PostgreSQL 18 emits for the registry sidecar. */
function dump(...body: string[]): string {
  return [
    '--',
    '-- PostgreSQL database dump',
    '--',
    '',
    '\\restrict AbC123',
    '',
    '-- Dumped from database version 18.6 (Homebrew)',
    '-- Dumped by pg_dump version 18.6 (Homebrew)',
    '',
    'SET statement_timeout = 0;',
    'SET lock_timeout = 0;',
    "SET client_encoding = 'UTF8';",
    'SET standard_conforming_strings = on;',
    "SELECT pg_catalog.set_config('search_path', '', false);",
    'SET client_min_messages = warning;',
    'SET row_security = off;',
    '',
    ...body,
    '',
    '\\unrestrict AbC123',
    '',
  ].join('\n')
}

describe('sanitizeRegistryDdl', () => {
  it('drops the psql meta-command envelope and keeps the rows', () => {
    const out = sanitizeRegistryDdl(dump("INSERT INTO isahl_meta.meta_collections (table_name) VALUES ('zc_id_unit');"))
    expect(out).not.toContain('\\restrict')
    expect(out).not.toContain('\\unrestrict')
    expect(out).toContain("INSERT INTO isahl_meta.meta_collections (table_name) VALUES ('zc_id_unit');")
  })

  it('drops the dump session preamble — it would otherwise outlive the load on the pooled connection', () => {
    const out = sanitizeRegistryDdl(dump("INSERT INTO isahl_meta.meta_fields (fk_collection) VALUES ('zc_id_unit');"))
    expect(out).not.toContain('SET statement_timeout')
    expect(out).not.toContain('SET lock_timeout')
    expect(out).not.toContain('SET client_encoding')
    expect(out).not.toContain('SET client_min_messages')
    expect(out).not.toContain('SET row_security')
    expect(out).not.toContain("set_config('search_path'")
    expect(out).toContain("INSERT INTO isahl_meta.meta_fields (fk_collection) VALUES ('zc_id_unit');")
  })

  it('refuses a dump that carries anything but registry rows', () => {
    expect(() => sanitizeRegistryDdl(dump('CREATE TABLE isahl_meta.extra (a text);'), '/tmp/registry.sql'))
      .toThrow(/never has/)
    expect(() => sanitizeRegistryDdl(dump('CREATE TABLE isahl_meta.extra (a text);'), '/tmp/registry.sql'))
      .toThrow(/\/tmp\/registry\.sql/)
    expect(() => sanitizeRegistryDdl(dump("INSERT INTO isahl.zc_ad_object (code) VALUES ('x');")))
      .toThrow(/never has/)
    expect(() => sanitizeRegistryDdl(dump('DELETE FROM isahl_meta.meta_fields;')))
      .toThrow(/never has/)
  })

  it('leaves a non-dump file (this plugin\'s own baseline) alone apart from meta-commands', () => {
    const baseline = [
      '-- AppCreator standalone: isahl_meta schema seed',
      'SET check_function_bodies = false;',
      'CREATE TYPE isahl_meta.collection_type AS ENUM (',
      "    'table',",
      "    'view'",
      ');',
      '\\unrestrict not-a-real-command',
      '',
    ].join('\n')
    const out = sanitizeRegistryDdl(baseline, 'baseline.sql')
    expect(out).toContain('SET check_function_bodies = false;')
    expect(out).toContain("CREATE TYPE isahl_meta.collection_type AS ENUM (")
    expect(out).not.toContain('\\unrestrict not-a-real-command')
  })

  it('keeps a multi-line literal whose continuation line starts with a backslash', () => {
    const out = sanitizeRegistryDdl(dump(
      "INSERT INTO isahl_meta.meta_collections (biz_description) VALUES ('line one",
      '\\restrict not-a-command-this-is-data',
      "line two');",
    ))
    expect(out).toContain('\\restrict not-a-command-this-is-data')
    expect(out).not.toMatch(/^\\restrict AbC123$/m)
    expect(out).not.toMatch(/^\\unrestrict AbC123$/m)
  })

  it('keeps a literal line that looks like the dump preamble', () => {
    const out = sanitizeRegistryDdl(dump(
      "INSERT INTO isahl_meta.meta_fields (config) VALUES ('{\"note\": \"line one",
      'SET statement_timeout = 999;',
      'SELECT pg_catalog.set_config(\'search_path\', \'nope\', false);',
      "line two\"}');",
    ))
    expect(out).toContain('SET statement_timeout = 999;')
    expect(out).toContain("SELECT pg_catalog.set_config('search_path', 'nope', false);")
    expect(out).not.toContain('SET statement_timeout = 0;')
  })

  it('is not confused by an escaped quote inside a literal', () => {
    const out = sanitizeRegistryDdl(dump(
      "INSERT INTO isahl_meta.meta_fields (title) VALUES ('it''s",
      '\\kept-as-data\');',
    ))
    expect(out).toContain('\\kept-as-data')
  })

  it('refuses a COPY dump instead of loading zero rows', () => {
    const copy = [
      '--',
      '-- PostgreSQL database dump',
      '--',
      'COPY isahl_meta.meta_collections (table_name) FROM stdin;',
      'zc_id_unit',
      '\\.',
    ].join('\n')
    expect(() => sanitizeRegistryDdl(copy, '/tmp/registry.sql')).toThrow(/is a COPY dump/)
    expect(() => sanitizeRegistryDdl(copy, '/tmp/registry.sql')).toThrow(/--inserts/)
    expect(() => sanitizeRegistryDdl(copy, '/tmp/registry.sql')).toThrow(/\/tmp\/registry\.sql/)
  })

  it('leaves ordinary SQL untouched', () => {
    const sql = 'CREATE SCHEMA IF NOT EXISTS isahl_meta;\nCREATE TABLE isahl_meta.t (a text);\n'
    expect(sanitizeRegistryDdl(sql, 'x.sql')).toBe(sql)
  })

  it('passes the vendored baseline through byte-identical', async () => {
    const file = fileURLToPath(new URL('../vendor/backend/ddl/002_isahl_meta_schema.sql', import.meta.url))
    const baseline = await readFile(file, 'utf8')
    expect(sanitizeRegistryDdl(baseline, file)).toBe(baseline)
  })
})
