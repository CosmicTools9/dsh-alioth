/**
 * Adapter gate reachability: every script a vendored adapter gate invokes must
 * exist inside the vendor tree. A missing program spawns ENOENT, which the
 * gate classifier reports as `path-missing` — not LLM-fixable — so the track
 * stalls instead of recovering. Checked at framework-sync time (the sync set
 * must carry the script and its data) and by the vendor-reachability test.
 * @module skill-alioth/gate-programs
 */

import { existsSync, readdirSync, readFileSync } from 'node:fs'
import { extname, join } from 'node:path'
import { parseAdapterDocument } from './adapter.ts'

/** Gate arguments with these extensions are repo-relative script paths. */
const SCRIPT_PATH_EXTENSION: Record<string, true> = {
  '.ts': true, '.mts': true, '.js': true, '.mjs': true, '.cjs': true, '.sh': true,
}

/**
 * Gate programs the runtime always permits — the code-truth floor upstream
 * keeps in `tool_registry`. The vendored `_runtime.yaml` mirrors it and may
 * append operator entries (codegraph/git/node); a runtime mirror that does not
 * cover this list is a governance defect, asserted by test.
 */
export const GATE_PROGRAM_WHITELIST: readonly string[] = [
  'target/debug/ontology-mapping',
  'bun',
  'npx',
  'cargo',
  'bash',
]

/**
 * Whether a gate program is permitted: an exact entry, or a path inside an
 * entry's directory (upstream matches `program == entry || program.startsWith(entry + '/')`).
 * @param program - Program the gate declared.
 * @param allowed - Permitted entries; empty means the caller opted out.
 */
export function isAllowedGateProgram(program: string, allowed: readonly string[]): boolean {
  return allowed.some(entry => program === entry || program.startsWith(`${entry}/`))
}

/** One adapter gate script that the vendor tree cannot spawn. */
export interface UnreachableGateProgram {
  /** Adapter file the gate belongs to. */
  readonly adapter: string
  /** `track.step` position carrying the gate. */
  readonly step: string
  /** Repo-relative script path the gate invokes. */
  readonly script: string
}

/**
 * List adapter gate scripts missing from the vendor tree.
 * @param adaptersDir - Directory holding the vendored `*.yaml` adapters.
 * @param vendorRoot - Root the adapter-relative script paths resolve against.
 * @returns One entry per unreachable script, adapter order.
 */
export function unreachableGatePrograms(
  adaptersDir: string, vendorRoot: string,
): UnreachableGateProgram[] {
  const missing: UnreachableGateProgram[] = []
  const seen = new Set<string>()
  const files = readdirSync(adaptersDir)
    .filter(name => name.endsWith('.yaml') && !name.startsWith('_'))
    .sort()
  for (const entry of files) {
    const adapter = parseAdapterDocument(readFileSync(join(adaptersDir, entry), 'utf8'), entry)
    for (const track of adapter.tracks) {
      for (const step of track.steps) {
        for (const gate of step.gates) {
          if (gate.kind !== 'program') continue
          for (const arg of gate.args) {
            if (SCRIPT_PATH_EXTENSION[extname(arg)] === undefined) continue
            const key = `${entry}\u0000${arg}`
            if (seen.has(key)) continue
            seen.add(key)
            if (!existsSync(join(vendorRoot, arg))) {
              missing.push({ adapter: entry, step: `${track.name}.${step.id}`, script: arg })
            }
          }
        }
      }
    }
  }
  return missing
}
