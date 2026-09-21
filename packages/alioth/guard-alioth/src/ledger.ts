/**
 * 会话台账：`session/event` 火灾软管的**唯一**消费者。
 *
 * 一份事件流喂三个消费面，避免同一事件被多次解读：
 * - 用量：`assistant/message.usage` → `UsageEvent[]`，最终交给 verify-alioth 的
 *   `aggregateUsage`（成本口径不可得时显式 `unavailable`，MUST NOT 以 0 冒充）；
 * - turn 台账：产物/证据/澄清/终态计数（闭环追问的判据）与墙钟、步数；
 * - turn 预算：`turn/end` 时按 `turnTimeoutSec`（墙钟）与 `maxStepsPerTurn`（步数）判定超限，
 *   记一条待消费的违规，由下一次 `agent/pre-step` 拒绝并写清原因。
 *
 * 成本上限不在守卫面判定：守卫没有价表部署选择（`usage()` 如实报 `unavailable`），
 * 成本可见性由 T5 的 `alioth_usage` 负责。每次判定的时间口径取自事件自带的
 * `time`（epoch ms），因此台账完全可重放、测试无需假时钟。
 * @module @dsh-alioth/guard-alioth/ledger
 */

import type { SessionEvent } from '@deepseek-ai/dsh-session'
import { aggregateUsage, type UsageEvent, type UsageSummary } from '@dsh-alioth/verify-alioth'
import { GUARD_RULE_IDS, guardFeedback } from './rules.ts'
import {
  ARTIFACT_TOOLS,
  CLARIFICATION_TOOLS,
  CLOSURE_EVIDENCE_TOOLS,
  TERMINAL_REPORT_TOOLS,
  type NudgeView,
} from './nudge.ts'

/** turn 台账（只读视图，闭环追问的输入）。 */
export interface TurnLedger extends NudgeView {
  /** 从 `turn/start` 到 `turn/end` 的墙钟；未结束 → 0。 */
  readonly wallMs: number
  readonly steps: number
  readonly calls: number
  readonly closed: boolean
}

/** 一次预算超限的判定结果（消费一次即清）。 */
export interface TurnViolation {
  readonly sessionId: string
  readonly turn: number
  readonly wallMs: number
  readonly steps: number
  readonly reason: string
}

/** 台账的部署选择（守卫 Config 的预算字段）。 */
export interface SessionLedgerOptions {
  readonly maxStepsPerTurn?: number
  readonly turnTimeoutSec?: number
}

interface MutableTurn {
  turn: number
  startedAt: number | null
  wallMs: number
  steps: number
  calls: number
  artifactCalls: number
  evidenceCalls: number
  clarification: boolean
  terminalReport: boolean
  nudged: boolean
  closed: boolean
}

interface SessionState {
  turn: MutableTurn | null
  usage: UsageEvent[]
  /** `${turn}:${step}` → 该步 `step/start` 的时刻（算模型调用时延）。 */
  stepStarts: Map<string, number>
  pending: TurnViolation | null
}

function emptyTurn(turn: number, startedAt: number): MutableTurn {
  return {
    turn,
    startedAt,
    wallMs: 0,
    steps: 0,
    calls: 0,
    artifactCalls: 0,
    evidenceCalls: 0,
    clarification: false,
    terminalReport: false,
    nudged: false,
    closed: false,
  }
}

function viewOf(turn: MutableTurn): TurnLedger {
  return {
    turn: turn.turn,
    wallMs: turn.wallMs,
    steps: turn.steps,
    calls: turn.calls,
    artifactCalls: turn.artifactCalls,
    evidenceCalls: turn.evidenceCalls,
    clarification: turn.clarification,
    terminalReport: turn.terminalReport,
    nudged: turn.nudged,
    closed: turn.closed,
  }
}

/** 会话台账：观察事件、汇总用量、累计 turn 台账、判定 turn 预算。 */
export class SessionLedger {
  private readonly sessions = new Map<string, SessionState>()
  private readonly options: SessionLedgerOptions

  constructor(options: SessionLedgerOptions = {}) {
    this.options = options
  }

  private stateFor(sessionId: string): SessionState {
    const existing = this.sessions.get(sessionId)
    if (existing !== undefined) {
      return existing
    }
    const created: SessionState = { turn: null, usage: [], stepStarts: new Map(), pending: null }
    this.sessions.set(sessionId, created)
    return created
  }

