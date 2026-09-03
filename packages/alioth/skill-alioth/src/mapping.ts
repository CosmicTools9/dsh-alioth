/**
 * Adapter tool-surface mapping. The model distribution's skill-adapters
 * reference a generic tool vocabulary (`read_file`, `write_file`,
 * `search_files`); the harness registers concrete tools (`read`/`tool:read`,
 * `write`/`tool:write`, `glob`, `grep`). The mapping is declarative and
 * verified against a deployment's registered set so a missing surface fails
 * loud at composition time, not mid-run.
 * @module @dsh-alioth/skill-alioth/mapping
 */

import type { Adapter } from './adapter.ts'

/**
 * Adapter tool name → accepted harness tool names (any one satisfies).
 *
 * PROGRAMMATIC-FIRST RULE (amended 2026-09-03, full-stack surface): contract
 * artifacts (app.json/module.json/extensions/entity rows) are produced ONLY
 * by programmatic generators/tools (alioth_app_write / alioth_app_configure /
 * alioth_entity_write). CODE files under `Sources/` and `Prototypes/` are the
 * exception: they are authored by the model with the harness `write` tool
 * inside whitelisted workflow steps and accepted only by programmatic gates
 * (bun prototype build, nav check, cargo check). `write_file` therefore maps
 * to the harness write surface; gate failures — not mapping absence — reject
 * bad code.
 */
export const ADAPTER_TOOL_TO_DSH: Readonly<Record<string, readonly string[]>> = {
  read_file: ['read', 'tool:read'],
  write_file: ['write', 'tool:write'],
  search_files: ['glob', 'tool:glob', 'grep', 'tool:grep'],
}

export interface MissingTool {
  readonly adapterTool: string
  readonly required: readonly string[]
  readonly usedBy: readonly string[]
}

/** Tools the adapter references that no registered harness tool satisfies. */
export function missingToolSurface(adapter: Adapter, registered: ReadonlySet<string>): readonly MissingTool[] {
  const used = new Map<string, string[]>()
  for (const track of adapter.tracks) {
    for (const step of track.steps) {
      for (const tool of step.tools) {
        const list = used.get(tool)
        if (list === undefined) {
          used.set(tool, [step.id])
        } else {
          list.push(step.id)
        }
      }
    }
  }
  const missing: MissingTool[] = []
  for (const [tool, steps] of used) {
    const required = ADAPTER_TOOL_TO_DSH[tool]
    if (required === undefined) {
      missing.push({ adapterTool: tool, required: [], usedBy: steps })
      continue
    }
    if (!required.some(name => registered.has(name))) {
      missing.push({ adapterTool: tool, required, usedBy: steps })
    }
  }
  return missing
}
