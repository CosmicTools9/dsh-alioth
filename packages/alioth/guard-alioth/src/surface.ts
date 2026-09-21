/**
 * 执行层工具面判定：声明工具面（§3 工具面强制）、namespace 写沙箱、plan 写面收窄。
 *
 * 三者的判据都取自**已解析的 adapter 步骤**、部署配置与调用参数，纯函数、无 IO——
 * 调用点负责用 `scope` 模块现取当前步骤。判定结果一律带 §3 指定的规则码前缀
 * （见 `rules` 模块）。
 *
 * 路径口径：写面 = `Pre-Proc/{ns}/{Sources,Prototypes,Apps,AppAgentTraces}/`
 * （上游 `tool_registry/mod.rs:391-460`），显式拒绝 `.` / `..` 组件穿越，并以
 * `autonomy.yaml` 为写黑名单。绝对路径按部署的 `preProcRoot` 判定（不依赖目录名恰好
 * 叫 `Pre-Proc`），相对路径按 adapter 的 `Pre-Proc/{ns}/...` 口径判定。
 * @module @dsh-alioth/guard-alioth/surface
 */

import path from 'node:path'
import { ADAPTER_TOOL_TO_DSH, type Adapter, type Step } from '@dsh-alioth/skill-alioth'
import { GUARD_RULE_IDS, guardDenyReason } from './rules.ts'

/** 判定结果：`ok` 为真即放行；否则 `reason` 是带规则码前缀的单行拒绝文本。 */
export type Adjudication =
  | { readonly ok: true }
  | { readonly ok: false; readonly reason: string }

/** 本组元工具（推进/验证/编排面）：任何 `alioth_` 前缀工具都不受步骤工具面收窄。 */
export const META_TOOL_PREFIX = 'alioth_'

/**
 * 写类工具 → 其路径参数名。`read` 不需要（只读），`bash` 不做路径推断（§3）——
 * 无法静态判定写面的调用不在此表内，也就不会被沙箱误判。
 */
export const WRITE_TOOL_PATH_ARG: Readonly<Record<string, string>> = {
  write: 'file_path',
  edit: 'file_path',
  str_replace_editor: 'path',
}

/** 沙箱允许的写区（namespace 目录下第一层）。 */
export const SANDBOX_ZONES: Readonly<Record<string, true>> = {
  Sources: true,
  Prototypes: true,
  Apps: true,
  AppAgentTraces: true,
}

/** 写黑名单文件名：授权声明只由人工维护，模型不得改写。 */
export const SANDBOX_WRITE_BLACKLIST: Readonly<Record<string, true>> = {
  'autonomy.yaml': true,
}

/** adapter 口径的沙箱根段（相对形式的首段）。 */
const PRE_PROC_SEGMENT = 'Pre-Proc'

/**
 * 当前步骤声明的 harness 工具面 = `步骤 tools ∪ adapter default_tools` 的映射并集
 * （与 workflow 插件 `alioth_workflow_step` 报给模型的 `harnessTools` 同口径）。
 * 无当前步骤 → 空（调用点据此跳过判定，见 `scope`）。
 */
export function declaredToolSurface(adapter: Adapter, step: Step | undefined): readonly string[] {
  const harness = new Set<string>()
  for (const tool of [...step?.tools ?? [], ...adapter.defaultTools]) {
    for (const name of ADAPTER_TOOL_TO_DSH[tool] ?? []) {
      harness.add(name)
    }
  }
  return [...harness].sort()
}

/** 写类调用的路径参数（非写类工具或参数缺失 → `undefined`）。 */
export function writePathOf(tool: string, args: unknown): string | undefined {
  const argName = WRITE_TOOL_PATH_ARG[tool]
  if (argName === undefined || typeof args !== 'object' || args === null) {
    return undefined
  }
  const value = (args as Record<string, unknown>)[argName]
  return typeof value === 'string' ? value : undefined
}

/**
 * 工具面强制：调用名必须在声明的 harness 工具面内，元工具（`alioth_*`）不受限。
 * @param tool - 调用名。
 * @param allowed - 本步声明的 harness 工具名集合。
 */
export function toolSurfaceVerdict(tool: string, allowed: readonly string[]): Adjudication {
  if (tool.startsWith(META_TOOL_PREFIX) || allowed.includes(tool)) {
    return { ok: true }
  }
  const listed = allowed.length === 0 ? '（本步未声明任何 harness 工具）' : allowed.join(', ')
  return {
    ok: false,
    reason: guardDenyReason(
      'tool-denied',
      tool,
      GUARD_RULE_IDS.toolSurface,
      `本步允许：${listed}；元工具 alioth_* 不受限`,
    ),
  }
}