  /** 观察一条会话事件（火灾软管回调，绝不抛出：坏事件按忽略处理）。 */
  observe(sessionId: string, event: SessionEvent): void {
    const state = this.stateFor(sessionId)
    switch (event.type) {
      case 'turn/start':
        state.turn = emptyTurn(event.data.turn, event.time)
        return
      case 'step/start':
        if (state.turn !== null && state.turn.turn === event.data.turn) {
          state.turn.steps += 1
        }
        state.stepStarts.set(`${event.data.turn}:${event.data.step}`, event.time)
        return
      case 'assistant/message': {
        const usage = event.data.usage
        if (usage === undefined) {
          return
        }
        const started = state.stepStarts.get(`${event.data.turn}:${event.data.step}`)
        state.usage.push({
          step: event.data.step,
          model: event.data.message.source.model,
          tokensIn: usage.inputTokens,
          tokensOut: usage.outputTokens,
          latencyMs: started === undefined ? 0 : Math.max(0, event.time - started),
          turn: event.data.turn,
        })
        return
      }
      case 'tool/call':
        this.noteCall(state, event.data.turn, event.data.name)
        return
      case 'turn/end':
        this.closeTurn(sessionId, state, event.data.turn, event.time)
        return
      default:
        return
    }
  }

  private noteCall(state: SessionState, turn: number, tool: string): void {
    if (state.turn === null || state.turn.turn !== turn) {
      return
    }
    state.turn.calls += 1
    if (ARTIFACT_TOOLS.includes(tool)) {
      state.turn.artifactCalls += 1
    }
    if (CLOSURE_EVIDENCE_TOOLS.includes(tool)) {
      state.turn.evidenceCalls += 1
    }
    if (CLARIFICATION_TOOLS.includes(tool)) {
      state.turn.clarification = true
    }
    if (TERMINAL_REPORT_TOOLS.includes(tool)) {
      state.turn.terminalReport = true
    }
  }

  private closeTurn(sessionId: string, state: SessionState, turn: number, time: number): void {
    const record = state.turn
    if (record === null || record.turn !== turn) {
      return
    }
    record.wallMs = record.startedAt === null ? 0 : Math.max(0, time - record.startedAt)
    record.closed = true
    const violation = this.judgeTurn(sessionId, record)
    if (violation !== null) {
      state.pending = violation
    }
  }

  /** turn 预算判定：墙钟与步数各自独立，任一超限即记一条违规（含规则码前缀原因）。 */
  private judgeTurn(sessionId: string, record: MutableTurn): TurnViolation | null {
    const timeoutSec = this.options.turnTimeoutSec
    if (timeoutSec !== undefined && record.wallMs > timeoutSec * 1000) {
      return {
        sessionId,
        turn: record.turn,
        wallMs: record.wallMs,
        steps: record.steps,
        reason: guardFeedback({
          ruleId: GUARD_RULE_IDS.turnBudget,
          repairClass: 'not-fixable',
          message: `turn ${record.turn} 墙钟 ${record.wallMs}ms 超过 turnTimeoutSec=${timeoutSec}s`,
          action: '把工作拆成更小的 turn 后重来；需要更长单轮时调高部署配置 turnTimeoutSec',
          evidence: `steps=${record.steps} calls=${record.calls}`,
        }),
      }
    }
    const maxSteps = this.options.maxStepsPerTurn
    if (maxSteps !== undefined && record.steps > maxSteps) {
      return {
        sessionId,
        turn: record.turn,
        wallMs: record.wallMs,
        steps: record.steps,
        reason: guardFeedback({
          ruleId: GUARD_RULE_IDS.turnBudget,
          repairClass: 'not-fixable',
          message: `turn ${record.turn} 步数 ${record.steps} 超过 maxStepsPerTurn=${maxSteps}`,
          action: '把工作拆成更小的 turn 后重来；需要更多步数时调高部署配置 maxStepsPerTurn',
          evidence: `wallMs=${record.wallMs} calls=${record.calls}`,
        }),
      }
    }
    return null
  }

  /** 本 turn 的台账视图（无记录 → `null`）。 */
  turnRecord(sessionId: string, turn: number): TurnLedger | null {
    const record = this.sessions.get(sessionId)?.turn
    return record === undefined || record === null || record.turn !== turn ? null : viewOf(record)
  }

  /** 标记本 turn 已追问（上限 1 次/turn 的消费点）。 */
  markNudged(sessionId: string, turn: number): void {
    const record = this.sessions.get(sessionId)?.turn
    if (record !== null && record !== undefined && record.turn === turn) {
      record.nudged = true
    }
  }

  /** 会话用量汇总（无事件 → 全 0；成本口径不可得时显式 `unavailable`）。 */
  usage(sessionId: string): UsageSummary {
    return aggregateUsage(this.sessions.get(sessionId)?.usage ?? [])
  }

  /** 消费一次待处理的预算违规（阻断下一步后即清，不反复卡死会话）。 */
  takeViolation(sessionId: string): TurnViolation | null {
    const state = this.sessions.get(sessionId)
    if (state === undefined || state.pending === null) {
      return null
    }
    const violation = state.pending
    state.pending = null
    return violation
  }
}
