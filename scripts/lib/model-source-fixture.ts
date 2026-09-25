/**
 * Assemble a **model source** directory: the layout `ALIOTH_MODEL_SOURCE` must point at once
 * `builtin` (the package's frozen snapshot) is retired.
 *
 * A source is the union of two halves that today live apart:
 * - the **release content** (`latest.json` version anchor + the registry rows as
 *   `*isahl_meta*.sql`, plus the model's own DDL/seeds) — derived data, delivered out of Git;
 * - the **kit** the plugin consumes but a model release does not ship: `skill-adapters/*.yaml`
 *   and, when present, `Pre-Proc/Alioth/_schema/*.schema.json`.
 *
 * Used by three callers, one implementation:
 * - `scripts/assemble-model-source.ts` — a deployment assembles the real source (channel + kit);
 * - `scripts/smoke-composition.ts` / `tests/**` — a self-contained fixture (tiny seed + kit),
 *   so the suites stay keyless and network-free;
 * - the container self-check, same fixture idea.
 *
 * @module scripts/lib/model-source-fixture
 */

import { cp, mkdir, readFile, readdir, rm, writeFile } from 'node:fs/promises'
import path from 'node:path'
import { fileURLToPath } from 'node:url'
import { sanitizeRegistryDdl } from '@dsh-alioth/env-alioth'

/** The kit vendored inside this package: adapters + artifact schemas a release does not carry. */
const VENDORED_KIT_DIR = path.resolve(
  fileURLToPath(new URL('../../packages/alioth/env-alioth/vendor', import.meta.url)),
)

/** Release files a source may carry; only `latest.json` and the registry rows are consumed. */
const RELEASE_FILES = ['latest.json', '001_schema.sql', '002_isahl_tables.sql', 'seed-dimensions.sql']

/**
 * A tiny registry seed in the release's positional `--inserts` shape (the vendored structure
 * baseline provides the schema), enough for a registry-backed tool call to round-trip.
 */
export const FIXTURE_REGISTRY_SEED = `
INSERT INTO isahl_meta.meta_collections VALUES
  ('fixture_collection', now(), now(), 1, 1, 'Fixture collection', 'table', '{}'::jsonb, NULL, 'isahl', 'assembled fixture row');
INSERT INTO isahl_meta.meta_fields VALUES
  ('fixture_collection', 'name', now(), now(), 1, 1, 'scalar', 'text', true, NULL, NULL, 'Name'),
  ('fixture_collection', 'qty', now(), now(), 1, 1, 'scalar', 'integer', false, NULL, NULL, 'Quantity');
`

export interface ModelSourceOverlay {
  /** Release content to copy in (a resolved channel snapshot). */
  readonly releaseDir?: string
  /** Kit to copy in; defaults to this package's vendored kit. Pass `null` to skip. */
  readonly kitDir?: string | null
  /** A registry seed written to `backend/ddl/003_isahl_meta_seed.sql` (fixtures). */
  readonly seedSql?: string
}

/** Copy the kit's consumed halves (adapters, artifact schemas) into `outDir`. */
async function copyKit(outDir: string, kitDir: string): Promise<number> {
  let copied = 0
  const adapters = path.join(kitDir, 'skill-adapters')
  if (await readdir(adapters).then(names => names.length > 0, () => false)) {
    await cp(adapters, path.join(outDir, 'skill-adapters'), { recursive: true })
    copied += (await readdir(path.join(outDir, 'skill-adapters'))).length
  }
  const schemas = path.join(kitDir, 'Pre-Proc', 'Alioth', '_schema')
  if (await readdir(schemas).then(names => names.length > 0, () => false)) {
    await cp(schemas, path.join(outDir, 'Pre-Proc', 'Alioth', '_schema'), { recursive: true })
  }
  return copied
}

/**
 * Assemble a model source at `outDir`.
 * @param outDir - target directory (created when absent).
 * @param overlay - release content, kit and/or a fixture seed.
 * @returns the adapter file count copied in, for callers that assert a complete source.
 */
export async function assembleModelSource(outDir: string, overlay: ModelSourceOverlay): Promise<{ adapters: number }> {
  await mkdir(outDir, { recursive: true })

  if (overlay.releaseDir !== undefined) {
    const entries = await readdir(overlay.releaseDir)
    // Both publication layouts: files at the release root, which is what a resolved snapshot dir is.
    const wanted = entries.filter(name => RELEASE_FILES.includes(name))
    for (const name of wanted) {
      await cp(path.join(overlay.releaseDir, name), path.join(outDir, name))
    }
    // The registry rows: derived data, carried by name rather than assumed, and placed under
    // `backend/ddl/` — the layout both code generations read (the flat release root is also
    // accepted, but an older deployment scans only `backend/ddl/`, and one file beats two copies).
    // Stale copies from an earlier layout are dropped so the source is deterministic.
    await rm(path.join(outDir, 'isahl_meta-registry.sql'), { force: true })
    await mkdir(path.join(outDir, 'backend', 'ddl'), { recursive: true })
    // The structure baseline travels with the source: same directory as the rows, so the file name
    // order puts the schema first in every code generation (an older build does not prepend the
    // package's copy, and sorts by full path).
    await cp(
      path.join(VENDORED_KIT_DIR, 'backend', 'ddl', '002_isahl_meta_schema.sql'),
      path.join(outDir, 'backend', 'ddl', '002_isahl_meta_schema.sql'),
    )
    for (const name of entries.filter(name => name.includes('isahl_meta') && name.endsWith('.sql'))) {
      const text = await readFile(path.join(overlay.releaseDir, name), 'utf8')
      // `pg_dump` 18 wraps its output in psql-only meta-commands (`\restrict` / `\unrestrict`).
      // They are stripped here as well as at execution time: a deployment may run an older plugin
      // build that executes the file verbatim, and a delivered source should not depend on that.
      await writeFile(path.join(outDir, 'backend', 'ddl', name), sanitizeRegistryDdl(text, name))
    }
  }

  if (overlay.seedSql !== undefined) {
    await mkdir(path.join(outDir, 'backend', 'ddl'), { recursive: true })
    await writeFile(path.join(outDir, 'backend', 'ddl', '003_isahl_meta_seed.sql'), overlay.seedSql)
    // A fixture still needs a version anchor: the resolver reads `latest.json` for any source, and
    // a source without one reports no version at all.
    await writeFile(
      path.join(outDir, 'latest.json'),
      JSON.stringify({ version: 'v0.0.0-fixture', published_at: new Date(0).toISOString() }, null, 2) + '\n',
    )
  }

  const kitDir = overlay.kitDir === undefined ? VENDORED_KIT_DIR : overlay.kitDir
  const adapters = kitDir === null ? 0 : await copyKit(outDir, kitDir)
  return { adapters }
}
