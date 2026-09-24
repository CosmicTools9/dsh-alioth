/**
 * What the console is allowed to show of an app: prototypes, and nothing else.
 *
 * Source (`Sources/**`, `modules/**`, `extensions/**`, `app.json`, traces) is a
 * paid, time-limited download — it must not be browsable or readable from the
 * browser. The harness's own file Remote cannot express that (its contract
 * allows absolute paths outside the workspace root, so it is disabled in the
 * bundle patch); this module is the one allowlist that the console's read
 * surfaces share:
 *
 * - `GET /preview/<rel>` — the byte server (HTML prototypes, their assets);
 * - `GET /api/alioth/prototypes` — the listing behind the 原型 tab.
 *
 * Two sanctioned layouts are visible (`docs`/`eval-report.ts` and the vendored
 * prototype tool agree on them):
 * - namespace tree `Pre-Proc/{ns}/Prototypes/**` — block/module shells and
 *   `_shared/` assets;
 * - app-level `Apps/{app}/prototype.html` (AppCreator layout) and, when an app
 *   keeps its own tree, `Apps/{app}/Prototypes/**`.
 * @module @dsh-alioth/auth-web-alioth/prototypes
 */

import { readdir, stat } from 'node:fs/promises'
import path from 'node:path'

/** Namespace-level prototype tree name (module-internal: the allowlist's own vocabulary). */
const PROTOTYPE_DIR = 'Prototypes'

/** The single-file prototype entry point of an AppCreator app. */
const PROTOTYPE_ENTRY = 'prototype.html'

/** One visible prototype artifact. */
export interface PrototypeEntry {
  /** Root-relative path — usable verbatim as `/preview/{rel}`. */
  readonly rel: string
  /** File name for display. */
  readonly name: string
  /** Which sanctioned tree it came from. */
  readonly group: 'app' | 'namespace'
  /** Path relative to its group root (display + sorting). */
  readonly label: string
  /** `html` entries open as prototypes; everything else is an asset. */
  readonly kind: 'html' | 'asset'
  readonly bytes: number
}

/** Where to look (the caller resolves the session's app first). */
export interface PrototypeScope {
  readonly preProcRoot: string
  readonly namespace: string
  readonly appCode: string
}

const MAX_ENTRIES = 400
const MAX_DEPTH = 6

/**
 * Whether a root-relative path may be read by the console. Deliberately a pure
 * shape test over an already-decoded, `/`-separated path: callers reject `..`
 * and empty segments before asking, and re-checking here keeps the rule usable
 * on its own.
 * @param rel - root-relative path under the Pre-Proc root.
 * @returns true when the path is a prototype artifact.
 */
export function isPrototypePath(rel: string): boolean {
  if (rel.includes('\0') || rel.includes('\\')) return false
  const segments = rel.split('/')
  if (segments.some(segment => segment === '' || segment === '.' || segment === '..')) return false
  if (segments[0] !== 'Pre-Proc' || segments.length < 3) return false
  const [, , kind, ...rest] = segments
  if (kind === PROTOTYPE_DIR) {
    // Any file under the namespace prototype tree (assets included); the bare
    // directory itself is not a file.
    return rest.length >= 1
  }
  if (kind !== 'Apps') return false
  const app = segments[3]
  if (app === undefined) return false
  const tail = segments.slice(4)
  // Apps/{app}/prototype.html — exactly the entry point, at that depth.
  if (tail.length === 1 && tail[0] === PROTOTYPE_ENTRY) return true
  // Apps/{app}/Prototypes/** — an app that keeps its own prototype tree.
  return tail.length >= 2 && tail[0] === PROTOTYPE_DIR
}

/** Every path this module hands out, as a `/preview/…` URL. */
export function prototypeUrl(rel: string): string {
  return `/preview/${rel.split('/').map(encodeURIComponent).join('/')}`
}

/** Recursively collect files under `root`, depth- and count-capped, symlink-free. */
async function walk(
  root: string,
  scopeRoot: string,
  group: PrototypeEntry['group'],
  out: PrototypeEntry[],
): Promise<void> {
  const step = async (dir: string, depth: number): Promise<void> => {
    if (depth > MAX_DEPTH || out.length >= MAX_ENTRIES) return
    const entries = await readdir(dir, { withFileTypes: true }).catch(() => null)
    if (entries === null) return
    for (const entry of entries.sort((left, right) => left.name.localeCompare(right.name))) {
      if (out.length >= MAX_ENTRIES) return
      const full = path.join(dir, entry.name)
      // Symlinks are skipped outright: a prototype tree must not become a window
      // onto the rest of the deployment through a link.
      if (entry.isDirectory()) {
        await step(full, depth + 1)
        continue
      }
      if (!entry.isFile()) continue
      const info = await stat(full).catch(() => null)
      if (info === null) continue
      const rel = path.relative(scopeRoot, full).split(path.sep).join('/')
      const label = path.relative(root, full).split(path.sep).join('/')
      out.push({
        rel,
        name: entry.name,
        group,
        label,
        kind: entry.name.endsWith('.html') ? 'html' : 'asset',
        bytes: info.size,
      })
    }
  }
  await step(root, 0)
}

/**
 * List the prototype artifacts visible for one app: the app's own entry point
 * and private tree first, then the namespace tree (shared shells + assets).
 * @param scope - resolved app scope.
 * @returns visible entries, app group first, each in lexical order; capped.
 */
export async function listPrototypes(scope: PrototypeScope): Promise<readonly PrototypeEntry[]> {
  // `preProcRoot` already carries the `Pre-Proc` segment (it is the deployment's
  // `ALIOTH_PRE_PROC_ROOT`), so entry paths are made relative to its PARENT —
  // the content root the `/preview/…` route resolves against.
  const contentRoot = path.dirname(scope.preProcRoot)
  const namespaceRoot = path.join(scope.preProcRoot, scope.namespace)
  const appRoot = path.join(namespaceRoot, 'Apps', scope.appCode)
  const relOf = (full: string): string => path.relative(contentRoot, full).split(path.sep).join('/')
  const out: PrototypeEntry[] = []

  const entry = path.join(appRoot, PROTOTYPE_ENTRY)
  const entryInfo = await stat(entry).catch(() => null)
  if (entryInfo?.isFile() === true) {
    out.push({
      rel: relOf(entry),
      name: PROTOTYPE_ENTRY,
      group: 'app',
      label: PROTOTYPE_ENTRY,
      kind: 'html',
      bytes: entryInfo.size,
    })
  }
  await walk(path.join(appRoot, PROTOTYPE_DIR), contentRoot, 'app', out)
  await walk(path.join(namespaceRoot, PROTOTYPE_DIR), contentRoot, 'namespace', out)
  // Deterministic and UX-ordered: the app's own prototypes, then the namespace
  // tree; entry pages before the assets they pull in, lexical inside each band.
  const band = (entry: PrototypeEntry): number => (entry.group === 'app' ? 0 : 2) + (entry.kind === 'html' ? 0 : 1)
  return [...out].sort((left, right) => band(left) - band(right) || left.rel.localeCompare(right.rel))
}
