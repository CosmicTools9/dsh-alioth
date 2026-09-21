/**
 * Adapter tool-surface mapping. The model distribution's skill-adapters
 * reference a generic tool vocabulary (`read_file`, `write_file`,
 * `search_files`, …); the harness registers concrete tools (`read`, `write`,
 * `edit`, `glob`, `grep`, `bash`, `lsp`, `todo_write`, `web_search`,
 * `web_fetch`, `read_image`, `terminal_open/read/close`). The mapping is
 * declarative and verified against a deployment's registered set so a missing
 * surface fails loud at composition time, not mid-run.
 *
 * {@link ADAPTER_TOOL_VOCABULARY} is the full upstream vocabulary (contract §3);
 * every entry MUST have either a mapping here or a {@link MANUAL_ADAPTER_TOOLS}
 * entry with a reason — an uncovered tool is a defect, never a silent pass.
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
  // File trio + code authoring/reading.
  read_file: ['read'],
  write_file: ['write'],
  patch_file: ['edit'],
  list_dir: ['read'],
  search_files: ['glob', 'grep'],
  search_content: ['grep'],
  inspect_image: ['read_image'],
  // Command / code execution / diffing all ride the harness shell surface.
  run_command: ['bash'],
  run_code: ['bash'],
  file_diff: ['bash'],
  eval: ['bash'],
  http_get: ['web_fetch'],
  web_search: ['web_search'],
  // Code intelligence.
  code_intel: ['lsp'],
  // Progress tracking.
  todo: ['todo_write'],
  // Local service lifecycle (terminal-backed sessions in the harness).
  start_service: ['terminal_open'],
  stop_service: ['terminal_close'],
  service_logs: ['terminal_read'],
  // Deferred-blocker registry: the 本仓 equivalent lands in Wave 2 under this name.
  defer_blocker: ['alioth_deferred'],
}

/**
 * Adapter tools that have no harness tool equivalent — satisfied by a
 * documented manual or skill path, never by a tool call. Kept separate from
 * the mapping so a missing *mapping* still fails loud while a known human step
 * stays visible in the step payload instead of reading as a defect.
 *
 * 与上游差别：上游 AppAgent 自带这些工具（`tool_registry`），harness 侧只有通用工具面，
 * 故本仓显式降级为人工/技能路径，并要求调用点把这些条目透出给模型。
 */
export const MANUAL_ADAPTER_TOOLS: Readonly<Record<string, string>> = {
  visual_verify:
    'ego-browser 技能目视验证（harness 无视觉验证工具；上游该门禁要求写 visual-verify/report.json 且 /verdict == "PASS"）',
  debug_start: 'harness 无调试器工具面：人工用 bash 起调试器（或由部署侧提供 wrapper）后按 ADA 报告恢复',
  debug_eval: 'harness 无调试器工具面：调试会话表达式求值只能人工在调试器前端执行',
  debug_breakpoint: 'harness 无调试器工具面：断点设置需人工在调试器前端完成',
  debug_resume: 'harness 无调试器工具面：恢复执行需人工在调试器前端完成',
  debug_stack: 'harness 无调试器工具面：调用栈查看需人工在调试器前端完成',
  debug_stop: 'harness 无调试器工具面：终止调试会话需人工杀死进程（bash/hub stop）并记录结论',
  lint_file: 'harness 无独立 lint 工具：改由 bash 跑项目 linter，或交给该步的门禁程序（bun/npx script）判定',
  openspec_status: '本仓无 openspec 工作流工具：改读 openspec/ 目录或由人工核对变更状态',
}

/**
 * 适配器工具词汇表（契约 §3：上游 HEAD `skill-adapters/*.yaml` 实际出现的全部工具名，
 * 含各 adapter 的 `default_tools`）。完备性由测试断言：每个词必须有 harness 映射或
 * {@link MANUAL_ADAPTER_TOOLS} 人工条目——「未覆盖」不得静默通过。
 */
export const ADAPTER_TOOL_VOCABULARY: readonly string[] = [
  'read_file',
  'write_file',
  'patch_file',
  'file_diff',
  'lint_file',
  'search_files',
  'search_content',
  'list_dir',
  'run_command',
  'run_code',
  'code_intel',
  'todo',
  'visual_verify',
  'inspect_image',
  'http_get',
  'web_search',
  'start_service',
  'stop_service',
  'service_logs',
  'debug_start',
  'debug_eval',
  'debug_breakpoint',
  'debug_resume',
  'debug_stack',
  'debug_stop',
  'eval',
  'openspec_status',
]

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
