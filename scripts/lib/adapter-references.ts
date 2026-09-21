/**
 * Adapter reference reachability: every non-script asset a vendored adapter
 * declares — `reference_paths` (adapter- and step-level) and per-step `inputs`
 * — must be resolvable inside the vendor tree.
 *
 * Why this gate exists: `unreachableGatePrograms` covers the *spawn* surface
 * (a missing script is an ENOENT the classifier calls path-missing, not
 * LLM-fixable). The reference surface is the same class of defect for declared
 * assets: an adapter that hands the model `docs/specs/MODULE_SPEC.md` or
 * `.agents/skills/alioth-service/references/inventory-voucher-contracts.md` as
 * an input while the sync set never carried that file ships a step whose
 * context is silently incomplete. Upstream asserts the same class textually in
 * `scripts/check/check-agent-skill-adapter-paths.ts` (gateway-shell reference
 * declared *and* present); this module generalises it to every declared
 * reference, with the YAML parsed by `parseAdapterDocument` rather than
 * pattern-matched.
 *
 * Resolution rule (a real segment walk, no path globbing library):
 *
 * - A path is split on `/`; `{ns}` / `{module}` / `{app}` / any `{...}` segment
 *   is a runtime coordinate. Everything at or below it is produced in the
 *   content root at run time (scaffolding, an earlier step, a plan-step
 *   artifact) — the vendored tree cannot be expected to hold it, so the walk
 *   stops there and the *static prefix* that was resolved so far must be a
 *   directory ("a directory that exists satisfies the glob").
 * - A path without a template segment names a fixed asset. Every segment must
 *   exist, directories for all but the last, and the whole reference must
 *   resolve to a file or a directory.
 * - `..` escapes the vendor tree, so it can never denote a carried asset;
 *   absolute paths are not vendor-relative and are skipped.
 * @module scripts/lib/adapter-references
 */

import { existsSync, readdirSync, readFileSync, statSync } from 'node:fs'
import { join } from 'node:path'
import { parseAdapterDocument } from '@dsh-alioth/skill-alioth'

/** One declared adapter reference the vendor tree cannot resolve. */
export interface UnreachableAdapterReference {
  /** Adapter file the reference is declared in. */
  readonly adapter: string
  /** `track.step` position, or `adapter` for an adapter-level declaration. */
  readonly step: string
  /** The declared reference string, verbatim. */
  readonly reference: string
}

/** Step label for references declared outside a step (adapter-level). */
const ADAPTER_SCOPE = 'adapter'

/** Whether a path segment is a `{ns}` / `{module}` / `{app}` style template. */
function isTemplateSegment(segment: string): boolean {
  return segment.length > 2 && segment.startsWith('{') && segment.endsWith('}')
}

/**
 * Resolve one declared reference against `vendorRoot`.
 * @returns Whether the reference is reachable (see the module resolution rule).
 */
function isReachable(reference: string, vendorRoot: string): boolean {
  if (reference.startsWith('/')) return true
  let current = vendorRoot
  const segments = reference.split('/').filter(segment => segment !== '' && segment !== '.')
  for (const [index, segment] of segments.entries()) {
    if (isTemplateSegment(segment)) {
      // Runtime coordinate: the static prefix walked so far exists by
      // construction, and the remainder is produced in the content root.
      return true
    }
    if (segment === '..') return false
    const next = join(current, segment)
    const last = index === segments.length - 1
    if (!existsSync(next)) return false
    if (last) return true
    if (!statSync(next).isDirectory()) return false
    current = next
  }
  return existsSync(current)
}

/** Vendored adapter files (underscore-prefixed files are runtime config, not adapters). */
function adapterFiles(adaptersDir: string): string[] {
  return readdirSync(adaptersDir)
    .filter(name => name.endsWith('.yaml') && !name.startsWith('_'))
    .sort()
}

/**
 * List declared adapter references the vendor tree cannot resolve.
 * @param adaptersDir - Directory holding the vendored `*.yaml` adapters.
 * @param vendorRoot - Root the declared references resolve against.
 * @returns One entry per unreachable reference — adapter order, then
 *   adapter-level declarations before track/step declarations.
 */
export function unreachableAdapterReferences(
  adaptersDir: string, vendorRoot: string,
): UnreachableAdapterReference[] {
  const missing: UnreachableAdapterReference[] = []
  const seen = new Set<string>()
  for (const entry of adapterFiles(adaptersDir)) {
    const adapter = parseAdapterDocument(readFileSync(join(adaptersDir, entry), 'utf8'), entry)
    const declared: { step: string; reference: string }[] = adapter.referencePaths
      .map(reference => ({ step: ADAPTER_SCOPE, reference }))
    for (const track of adapter.tracks) {
      for (const step of track.steps) {
        const scope = `${track.name}.${step.id}`
        for (const reference of step.referencePaths) declared.push({ step: scope, reference })
        for (const reference of step.inputs) declared.push({ step: scope, reference })
      }
    }
    for (const item of declared) {
      const key = `${entry}\u0000${item.step}\u0000${item.reference}`
      if (seen.has(key)) continue
      seen.add(key)
      if (!isReachable(item.reference, vendorRoot)) {
        missing.push({ adapter: entry, step: item.step, reference: item.reference })
      }
    }
  }
  return missing
}
