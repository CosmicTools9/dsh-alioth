/**
 * `FlowPlan` 的 wire 形态（上游 `state.rs::FlowPlan` 的 serde 面）。
 *
 * 上游把 flow plan 落成 `Pre-Proc/{namespace}/Apps/{app}/flow-plan.json`，`compose_app` 再**确定性**
 * 地从它组装模块/block/扩展产物（`composer.rs::compose_from_flow_plan`——报告的产物清单是纯派生
 * 数据，不再依赖 LLM）。本仓的计划在管线里是**参数**，此前没有产物面：消费者拿不到「这份 app 是按
 * 什么计划生成的」，计划漂移也无从对照。本模块给出与上游逐键同形的读写。
 *
 * 名面：上游 serde 默认 = 字段原名（snake_case）；本仓 TS 面用 camelCase。写出用 snake（上游 canonical），
 * 读入**两种都接受**（该模块的 serde-alias 兼容口径：`created_scenes`/`created_factors` 等旧别名照收）。
 * `ontology_model_json` 是 JSON **字符串**——只做键名转换，绝不下钻字符串内容。
 * @module @dsh-alioth/skill-alioth/flow-plan-wire
 */

import type { FlowPlan } from './agent-contract.ts'

/** camelCase → snake_case（键名转换；上游 serde 面）。 */
function toSnake(key: string): string {
  return key.replace(/[A-Z]/g, letter => `_${letter.toLowerCase()}`)
}

/** snake_case → camelCase（读入归一）。 */
function toCamel(key: string): string {
  return key.replace(/_([a-z0-9])/g, (_match, letter: string) => letter.toUpperCase())
}

/** 递归转换键名；字符串/数字/布尔/null 原样（字符串里的 JSON 内容不动）。 */
function renameKeys(value: unknown, rename: (key: string) => string): unknown {
  if (Array.isArray(value)) {
    return value.map(entry => renameKeys(entry, rename))
  }
  if (typeof value !== 'object' || value === null) {
    return value
  }
  const out: Record<string, unknown> = {}
  for (const [key, entry] of Object.entries(value as Record<string, unknown>)) {
    out[rename(key)] = renameKeys(entry, rename)
  }
  return out
}

/**
 * 计划 → 上游 wire 形态（snake_case 键，逐键同形）。
 * @param plan - the pipeline plan.
 */
export function flowPlanToWire(plan: FlowPlan): Record<string, unknown> {
  return renameKeys(plan, toSnake) as Record<string, unknown>
}

/** 读入必需键（皆为上游必填面：缺一即拒绝，不猜默认值）。 */
const REQUIRED_KEYS: readonly string[] = [
  'usedModules',
  'namespace',
  'knownEntities',
  'workflowSteps',
  'missingInfo',
  'createdModules',
  'createdBlocks',
  'createdServices',
]

/**
 * wire 形态 → 计划（snake/camel 两种名面都接受；缺必需键 → `null`，fail-closed 不猜）。
 * @param value - parsed JSON (or any unknown).
 */
export function flowPlanFromWire(value: unknown): FlowPlan | null {
  if (typeof value !== 'object' || value === null || Array.isArray(value)) {
    return null
  }
  const raw = value as Record<string, unknown>
  // 旧别名归一 MUST 先于必需键检查：`created_scenes` 经 camel 化只留下 `createdScenes`，
  // 若先判必需键就会把一份合法的旧形态判成 None（fail-closed 变成 fail-wrong）。
  const aliased: Record<string, unknown> = { ...raw }
  for (const [alias, canonical] of [
    ['created_scenes', 'created_blocks'],
    ['created_factors', 'created_services'],
  ] as const) {
    if (!(canonical in aliased) && alias in raw) {
      aliased[canonical] = raw[alias]
    }
  }
  const plan = renameKeys(aliased, toCamel) as Record<string, unknown>
  for (const key of REQUIRED_KEYS) {
    if (!(key in plan)) {
      return null
    }
  }
  return plan as unknown as FlowPlan
}
