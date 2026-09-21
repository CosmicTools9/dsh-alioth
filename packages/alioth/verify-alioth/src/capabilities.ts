/**
 * 运行面能力与降级广告 —— 对齐上游 `capabilities.rs`：
 *
 * - 六组只读事实：`tools` / `skills` / `gates` / `autonomy` / `llm` / `budget`；
 * - 值三态：具体值 / `unknown(reason)`——不可得 MUST 显式（MUST NOT 空值、省略或猜测）；
 * - **组级错误隔离**：任一组采集失败只把该组置 `unknown(reason)`，其余组照常返回；
 * - 零副作用：纯聚合，不写盘、不改状态。
 * @module @dsh-alioth/verify-alioth/capabilities
 */

/** 单组事实（不可得 = 显式 unknown + 原因）。 */
export type CapabilityValue<T> = { readonly kind: 'value'; readonly value: T } | { readonly kind: 'unknown'; readonly reason: string }

/** 技能适配器事实（漂移判定结论由采集器给出）。 */
export interface SkillFact {
  readonly name: string
  readonly version: string
  readonly drifted: boolean
}

/** 门与阻塞组。 */
export interface GatesFact {
  readonly degraded: readonly string[]
  readonly human: readonly string[]
  readonly deferredOpen: number
  readonly plansPending: number
}

/** 自主政策组（政策不可解析 → failClosed=true，全部动作转人工）。 */
export interface AutonomyFact {
  readonly level: string
  readonly failClosed: boolean
}

/** LLM 就绪组。 */
export interface LlmFact {
  readonly ready: boolean
  readonly probe: string
}

/** turn 预算组。 */
export interface BudgetFact {
  readonly maxSteps: number
  readonly turnsUsed: number
  readonly turnTimeoutSec: number
}

/** 能力广告报表（只读）。 */
export interface CapabilityReport {
  readonly tools: CapabilityValue<readonly string[]>
  readonly skills: CapabilityValue<readonly SkillFact[]>
  readonly gates: CapabilityValue<GatesFact>
  readonly autonomy: CapabilityValue<AutonomyFact>
  readonly llm: CapabilityValue<LlmFact>
  readonly budget: CapabilityValue<BudgetFact>
}

type GroupKey = keyof CapabilityReport

/** 采集结果：成功带原始值，失败带原因（供该组降级为 unknown）。 */
type GroupProbe = { readonly ok: true; readonly value: unknown } | { readonly ok: false; readonly reason: string }

async function runGroup(key: GroupKey, collector: (() => Promise<unknown> | unknown) | undefined): Promise<GroupProbe> {
  if (collector === undefined) {
    return { ok: false, reason: `未提供 ${key} 组采集器（不可得必须显式 unknown，不得省略或默认）` }
  }
  try {
    return { ok: true, value: await collector() }
  } catch (error) {
    return { ok: false, reason: `${key} 组采集失败：${error instanceof Error ? error.message : String(error)}` }
  }
}

function toCapability<T>(key: GroupKey, probe: GroupProbe, check: (value: unknown) => T | null): CapabilityValue<T> {
  if (!probe.ok) return { kind: 'unknown', reason: probe.reason }
  const value = check(probe.value)
  if (value === null) {
    return { kind: 'unknown', reason: `${key} 组事实形态非法（采集器返回值与契约形态不符，MUST NOT 猜测）` }
  }
  return { kind: 'value', value }
}

function asRecord(value: unknown): Record<string, unknown> | null {
  return typeof value === 'object' && value !== null && !Array.isArray(value) ? (value as Record<string, unknown>) : null
}

function asStringArray(value: unknown): readonly string[] | null {
  return Array.isArray(value) && value.every(item => typeof item === 'string') ? (value as readonly string[]) : null
}

function asInteger(value: unknown): number | null {
  return typeof value === 'number' && Number.isInteger(value) && value >= 0 ? value : null
}

function checkSkills(value: unknown): readonly SkillFact[] | null {
  if (!Array.isArray(value)) return null
  const out: SkillFact[] = []
  for (const item of value) {
    const record = asRecord(item)
    if (record === null) return null
    const name = record['name']
    const version = record['version']
    const drifted = record['drifted']
    if (typeof name !== 'string' || name === '' || typeof version !== 'string' || version === '' || typeof drifted !== 'boolean') {
      return null
    }
    out.push({ name, version, drifted })
  }
  return out
}

function checkGates(value: unknown): GatesFact | null {
  const record = asRecord(value)
  if (record === null) return null
  const degraded = asStringArray(record['degraded'])
  const human = asStringArray(record['human'])
  const deferredOpen = asInteger(record['deferredOpen'])
  const plansPending = asInteger(record['plansPending'])
  if (degraded === null || human === null || deferredOpen === null || plansPending === null) return null
  return { degraded, human, deferredOpen, plansPending }
}

function checkAutonomy(value: unknown): AutonomyFact | null {
  const record = asRecord(value)
  if (record === null) return null
  const level = record['level']
  const failClosed = record['failClosed']
  if (typeof level !== 'string' || level.trim() === '' || typeof failClosed !== 'boolean') return null
  return { level, failClosed }
}

function checkLlm(value: unknown): LlmFact | null {
  const record = asRecord(value)
  if (record === null) return null
  const ready = record['ready']
  const probe = record['probe']
  if (typeof ready !== 'boolean' || typeof probe !== 'string') return null
  return { ready, probe }
}

function checkBudget(value: unknown): BudgetFact | null {
  const record = asRecord(value)
  if (record === null) return null
  const maxSteps = asInteger(record['maxSteps'])
  const turnsUsed = asInteger(record['turnsUsed'])
  const turnTimeoutSec = asInteger(record['turnTimeoutSec'])
  if (maxSteps === null || turnsUsed === null || turnTimeoutSec === null) return null
  return { maxSteps, turnsUsed, turnTimeoutSec }
}

/**
 * 聚合六组能力事实。每组独立 try/catch：失败只把该组置 `unknown(reason)`，其余组照常。
 * 未提供采集器 = 该组事实不可得 → 同样显式 `unknown(reason)`。
 */
export async function collectCapabilities(
  input: Partial<Record<GroupKey, () => Promise<unknown> | unknown>>,
): Promise<CapabilityReport> {
  const [tools, skills, gates, autonomy, llm, budget] = await Promise.all([
    runGroup('tools', input.tools),
    runGroup('skills', input.skills),
    runGroup('gates', input.gates),
    runGroup('autonomy', input.autonomy),
    runGroup('llm', input.llm),
    runGroup('budget', input.budget),
  ])

  return {
    tools: toCapability('tools', tools, asStringArray),
    skills: toCapability('skills', skills, checkSkills),
    gates: toCapability('gates', gates, checkGates),
    autonomy: toCapability('autonomy', autonomy, checkAutonomy),
    llm: toCapability('llm', llm, checkLlm),
    budget: toCapability('budget', budget, checkBudget),
  }
}
