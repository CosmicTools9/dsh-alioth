/**
 * Alioth model snapshot resolution. dsh-alioth is a sibling consumer of the
 * Alioth model. The model's current channel is the Alioth model repository
 * (github:CosmicTools9/Alioth or a local checkout). The consumption-side
 * artifacts — the `backend/ddl/002_isahl_meta_schema.sql` structure baseline, the
 * `skill-adapters/*.yaml` definitions, and the prototype build scripts — are
 * vendored from the historical AppCreator distribution (frozen, Apache-2.0). The registry *rows*
 * are the model release's own sidecar (`isahl_meta-registry.sql`) — derived data, delivered with
 * the release and never committed — so a source either carries them or the registry boots
 * `missing`;
 * the package no longer ships a snapshot of its own (`builtin` retired 2026-09-24).
 * GitHub snapshots are cached per commit SHA under `<dataRoot>/models`.
 * @module @dsh-alioth/env-alioth/model-source
 */

import { execFile } from 'node:child_process'
import { existsSync } from 'node:fs'
import { mkdir, readFile, readdir, rename, rm } from 'node:fs/promises'
import path from 'node:path'
import { Readable } from 'node:stream'
import type { ReadableStream as NodeWebReadableStream } from 'node:stream/web'
import { pipeline } from 'node:stream/promises'
import { promisify } from 'node:util'
import { createGunzip } from 'node:zlib'
import { extract as extractTar } from 'tar-fs'

const execFileAsync = promisify(execFile)

/** Where model provenance lives inside a snapshot (see `alioth-gen/src/lib.rs`). */
const ALIOTH_GEN_LIB = path.join('backend', 'vendor', 'alioth-gen', 'src', 'lib.rs')

/** Where the Alioth model version literal is anchored in `lib.rs`. */
const MODEL_VERSION_RE = /ALIOTH_MODEL_VERSION[\s\S]{0,400}?unwrap_or_else\(\s*\|_\|\s*"([^"]+)"/

/**
 * A parsed model-source spec.
 * - `github` — `{repo}` is `owner/name`; `ref` is a branch, tag, or SHA.
 * - `local` — a filesystem path to a model distribution (or an assembled source that also carries
 *   `skill-adapters/`).
 */
export type ModelSpec
  = | { kind: 'github'; repo: string; ref: string }
    | { kind: 'local'; path: string }

/**
 * The configured model source, or a loud failure. There is no default: the package used to ship a
 * frozen snapshot (`builtin`) and that was retired on 2026-09-24 — a silent fallback to "whatever
 * the package has" is exactly the second copy that drifts from the model.
 * @param value - the source string, defaulting to `ALIOTH_MODEL_SOURCE`.
 */
export function requireModelSource(value: string | undefined = process.env.ALIOTH_MODEL_SOURCE): string {
  if (value === undefined || value.trim().length === 0) {
    throw new Error(
      'env-alioth: ALIOTH_MODEL_SOURCE is required — point it at a model distribution '
      + '(github:owner/repo[@ref]) or at an assembled source directory (release content plus '
      + 'skill-adapters/); the package no longer ships a frozen snapshot.',
    )
  }
  return value.trim()
}

/** Parse a model-source string: `github:owner/repo[@ref]` or a filesystem path. */
export function parseModelSource(spec: string): ModelSpec {
  if (spec === 'builtin') {
    // Said out loud: `builtin` would otherwise parse as a local path and fail far away, as a
    // missing directory.
    throw new Error(
      "env-alioth: model source 'builtin' was retired (2026-09-24) — point ALIOTH_MODEL_SOURCE at "
      + 'a model distribution (github:owner/repo[@ref]) or an assembled source directory.',
    )
  }
  if (spec.startsWith('github:')) {
    const rest = spec.slice('github:'.length)
    const at = rest.lastIndexOf('@')
    const repo = at === -1 ? rest : rest.slice(0, at)
    const ref = at === -1 ? 'main' : rest.slice(at + 1)
    if (!/^[^/\s]+\/[^/\s]+$/.test(repo)) {
      throw new Error(`env-alioth: invalid github model source ${JSON.stringify(spec)} (expected github:owner/repo[@ref])`)
    }
    if (ref.length === 0) {
      throw new Error(`env-alioth: empty ref in model source ${JSON.stringify(spec)}`)
    }
    return { kind: 'github', repo, ref }
  }
  if (spec.length === 0) {
    throw new Error('env-alioth: empty model source')
  }
  return { kind: 'local', path: spec }
}

