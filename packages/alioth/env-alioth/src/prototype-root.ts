/**
 * Prototype content-root provisioning. The vendored prototype chain
 * (`scripts/prototype-tool.js` + `scripts/check/*`) expects the upstream
 * repo-root layout: `Pre-Proc/` (artifact tree), `.agents/skills/alioth-design/
 * references/` (design tokens, shells, icon pool) and `Framework/frontend/
 * components/utilities.json` (utility registry) all under one root, with the
 * tool invoked relative to that root. Deployments point PROTOTYPE_TOOL_ROOT at
 * this content root (default: the parent of the Pre-Proc root) and the
 * provisioner materializes the vendored pieces there — idempotent, never
 * overwriting existing files.
 * @module @dsh-alioth/env-alioth/prototype-root
 */

import { cpSync, existsSync, lstatSync, mkdirSync, rmSync, symlinkSync, statSync, unlinkSync } from 'node:fs'
import path from 'node:path'
import { fileURLToPath } from 'node:url'

/** Vendored assets shipped inside the env-alioth package. */
export const VENDOR_ROOT = path.resolve(path.dirname(fileURLToPath(import.meta.url)), '..', 'vendor')

export interface PrototypeRootInfo {
  /** Content root (PROTOTYPE_TOOL_ROOT): parent of the Pre-Proc root. */
  readonly contentRoot: string
  /** The Pre-Proc artifact root as given. */
  readonly preProcRoot: string
  /** True when `contentRoot/Pre-Proc` is a symlink created by the provisioner. */
  readonly preProcLinked: boolean
}

function isDir(p: string): boolean {
  try {
    return statSync(p).isDirectory()
  } catch {
    return false
  }
}

/**
 * Materialize the vendored repo-root layout under `contentRoot`:
 * - `.agents/`, `Framework/`, `scripts/` copied from vendor when missing
 *   (never overwritten — local edits win);
 * - `Pre-Proc` symlinked to `preProcRoot` when the two differ (a pre-existing
 *   real directory is kept untouched — native layout).
 * Idempotent: safe to call on every gate run.
 */
export function provisionPrototypeRoot(preProcRoot: string, contentRoot = path.dirname(path.resolve(preProcRoot))): PrototypeRootInfo {
  const resolvedContent = path.resolve(contentRoot)
  const resolvedPreProc = path.resolve(preProcRoot)
  mkdirSync(resolvedContent, { recursive: true })

  for (const dir of ['.agents', 'Framework', 'scripts']) {
    const target = path.join(resolvedContent, dir)
    const source = path.join(VENDOR_ROOT, dir)
    if (!existsSync(target) && existsSync(source)) {
      mkdirSync(path.dirname(target), { recursive: true })
      cpSync(source, target, { recursive: true })
    }
  }

  let preProcLinked = false
  const preProcLink = path.join(resolvedContent, 'Pre-Proc')
  if (path.resolve(preProcRoot) !== preProcLink) {
    let state: 'dir' | 'symlink' | 'missing' = 'missing'
    try {
      state = lstatSync(preProcLink).isSymbolicLink() ? 'symlink' : isDir(preProcLink) ? 'dir' : 'missing'
    } catch {
      state = 'missing'
    }
    if (state === 'symlink') {
      preProcLinked = true
    } else if (state === 'missing') {
      mkdirSync(resolvedPreProc, { recursive: true })
      symlinkSync(resolvedPreProc, preProcLink, 'dir')
      preProcLinked = true
    }
  }

  return { contentRoot: resolvedContent, preProcRoot: resolvedPreProc, preProcLinked }
}

/** Remove a provisioned content root's copied assets (tests). */
export function removeProvisionedAssets(contentRoot: string): void {
  for (const dir of ['.agents', 'Framework', 'scripts']) {
    rmSync(path.join(contentRoot, dir), { recursive: true, force: true })
  }
  const preProcLink = path.join(contentRoot, 'Pre-Proc')
  try {
    if (lstatSync(preProcLink).isSymbolicLink()) {
      unlinkSync(preProcLink)
    }
  } catch {
    // not present / not a link
  }
}
