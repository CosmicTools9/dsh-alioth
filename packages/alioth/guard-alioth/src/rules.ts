/**
 * 守卫侧规则码与反馈渲染。
 *
 * 三个执行面拒绝（工具面 / 写沙箱 / plan 写面）的规则码由 Wave 2 契约 §3 逐字指定
 * （`tool-outside-declared-surface` / `write-outside-sandbox` /
 * `plan-write-outside-scope`）。渲染**复用** skill-alioth 的修复契约表
 * （{@link repairContractFor} + {@link formatRepairError}），只覆盖规则码本身：
 * 修复类 / 消息 / 下一步动作 / 证据截断口径两侧同源，不引入第二套修复词汇。
 * 因此每条拒绝文本都是单行 `[rule:<id>] class=<class> · <message> · 下一步：<action> · 证据：<head>`，
 * 可被 `ruleIdFromError`（唯一解析点）稳定回收。
 * @module @dsh-alioth/guard-alioth/rules
 */

import {
  formatRepairError,
  repairContractFor,
  type FailureKind,
  type RepairClass,
} from '@dsh-alioth/skill-alioth'

/**
 * 守卫规则码。
 *
 * 注：`tool-outside-declared-surface` / `write-outside-sandbox` / `plan-write-outside-scope`
 * 是契约 §3 指定的字面量，与 skill-alioth `REGISTERED_RULE_IDS` 中同语义的
 * `tool-call-denied` / `tool-write-outside-sandbox` / `plan-phase-write-outside-scope`
 * **并存**：守卫面按 §3 的码发出，修复类与建议动作仍取自该表。两套码的合并由主
 * agent 在 Wave 2 收口时决定（要么把守卫码登记进 `REGISTERED_RULE_IDS`，要么把
 * §3 的三个名字改为别名）。
 */
export const GUARD_RULE_IDS = {
  /** 调用名不在当前步骤声明的工具面内。 */
  toolSurface: 'tool-outside-declared-surface',
  /** 写路径越出 `Pre-Proc/{ns}/{Sources,Prototypes,Apps,AppAgentTraces}/`。 */
  writeSandbox: 'write-outside-sandbox',
  /** plan 步写入本步 `output_glob` 之外的路径。 */
  planWriteScope: 'plan-write-outside-scope',
  /** 修复墙：同一错误签名重复达到上限 / 同一调用签名连续重复。 */
  repairWall: 'repair-wall',
  /** 单 turn 墙钟或步数超限。 */
  turnBudget: 'turn-budget-exceeded',
  /** 闭环证据追问。 */
  closureNudge: 'closure-evidence-nudge',
  /** 无 ns 可解析：显式降级而非猜测。 */
  unknownScope: 'unknown-scope',
} as const

export type GuardRuleId = (typeof GUARD_RULE_IDS)[keyof typeof GUARD_RULE_IDS]

/**
 * 渲染一条拒绝原因：修复契约取自 skill-alioth 的失败特征表，规则码换成守卫侧字面量。
 * @param failure - 结构化失败特征（调用点给出）。
 * @param source - 工具或程序名（进契约消息）。
 * @param ruleId - 守卫规则码（契约 §3 字面量）。
 * @param evidence - 原始证据（如本步允许清单）。
 */
export function guardDenyReason(
  failure: FailureKind,
  source: string,
  ruleId: GuardRuleId,
  evidence: string,
): string {
  return formatRepairError({ ...repairContractFor(failure, source, evidence), ruleId })
}

/** 守卫侧直接构造的反馈（如修复墙、turn 预算）——非门禁失败特征，字段由调用点给出。 */
export interface GuardFeedbackInput {
  readonly ruleId: GuardRuleId
  readonly repairClass: RepairClass
  readonly message: string
  readonly action: string
  readonly evidence?: string
}

/** 用与 {@link guardDenyReason} 完全相同的单行格式渲染守卫反馈。 */
export function guardFeedback(input: GuardFeedbackInput): string {
  return formatRepairError({
    ruleId: input.ruleId,
    class: input.repairClass,
    message: input.message,
    suggestedAction: input.action,
    evidence: input.evidence ?? '',
  })
}