/** The model artifacts this plugin consumes from a snapshot, all absolute paths. */
export interface ModelArtifacts {
  /** Registry DDL, filename-sorted: the vendored structure baseline first, then the snapshot's
   * registry *data* seeds (the publication's dedicated `*isahl_meta*.sql`), falling back to the
   * vendored seeds when the snapshot ships none. Non-`isahl_meta` DDL (the model's own physical
   * tables, dimension seeds, AppCreator's persistence) is excluded — the registry is this
   * plugin's schema, not the model's. */
  readonly ddlFiles: readonly string[]
  /** `skill-adapters/*.yaml`. */
  readonly skillAdapterFiles: readonly string[]
  /** `Pre-Proc/Alioth/_schema/*.schema.json`. */
  readonly artifactSchemaFiles: readonly string[]
  /** The publication's own version (`latest.json`), when the snapshot is a model release. */
  readonly publicationVersion?: string
  /**
   * Where the registry rows came from. `snapshot` — the model source ships them (the dedicated
   * `*isahl_meta*.sql` copy a model release is generated with); `missing` — it ships none, so the
   * registry boots unseeded and the path degrades to the metadata the model's own DDL carries
   * (a warning, never a boot failure).
   */
  readonly registrySource: 'snapshot' | 'missing'
}

/** Registry structure baseline: frozen AppCreator distribution, vendored inside this package. */
const VENDOR_DDL = path.resolve(new URL('../vendor/backend/ddl', import.meta.url).pathname)
/** The baseline's schema file — the one registry SQL a model release never has to ship. */
const SCHEMA_BASELINE_FILE = '002_isahl_meta_schema.sql'

/**
 * Resolve where a model snapshot's released content lives, accepting both publication layouts:
 * the flat fixed-path release (2026-09-21+: files at the repository root, version carried by
 * `latest.json` and the annotated tag) and the older `<root>/<version>/` directories. Same rule
 * as `scripts/generate-semantic-dicts.ts`; a snapshot with neither (the vendored AppCreator tree)
 * resolves to its own root.
 * @param root - snapshot root.
 */
async function resolveContentRoot(root: string): Promise<{ contentDir: string; version?: string }> {
  const release = await readFile(path.join(root, 'latest.json'), 'utf8')
    .then(text => JSON.parse(text) as { version?: unknown })
    .catch(() => null)
  const version = typeof release?.version === 'string' && release.version.length > 0 ? release.version : undefined
  if (version === undefined) {
    return { contentDir: root }
  }
  const versioned = path.join(root, version)
  return { contentDir: await dirHasEntries(versioned) ? versioned : root, version }
}

/**
 * The registry DDL to execute, filename-sorted, with the source it came from. The registry rows
 * come from the snapshot only: the official distribution ships them (the dedicated
 * `*isahl_meta*.sql` copy a model release is generated with), while a copy that ships none
 * degrades — `missing`, the
 * boot warns, and the registry stays empty, leaving the agent to work from the metadata the
 * model's own DDL carries. The frozen structure baseline is always included, so a snapshot that
 * ships rows but no schema still bootstraps.
 * Non-`isahl_meta` SQL (the model's physical tables, dimension seeds) is never executed: the
 * registry belongs to this plugin, not to the model.
 */
async function registryDdl(
  contentDir: string,
): Promise<{ files: string[]; source: ModelArtifacts['registrySource'] }> {
  const [nested, flat] = await Promise.all([
    listDirMatching(path.join(contentDir, 'backend', 'ddl'), '.sql', name => name.includes('isahl_meta')),
    listDirMatching(contentDir, '.sql', name => name.includes('isahl_meta')),
  ])
  const byName = new Map([...nested, ...flat].map(file => [path.basename(file), file]))
  const shipped = [...byName.keys()].some(name => name !== SCHEMA_BASELINE_FILE)
  byName.set(SCHEMA_BASELINE_FILE, byName.get(SCHEMA_BASELINE_FILE) ?? path.join(VENDOR_DDL, SCHEMA_BASELINE_FILE))
  const files = [...byName.entries()].sort(([a], [b]) => a.localeCompare(b)).map(([, file]) => file)
  const usable = files.filter(file => existsSync(file))
  if (usable.length === 0) {
    return { files: [], source: 'missing' }
  }
  return { files: usable, source: shipped ? 'snapshot' : 'missing' }
}

export async function inspectModelArtifacts(root: string): Promise<ModelArtifacts> {
  const { contentDir, version } = await resolveContentRoot(root)
  const { files: ddlFiles, source: registrySource } = await registryDdl(contentDir)
  const skillAdapterFiles = await listDirMatching(path.join(root, 'skill-adapters'), '.yaml')
  const artifactSchemaFiles = await listDirMatching(path.join(root, 'Pre-Proc', 'Alioth', '_schema'), '.schema.json')
  return {
    ddlFiles,
    skillAdapterFiles,
    artifactSchemaFiles,
    ...(version === undefined ? {} : { publicationVersion: version }),
    registrySource,
  }
}

