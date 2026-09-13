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
  // Upstream vocabulary the adapters declare beyond the file trio (alioth-block
  // 1.3 / alioth-service 1.0-1.5 use run_command, alioth-block 1.1 + alioth-gui
  // use code_intel). Both have first-class harness equivalents; a deployment
  // that does not register them makes the step's declared surface unreachable,
  // which missingToolSurface reports rather than hiding.
  run_command: ['bash', 'tool:bash'],
  code_intel: ['lsp', 'tool:lsp'],
}

/**
 * Adapter tools that have no harness tool equivalent — satisfied by a
 * documented manual or skill path, never by a tool call. Kept separate from
 * the mapping so a missing *mapping* still fails loud while a known human step
 * stays visible in the step payload instead of reading as a defect.
 */
export const MANUAL_ADAPTER_TOOLS: Readonly<Record<string, string>> = {
  visual_verify: 'ego-browser 技能目视验证（upstream 有自己的 visual_verify 工具，harness 侧无等价物）',
}

export interface MissingTool {
  readonly adapterTool: string
  readonly required: readonly string[]
  readonly usedBy: readonly string[]
}

/** One adapter tool the deployment must satisfy by hand. */
export interface ManualTool {
  readonly adapterTool: string
  readonly reason: string
  readonly usedBy: readonly string[]
}

/** Steps grouped by the adapter tool they declare. */
function toolsByStep(adapter: Adapter): Map<string, string[]> {
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
  return used
}

/** Tools the adapter references that no registered harness tool satisfies. */
export function missingToolSurface(adapter: Adapter, registered: ReadonlySet<string>): readonly MissingTool[] {
  const missing: MissingTool[] = []
  for (const [tool, steps] of toolsByStep(adapter)) {
    if (MANUAL_ADAPTER_TOOLS[tool] !== undefined) continue
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

/** Adapter tools the deployment satisfies manually (no harness tool exists). */
export function manualToolSurface(adapter: Adapter): readonly ManualTool[] {
  const manual: ManualTool[] = []
  for (const [tool, steps] of toolsByStep(adapter)) {
    const reason = MANUAL_ADAPTER_TOOLS[tool]
    if (reason !== undefined) manual.push({ adapterTool: tool, reason, usedBy: steps })
  }
  return manual
}
