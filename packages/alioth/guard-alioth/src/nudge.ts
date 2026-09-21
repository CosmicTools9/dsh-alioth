/**
 * 闭环证据追问（上游 `add-appagent-completion-nudge` 的等价物）。
 *
 * 判定是纯函数，输入是本 turn 的台账（`ledger` 模块产出）：本 turn 曾调用产物类工具、
 * 却没有任何闭环证据（验证/门禁/裁决）时，注入**一次**收尾追问，要求先取得可判定证据
 * 再收尾。上限 1 次/turn，且与「向用户澄清提问」「终态汇报」互斥——三者都是收尾动作，
 * 同一 turn 内不得叠加。
 *
 * 【落点选择】实现落在 `agent/pre-step` 而非 `tools/post-execute`：
 * - pre-step 的 `{kind:'enter', messages}` 是**模型必须应答的 user 消息**，追问才是强约束；
 *   post-execute 的 `additionalContexts` 是随工具结果附带的被动上下文；
 * - pre-step 是天然的「本 turn 已做完一段动作、准备继续下一步」的观察点，正好是收尾追问
 *   的时机；auth-alioth 也已在同一缝上做执行前拦截（仓库内先例）。
 * @module @dsh-alioth/guard-alioth/nudge
 */

import { GUARD_RULE_IDS } from './rules.ts'

/** 产物类工具：改动/产出 AppAgent 产物（harness 文件工具 + 程序化产物写入面）。 */
export const ARTIFACT_TOOLS: readonly string[] = [
  'write',
  'edit',
  'alioth_app_write',
  'alioth_app_configure',
  'alioth_entity_write',
]

/** 闭环证据工具：取得**可判定**的证据（跑门禁 / 验证产物 / 独立裁决）。 */
export const CLOSURE_EVIDENCE_TOOLS: readonly string[] = [
  'alioth_workflow_complete',
  'alioth_verify',
  'alioth_closure',
]

/** 澄清提问工具：向用户提问本身就是收尾动作，与追问互斥。 */
export const CLARIFICATION_TOOLS: readonly string[] = ['ask_user_question']

/** 终态汇报工具：已交付结论，与追问互斥。 */
export const TERMINAL_REPORT_TOOLS: readonly string[] = ['present']

/** 追问的正文明细（注入前可断言）。 */
export const CLOSURE_NUDGE_TEXT =
  `[rule:${GUARD_RULE_IDS.closureNudge}] 本 turn 已产出或修改产物，但没有任何闭环证据`
  + '（未调用 alioth_workflow_complete / alioth_verify / alioth_closure）。'
  + '收尾前必须取得可判定的证据：先跑该步门禁或验证工具，再把结论写进汇报；'
  + '若确实无法取得证据，直接说明缺什么证据与阻塞点，不要以「已完成」收尾。'

/** 追问判定的输入视图（由台账给出）。 */
export interface NudgeView {
  readonly turn: number
  readonly artifactCalls: number
  readonly evidenceCalls: number
  readonly clarification: boolean
  readonly terminalReport: boolean
  readonly nudged: boolean
}

/** 追问判定：`nudge: false` 时 `reason` 说明为何不追问（可留痕）。 */
export type NudgeDecision =
  | { readonly nudge: true; readonly text: string }
  | { readonly nudge: false; readonly reason: string }

/**
 * 判定是否注入收尾追问。
 * @param enabled - 部署开关（`Config.nudge`）。
 * @param view - 本 turn 台账视图；`null` = 本 turn 尚无事件记录。
 */
export function decideClosureNudge(enabled: boolean, view: NudgeView | null): NudgeDecision {
  if (!enabled) {
    return { nudge: false, reason: 'nudge 未启用（Config.nudge=false）' }
  }
  if (view === null) {
    return { nudge: false, reason: '本 turn 尚无事件记录' }
  }
  if (view.nudged) {
    return { nudge: false, reason: `turn ${view.turn} 已追问过（上限 1 次/turn）` }
  }
  if (view.clarification) {
    return { nudge: false, reason: `turn ${view.turn} 已向用户澄清提问（互斥）` }
  }
  if (view.terminalReport) {
    return { nudge: false, reason: `turn ${view.turn} 已出终态汇报（互斥）` }
  }
  if (view.artifactCalls === 0) {
    return { nudge: false, reason: `turn ${view.turn} 未调用产物类工具（无需闭环追问）` }
  }
  if (view.evidenceCalls > 0) {
    return { nudge: false, reason: `turn ${view.turn} 已有闭环证据` }
  }
  return { nudge: true, text: CLOSURE_NUDGE_TEXT }
}
