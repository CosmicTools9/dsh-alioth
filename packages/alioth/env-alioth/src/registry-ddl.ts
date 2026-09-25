/**
 * Loading the registry SQL a model snapshot ships.
 *
 * The dedicated registry copy a model release is generated with is
 * `{model_dir}/isahl_meta-registry.sql`: a plain `pg_dump --data-only --inserts` of
 * `isahl_meta.meta_collections` + `meta_fields`, regenerated per model version, kept out of
 * git and out of the publication manifest — the consumption contract is "found ⇒ use,
 * missing ⇒ warn and continue". Two of its faces cannot be executed as-is over the ordinary
 * query interface the bootstrap uses (the publishing side deliberately leaves both to the
 * consumer):
 *
 * 1. PostgreSQL 18 opens the dump with `\restrict <token>` and closes it with
 *    `\unrestrict <token>`. psql handles those; the wire protocol rejects the line as a
 *    lexer error, so they are dropped here.
 * 2. The dump's session preamble (`SET …`, `SELECT pg_catalog.set_config('search_path','',false)`)
 *    is written for a one-shot psql restore. Executed over this plugin's POOLED connection it
 *    would outlive the load: `search_path` would stay empty for every later query in the
 *    session, and `statement_timeout`/`lock_timeout`/`row_security`/`client_min_messages`
 *    would stay at the dump's values. The preamble is therefore dropped too — the data
 *    statements are schema-qualified, and the dump's one real assumption
 *    (`standard_conforming_strings = on`) is PostgreSQL's default.
 *
 * Everything else in a recognised dump is refused: the sidecar's face is data rows for the two
 * registry tables and nothing else, so a dump that grows schema or session statements (or tries
 * to write outside `isahl_meta`) fails loudly instead of executing it against the registry.
 *
 * Only lines OUTSIDE string literals are inspected: `--inserts` writes values as single
 * quoted literals that may span lines (a description containing a newline), and a literal line
 * beginning with a backslash — or looking like a `SET` — is DATA, not a statement. Dropping it
 * would silently corrupt the row, so the scan tracks quote state. (`$$`-quoted bodies are not
 * tracked: `--inserts` output has none, and this plugin's own baseline files are not dumps,
 * so only meta-commands are stripped from them.)
 *
 * A `COPY … FROM stdin` dump is refused loudly: its data travels outside the statement text, so
 * executing it as SQL would load zero rows and look like a successful bootstrap.
 *
 * @module packages/alioth/env-alioth/src/registry-ddl
 */

/** A meta-command line: leading whitespace, then a backslash (never valid SQL). */
const META_COMMAND = /^\s*\\/
/** A COPY statement; only meaningful outside a literal, where the data block would be missing. */
const COPY_STATEMENT = /^\s*COPY\b[\s\S]*\bFROM\s+stdin\b/i
/** A dump's banner — what makes a file a dump this module may sanitize as data. */
const DUMP_BANNER = /^--\s*PostgreSQL database dump\b/
/** pg_dump's session-state lines: over a pooled connection they would outlive the load. */
const DUMP_SESSION_LINE = /^\s*(?:SET\s+[\w.]+\s*=|SELECT\s+pg_catalog\.set_config\s*\()/i
/** The only statements a registry sidecar may carry — rows for the two registry tables. */
const REGISTRY_ROW_INSERT = /^\s*INSERT\s+INTO\s+isahl_meta\.(?:meta_collections|meta_fields)\b/i
/** A comment-only or blank line. */
const NOISE_LINE = /^\s*(?:--.*)?$/

/**
 * Turn shipped registry SQL into statements that are safe to execute on this plugin's pooled
 * connection.
 * @param sql - the file's text.
 * @param source - the file path, for error messages.
 * @returns the executable SQL.
 * @throws when the text carries a `COPY … FROM stdin` statement, or — in a recognised dump —
 *   a statement that is neither a registry row nor dump preamble.
 */
export function sanitizeRegistryDdl(sql: string, source?: string): string {
  const where = source === undefined ? 'registry DDL' : `registry DDL ${source}`
  const lines = sql.split('\n')
  const isDump = lines.some(line => DUMP_BANNER.test(line))
  const kept: string[] = []
  let inString = false
  for (const line of lines) {
    if (!inString) {
      if (COPY_STATEMENT.test(line)) {
        throw new Error(
          `env-alioth: ${where} is a COPY dump — its rows travel outside the statement text, so `
          + 'executing it as SQL would load nothing. Generate it with `pg_dump --inserts` '
          + '(the model publish pipeline already does).',
        )
      }
      if (META_COMMAND.test(line)) {
        continue
      }
      if (isDump) {
        if (DUMP_SESSION_LINE.test(line)) {
          continue
        }
        if (!REGISTRY_ROW_INSERT.test(line) && !NOISE_LINE.test(line)) {
          throw new Error(
            `env-alioth: ${where} carries a statement a registry dump never has `
            + `(${line.trim().slice(0, 80)}) — a registry dump is \`pg_dump --data-only --inserts\` of `
            + 'isahl_meta.meta_collections/meta_fields, and anything else would run against the '
            + 'registry database; refusing it.',
          )
        }
      }
    }
    kept.push(line)
    // Track single-quote state: `''` is an escaped quote, so an odd run of quotes toggles.
    for (let i = 0; i < line.length; i++) {
      if (line[i] !== "'") {
        continue
      }
      inString = !inString
    }
  }
  return kept.join('\n')
}