/** List `dir` entries matching `suffix` (and `include` when given), filename-sorted to absolute paths. Missing dir → empty. */
function listDirMatching(dir: string, suffix: string, include?: (name: string) => boolean): Promise<string[]> {
  return readdir(dir)
    .then(names => names
      .filter(name => name.endsWith(suffix) && (include?.(name) ?? true))
      .sort()
      .map(name => path.join(dir, name)))
    .catch(() => [])
}

/**
 * Extract the Alioth model version from the vendored `alioth-gen` source.
 * The constant is env-driven at Rust runtime; its compiled-in default is the
 * honest version of the snapshot. Returns `'unknown'` when unreadable.
 */
export async function extractModelVersion(root: string): Promise<string> {
  try {
    const src = await readFile(path.join(root, ALIOTH_GEN_LIB), 'utf8')
    return MODEL_VERSION_RE.exec(src)?.[1] ?? 'unknown'
  } catch {
    return 'unknown'
  }
}

/** A resolved, ready-to-use model snapshot with provenance. */
export interface ModelSnapshot {
  /** Directory holding the snapshot (the checkout itself for local sources). */
  readonly dir: string
  /** Provenance ref: git SHA for github sources, `git rev-parse HEAD` for local checkouts, `'local'` fallback. */
  readonly sourceRef: string
  readonly modelVersion: string
  readonly artifacts: ModelArtifacts
}

async function gitHead(dir: string): Promise<string> {
  try {
    const { stdout } = await execFileAsync('git', ['-C', dir, 'rev-parse', 'HEAD'], { timeout: 5000 })
    const head = stdout.trim()
    return head.length > 0 ? head : 'local'
  } catch {
    return 'local'
  }
}

async function resolveGithubRef(repo: string, ref: string): Promise<string> {
  const url = `https://api.github.com/repos/${repo}/commits/${encodeURIComponent(ref)}`
  const res = await fetch(url, {
    headers: { accept: 'application/vnd.github+json' },
    signal: AbortSignal.timeout(20_000),
  })
  if (!res.ok) {
    throw new Error(`env-alioth: GitHub ref resolution failed (${res.status}) for ${repo}@${ref}`)
  }
  const body = (await res.json()) as { sha?: unknown }
  if (typeof body.sha !== 'string' || body.sha.length === 0) {
    throw new Error(`env-alioth: GitHub returned no SHA for ${repo}@${ref}`)
  }
  return body.sha
}

async function dirHasEntries(dir: string): Promise<boolean> {
  try {
    return (await readdir(dir)).length > 0
  } catch {
    return false
  }
}

async function downloadGithubTarball(repo: string, sha: string, dest: string): Promise<void> {
  const url = `https://codeload.github.com/${repo}/tar.gz/${sha}`
  const res = await fetch(url, { signal: AbortSignal.timeout(300_000) })
  if (!res.ok || res.body === null) {
    throw new Error(`env-alioth: tarball download failed (${res.status}) for ${repo}@${sha}`)
  }
  const staging = `${dest}.partial`
  await rm(staging, { recursive: true, force: true })
  await mkdir(path.dirname(staging), { recursive: true })
  // codeload tarballs carry one top-level `repo-sha/` directory; strip it so
  // the snapshot root is the repository root.
  await pipeline(
    Readable.fromWeb(res.body as unknown as NodeWebReadableStream<Uint8Array>),
    createGunzip(),
    extractTar(staging, { strip: 1 }),
  )
  await rm(dest, { recursive: true, force: true })
  await rename(staging, dest)
}

/**
 * Resolve a model spec into a snapshot, pulling and caching github tarballs as
 * needed. Local sources are validated in place — no copy is made.
 */
export async function resolveModelSnapshot(spec: ModelSpec, cacheRoot: string): Promise<ModelSnapshot> {
  if (spec.kind === 'local') {
    const dir = path.resolve(spec.path)
    const artifacts = await inspectModelArtifacts(dir)
    return {
      dir,
      sourceRef: await gitHead(dir),
      // A model release carries its version in `latest.json`; the vendored AppCreator tree carries
      // it in `alioth-gen/src/lib.rs`, which a release does not ship.
      modelVersion: artifacts.publicationVersion?.replace(/^v/, '') ?? await extractModelVersion(dir),
      artifacts,
    }
  }
  const sha = await resolveGithubRef(spec.repo, spec.ref)
  const dir = path.join(cacheRoot, 'models', spec.repo.replace('/', '__'), sha)
  if (!await dirHasEntries(dir)) {
    await downloadGithubTarball(spec.repo, sha, dir)
  }
  const artifacts = await inspectModelArtifacts(dir)
  return {
    dir,
    sourceRef: sha,
    modelVersion: artifacts.publicationVersion?.replace(/^v/, '') ?? await extractModelVersion(dir),
    artifacts,
  }
}
