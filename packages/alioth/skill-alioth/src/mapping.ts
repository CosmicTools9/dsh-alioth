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
 * PROGRAMMATIC-FIRST RULE: `write_file` maps to NOTHING. Artifact content is
 * produced by programmatic generators/tools (alioth_app_write /
 * alioth_app_configure / alioth_entity_write); the LLM must never write
 * artifact files from text instructions. A step requiring write_file fails
 * loud (missing surface) instead of silently delegating content to the model.
 */
export const ADAPTER_TOOL_TO_DSH: Readonly<Record<string, readonly string[]>> = {
  read_file: ['read', 'tool:read'],
  // Intentionally unmapped — see module doc. Keeping the key makes the
  // missing-surface error explicit per step instead of an unknown tool.
  write_file: [],
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
