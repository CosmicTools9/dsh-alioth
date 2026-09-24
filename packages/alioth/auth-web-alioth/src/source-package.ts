/**
 * The paid source package: what a download actually delivers.
 *
 * Deliberately a DIFFERENT allowlist from `prototypes.ts` — the console may show
 * prototypes to everyone, while the package below is exactly what a subscription
 * unlocks. The two must never be merged: one is the free surface, the other is the
 * product.
 *
 * Included (per app, in this archive order): `app.json`, `prototype.html`, then the
 * `modules/`, `extensions/` and `Sources/` trees — lexical inside each. Excluded: everything else, plus build output and
 * VCS noise inside the source tree (`target/`, `node_modules/`, `dist/`, `.git/`) —
 * a delivered package carries source, not artifacts of a previous build.
 * @module @dsh-alioth/auth-web-alioth/source-package
 */

import { readdir, readFile, stat } from 'node:fs/promises'
import path from 'node:path'
import type { ZipEntry } from './zip-store.ts'

/** Top-level files of an app that the package always carries. */
const ROOT_FILES = ['app.json', 'prototype.html'] as const

/** Directories walked recursively. */
const TREE_ROOTS = ['modules', 'extensions', 'Sources'] as const

/** Directory names pruned wherever they appear inside the tree. */
const PRUNED: Readonly<Record<string, true>> = {
  node_modules: true,
  '.git': true,
  target: true,
  dist: true,
  '.DS_Store': true,
}

/** Default cap: a delivered package is source + contract files, not a dump. */
const SOURCE_PACKAGE_MAX_BYTES = 64 * 1024 * 1024

/** One collected package. */
export interface SourcePackage {
  readonly entries: readonly ZipEntry[]
  /** Total uncompressed bytes. */
  readonly bytes: number
}

/** Refusal to build a package (over the cap). Thrown, never silently truncated. */
export class SourcePackageTooLargeError extends Error {
  /** Collected size when the cap tripped. */
  readonly bytes: number
  /** The cap that tripped. */
  readonly limit: number

  constructor(bytes: number, limit: number) {
    super(`source package is ${bytes} bytes, over the ${limit} byte cap`)
    this.name = 'SourcePackageTooLargeError'
    this.bytes = bytes
    this.limit = limit
  }
}

/**
 * Whether an app-relative path belongs in the paid package. Pure shape test; the
 * walker is what turns it into a listing.
 * @param rel - path relative to the app directory, `/`-separated.
 * @returns true when the path is delivered source.
 */
export function isSourcePackagePath(rel: string): boolean {
  if (rel === '' || rel.includes('\0') || rel.includes('\\')) return false
  const segments = rel.split('/')
  if (segments.some(segment => segment === '' || segment === '.' || segment === '..')) return false
  if (segments.length === 1) return (ROOT_FILES as readonly string[]).includes(segments[0] ?? '')
  return (TREE_ROOTS as readonly string[]).includes(segments[0] ?? '')
}

/**
 * Collect the package for one app.
 *
 * Symlinks are skipped outright (a delivered tree must not smuggle the deployment
 * in), ordering is lexical so the same app always yields the same archive, and the
 * size cap fails loud instead of handing over a truncated package.
 * @param appDir - absolute app directory (`{preProcRoot}/{ns}/Apps/{app}`).
 * @param appCode - app code, used as the archive's root folder.
 * @param limit - byte cap; defaults to {@link SOURCE_PACKAGE_MAX_BYTES}.
 * @returns the collected entries and their total size.
 */
export async function collectSourcePackage(
  appDir: string,
  appCode: string,
  limit: number = SOURCE_PACKAGE_MAX_BYTES,
): Promise<SourcePackage> {
  const entries: ZipEntry[] = []
  let bytes = 0

  const add = async (absolute: string, rel: string, info: { size: number; mtimeMs: number }): Promise<void> => {
    bytes += info.size
    if (bytes > limit) throw new SourcePackageTooLargeError(bytes, limit)
    // The archive carries a per-app root folder so extracting never scatters
    // files of two apps into one directory.
    entries.push({
      name: `${appCode}/${rel}`,
      data: await readFile(absolute),
      mtime: new Date(info.mtimeMs),
    })
  }

  for (const file of ROOT_FILES) {
    const absolute = path.join(appDir, file)
    const info = await stat(absolute).catch(() => null)
    if (info?.isFile() === true) await add(absolute, file, info)
  }

  const walk = async (dir: string, relDir: string, depth: number): Promise<void> => {
    if (depth > 12) return
    const children = await readdir(dir, { withFileTypes: true }).catch(() => null)
    if (children === null) return
    for (const child of children.sort((left, right) => left.name.localeCompare(right.name))) {
      if (PRUNED[child.name] === true) continue
      const absolute = path.join(dir, child.name)
      const rel = relDir === '' ? child.name : `${relDir}/${child.name}`
      if (child.isDirectory()) {
        await walk(absolute, rel, depth + 1)
        continue
      }
      if (!child.isFile()) continue
      const info = await stat(absolute).catch(() => null)
      if (info === null) continue
      await add(absolute, rel, info)
    }
  }

  for (const tree of TREE_ROOTS) {
    await walk(path.join(appDir, tree), tree, 0)
  }

  return { entries, bytes }
}
