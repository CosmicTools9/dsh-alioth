/**
 * Alioth model snapshot resolution. The model travels as a checkout of the
 * open-source AppCreator repository: `backend/ddl/*isahl_meta*.sql` bootstraps
 * the entity registry, `skill-adapters/*.yaml` and
 * `Pre-Proc/Alioth/_schema/*.schema.json` are consumed downstream by the
 * orchestration and artifact-generation packages. GitHub snapshots are cached
 * per commit SHA under `<dataRoot>/models`; local checkouts are used in place.
 * @module @dsh-alioth/env-alioth/model-source
 */

import { execFile } from 'node:child_process'
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
 * - `local` — a filesystem path to an AppCreator checkout.
 */
export type ModelSpec
  = | { kind: 'github'; repo: string; ref: string }
    | { kind: 'local'; path: string }

/** Parse a model-source string: `github:owner/repo[@ref]` or a filesystem path. */
export function parseModelSource(spec: string): ModelSpec {
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
  /** `backend/ddl/*isahl_meta*.sql`, filename-sorted. Non-`isahl_meta` DDL files (AppCreator's own persistence) are excluded. */
  readonly ddlFiles: readonly string[]
  /** `skill-adapters/*.yaml`. */
  readonly skillAdapterFiles: readonly string[]
  /** `Pre-Proc/Alioth/_schema/*.schema.json`. */
  readonly artifactSchemaFiles: readonly string[]
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

export async function inspectModelArtifacts(root: string): Promise<ModelArtifacts> {
  const ddlFiles = await listDirMatching(path.join(root, 'backend', 'ddl'), '.sql', name => name.includes('isahl_meta'))
  if (ddlFiles.length === 0) {
    throw new Error(`env-alioth: no backend/ddl/*isahl_meta*.sql under ${root} — not an Alioth model snapshot`)
  }
  const skillAdapterFiles = await listDirMatching(path.join(root, 'skill-adapters'), '.yaml')
  const artifactSchemaFiles = await listDirMatching(path.join(root, 'Pre-Proc', 'Alioth', '_schema'), '.schema.json')
  return { ddlFiles, skillAdapterFiles, artifactSchemaFiles }
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
    return { dir, sourceRef: await gitHead(dir), modelVersion: await extractModelVersion(dir), artifacts }
  }
  const sha = await resolveGithubRef(spec.repo, spec.ref)
  const dir = path.join(cacheRoot, 'models', spec.repo.replace('/', '__'), sha)
  if (!await dirHasEntries(dir)) {
    await downloadGithubTarball(spec.repo, sha, dir)
  }
  const artifacts = await inspectModelArtifacts(dir)
  return { dir, sourceRef: sha, modelVersion: await extractModelVersion(dir), artifacts }
}
