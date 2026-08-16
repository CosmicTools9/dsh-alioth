/**
 * Dev-seed test-entity hygiene. The model snapshot's registry seeds carry
 * test entities whose table names end in `-testing` / `-test` (measured: 3 of
 * 807 in the current seed). This is a SOFT boundary: the suffix rule follows
 * the upstream seed's naming convention and must be extended when upstream
 * introduces differently-named test entities. Shared by schema-info queries
 * and the semantic-index term loader.
 * @module @dsh-alioth/tool-alioth-meta/hygiene
 */

/** SQL predicate excluding test entities; `alias` prefixes the table column (e.g. `mc.`). */
export function testEntityFilter(alias = ''): string {
  return `${alias}table_name NOT LIKE '%-testing' AND ${alias}table_name NOT LIKE '%-test'`
}