/** 路径切段：只丢弃空段（保留 `.` / `..` 供穿越检查显式拒绝）。 */
function segmentsOf(target: string): readonly string[] {
  return target.split(/[\\/]+/).filter(segment => segment !== '')
}

/** 路径归属解析结果：成功时给出 adapter 口径的相对路径（`Pre-Proc/{ns}/...`）。 */
export type PathOrigin =
  | { readonly ok: true; readonly relative: string; readonly segments: readonly string[] }
  | { readonly ok: false; readonly reason: string }

/**
 * 判定写路径归属：先显式拒绝 `.` / `..`，再要求落在本部署的 `preProcRoot/{ns}/` 内。
 * @param target - 调用给出的路径参数原文。
 * @param namespace - 当前会话的 namespace。
 * @param preProcRoot - 部署的 Pre-Proc 根。
 */
export function pathOrigin(target: string, namespace: string, preProcRoot: string): PathOrigin {
  const segments = segmentsOf(target)
  const traversal = segments.find(segment => segment === '.' || segment === '..')
  if (traversal !== undefined) {
    return { ok: false, reason: `路径含 "${traversal}" 组件（显式拒绝穿越）：${target}` }
  }
  if (!path.isAbsolute(target)) {
    if (segments[0] !== PRE_PROC_SEGMENT || segments[1] !== namespace) {
      return { ok: false, reason: `路径不在 Pre-Proc/${namespace}/ 下：${target}` }
    }
    return { ok: true, relative: segments.join('/'), segments: segments.slice(2) }
  }
  const root = path.resolve(preProcRoot, namespace)
  const resolved = path.resolve(target)
  if (resolved !== root && !resolved.startsWith(`${root}${path.sep}`)) {
    return { ok: false, reason: `路径不在 Pre-Proc/${namespace}/ 下：${target}` }
  }
  const inner = path.relative(root, resolved)
  const relative = inner === '' ? '' : `Pre-Proc/${namespace}/${inner.split(path.sep).join('/')}`
  return { ok: true, relative, segments: inner === '' ? [] : inner.split(path.sep) }
}

/**
 * 写沙箱：写路径必须落在 `Pre-Proc/{ns}/{Sources,Prototypes,Apps,AppAgentTraces}/` 内。
 * @param target - 调用给出的路径参数原文。
 * @param namespace - 当前会话的 namespace。
 * @param preProcRoot - 部署的 Pre-Proc 根（绝对形式据此判定）。
 */
export function sandboxVerdict(target: string, namespace: string, preProcRoot: string): Adjudication {
  const deny = (detail: string): Adjudication => ({
    ok: false,
    reason: guardDenyReason('write-outside-sandbox', 'write', GUARD_RULE_IDS.writeSandbox, detail),
  })
  const origin = pathOrigin(target, namespace, preProcRoot)
  if (!origin.ok) {
    return deny(origin.reason)
  }
  const zone = origin.segments[0]
  if (zone === undefined || SANDBOX_ZONES[zone] === undefined) {
    return deny(`写面限于 Pre-Proc/${namespace}/{Sources,Prototypes,Apps,AppAgentTraces}/：${target}`)
  }
  const base = origin.segments[origin.segments.length - 1]
  if (base !== undefined && SANDBOX_WRITE_BLACKLIST[base] === true) {
    return deny(`${base} 是写黑名单文件（授权声明只由人工维护）：${target}`)
  }
  return { ok: true }
}

/**
 * plan 写面收窄：plan（方案）步只写本步 `output_glob` 声明的产物；越界即拒绝。
 * glob 已由 run state 解析过 `{ns}`/`{module}`/`{app}` 模板段。
 * @param target - 调用给出的路径参数原文。
 * @param namespace - 当前会话的 namespace。
 * @param preProcRoot - 部署的 Pre-Proc 根（绝对形式据此判定）。
 * @param planWriteGlobs - 当前 plan 步的已解析 output_glob 列表。
 */
export function planWriteVerdict(
  target: string,
  namespace: string,
  preProcRoot: string,
  planWriteGlobs: readonly string[],
): Adjudication {
  const origin = pathOrigin(target, namespace, preProcRoot)
  if (!origin.ok) {
    return {
      ok: false,
      reason: guardDenyReason('plan-write-outside-scope', 'write', GUARD_RULE_IDS.planWriteScope, origin.reason),
    }
  }
  if (planWriteGlobs.some(glob => path.matchesGlob(origin.relative, glob))) {
    return { ok: true }
  }
  const declared = planWriteGlobs.length === 0 ? '（本步未声明 output_glob）' : planWriteGlobs.join(', ')
  return {
    ok: false,
    reason: guardDenyReason(
      'plan-write-outside-scope',
      'write',
      GUARD_RULE_IDS.planWriteScope,
      `plan 步写面：${declared}；本次目标：${origin.relative === '' ? target : origin.relative}`,
    ),
  }
}
