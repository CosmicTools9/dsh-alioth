/**
 * `@dsh-alioth/guard-alioth` — AppAgent 执行层守卫插件（Wave 2 / T4）。
 *
 * 把上游 AppAgent 在**工具注册层**做的执行约束落到 harness 的三条缝上：
 *
 * 1. `tools/pre-execute`：
 *    - 工具面强制——调用名必须在当前步骤声明的 harness 工具面内（`enforceToolSurface`）；
 *    - namespace 写沙箱——写类调用的路径必须落在 `Pre-Proc/{ns}/{Sources,Prototypes,Apps,AppAgentTraces}/`；
 *    - plan 写面收窄——plan 步的写入必须命中该步 `output_glob`。
 *    无 ns 可解析（headless / 未选工作区）时**不猜**：放行并留一条显式降级证据。
 * 2. `tools/post-execute`：修复墙——`RetryBudget` 记调用/错误签名，三档动作
 *    （trim-retry / escalate / terminate）都收敛为 `block` + 带规则码前缀的纠正反馈。
 * 3. `agent/pre-step`：闭环证据追问（上限 1 次/turn，与澄清提问/终态汇报互斥）与
 *    turn 预算超限后的下一步阻断（`reject`，原因带规则码前缀）。
 *
 * `session/event` 是唯一的观察面：喂 `ledger`（用量/turn 台账/预算）与 `scope`
 * （本会话最近一次 `alioth_workflow_*` 调用的 `{ns,app}`）。范围与当前步骤每次判定
 * 现读磁盘，绝不使用过期缓存。
 *
 * 服务面 `ctx.aliothGuard`：`whitelistSource()`（门禁程序白名单生效来源：`file` /
 * `code_default` + 可区分原因）、`activeScope(sessionId)`、`usage(sessionId)`、
 * `degradations()`（降级不得只活在日志里）。
 * @module @dsh-alioth/guard-alioth
 */

import path from 'node:path'
import { readFileSync } from 'node:fs'
import { Context } from '@deepseek-ai/cordis'
import z from '@deepseek-ai/schemastery'
import { createUserMessage } from '@deepseek-ai/dsh-llm'
import type { Session } from '@deepseek-ai/dsh-session'
import type { ToolExecution } from '@deepseek-ai/dsh-tools'
import { RetryBudget, type RetryDecision } from '@dsh-alioth/skill-alioth'
import type { PriceTable, UsageSummary } from '@dsh-alioth/verify-alioth'
import { SessionLedger, type TurnViolation } from './ledger.ts'
import { decideClosureNudge, type NudgeDecision } from './nudge.ts'
import { GUARD_RULE_IDS, guardFeedback, type GuardRuleId } from './rules.ts'
import { RunScopeResolver, WORKFLOW_SCOPE_TOOLS, type ActiveScope } from './scope.ts'
import {
  planWriteVerdict,
  sandboxVerdict,
  toolSurfaceVerdict,
  writePathOf,
} from './surface.ts'
import { readWhitelistSource, type WhitelistSourceReport } from './whitelist.ts'

export const name = 'guard-alioth'
export const inject = ['aliothEnv', 'tools']

/** 适配器文件名缺省值（与 workflow 插件同一模型分发的 `skill-adapters/` 目录）。 */
export const DEFAULT_ADAPTER = 'alioth-app.yaml'

/** 降级证据表上限（bounded，避免长驻进程无界增长）。 */
const DEGRADATION_LIMIT = 100

export interface Config {
  /** Pre-Proc 产物树根；写沙箱的绝对路径按此判定。 */
  readonly preProcRoot: string
  /** 状态根；run state 落在 `<dataRoot>/workflows/{ns}/{app}/run-state.json`。 */
  readonly dataRoot: string
  /** 适配器文件名（`skill-adapters/` 下），与 workflow 插件的部署选择一致。 */
  readonly adapter?: string
  /** 工具面强制开关；默认 true。 */
  readonly enforceToolSurface?: boolean
  /** 闭环证据追问开关；默认 true。 */
  readonly nudge?: boolean
  /** 同一调用签名连续重复上限（死循环判定）；默认由 skill-alioth 给出。 */
  readonly maxRepeatCall?: number
  /** 同一错误签名重复上限（第 2 次裁剪重试，第 N 次转人工）。 */
  readonly maxRepeatError?: number
  /** 单 turn 步数上限；缺省不限（部署选择）。 */
  readonly maxStepsPerTurn?: number
  /** 单 turn 墙钟上限（秒）；缺省不限（部署选择）。 */
  readonly turnTimeoutSec?: number
  /** 模型单价表：JSON 文件路径（`{ "<model>": { centsPerInK, centsPerOutK } }`）；缺省 → 成本口径 `unavailable`。 */
  readonly priceTable?: string
  /** 单 turn 估算成本上限（分）；与 `priceTable` 同时给出才生效。 */
  readonly maxTurnCostCents?: number
}

export const Config: z<Config> = z.object({
  preProcRoot: z.string().required(),
  dataRoot: z.string().required(),
  adapter: z.string().default(DEFAULT_ADAPTER),
  enforceToolSurface: z.boolean().default(true),
  nudge: z.boolean().default(true),
  maxRepeatCall: z.number(),
  maxRepeatError: z.number(),
  maxStepsPerTurn: z.number(),
  turnTimeoutSec: z.number(),
  priceTable: z.string(),
  maxTurnCostCents: z.number(),
})

/** 一条显式降级证据（`unknown-scope` / 步骤不可读等）。 */
export interface Degradation {
  /** 会话 id；无 agent 的调用为 `null`。 */
  readonly sessionId: string | null
  readonly ruleId: GuardRuleId
  readonly reason: string
  /** Unix epoch 毫秒。 */
  readonly time: number
}

/** `ctx.aliothGuard` 服务面。 */
export interface AliothGuardService {
  /** 门禁程序白名单的生效来源（`file` = 运行时镜像；`code_default` = 回退代码常量 + 可区分原因）。 */
  whitelistSource(): Promise<WhitelistSourceReport>
  /** 现取会话的运行范围；无 workflow 调用记录 → `null`（未知范围）。 */
  activeScope(sessionId: string): Promise<ActiveScope | null>
  /** 会话用量与成本口径（成本不可得时显式 `unavailable`）。 */
  usage(sessionId: string): UsageSummary
  /** 已记录的降级证据（有界列表）。 */
  degradations(): readonly Degradation[]
}

declare module '@deepseek-ai/cordis' {
  interface Context {
    aliothGuard: AliothGuardService
  }
}

/** 修复墙三档动作 → 纠正反馈文本（规则码前缀 + 明确的下一步）。 */
function wallFeedback(
  decision: Exclude<RetryDecision, 'allow'>,
  tool: string,
  target: string,
): string {
  if (decision === 'terminate') {
    return guardFeedback({
      ruleId: GUARD_RULE_IDS.repairWall,
      repairClass: 'not-fixable',
      message: `检测到重复调用死循环：${tool} 的同一调用签名已连续重复达到上限`,
      action: '停止重复该调用：改变参数或换工具；确认无法推进时转人工',
      evidence: target,
    })
  }
  if (decision === 'escalate') {
    return guardFeedback({
      ruleId: GUARD_RULE_IDS.repairWall,
      repairClass: 'not-fixable',
      message: `已达修复墙：同一错误签名重复达到上限（${tool}）`,
      action: '停止自动修复：列出未完成项与阻塞点转人工',
      evidence: target,
    })
  }
  return guardFeedback({
    ruleId: GUARD_RULE_IDS.repairWall,
    repairClass: 'retryable',
    message: `上下文已裁剪：${tool} 的同一错误签名已重复出现`,
    action: '重读该错误的完整文本，换策略后再试；同一路径连续失败即触发裁剪重试',
    evidence: target,
  })
}

export function apply(ctx: Context, config: Config): void {
  const preProcRoot = path.resolve(config.preProcRoot)
  const adapter = config.adapter ?? DEFAULT_ADAPTER
  const enforceToolSurface = config.enforceToolSurface ?? true
  const nudgeEnabled = config.nudge ?? true
  const degradations: Degradation[] = []
  const degradedKeys = new Set<string>()
  const budgets = new Map<string, RetryBudget>()

  const prices = loadPrices()
  const ledger = new SessionLedger({
    ...config.maxStepsPerTurn === undefined ? {} : { maxStepsPerTurn: config.maxStepsPerTurn },
    ...config.turnTimeoutSec === undefined ? {} : { turnTimeoutSec: config.turnTimeoutSec },
    ...config.maxTurnCostCents === undefined ? {} : { maxTurnCostCents: config.maxTurnCostCents },
    ...prices === undefined ? {} : { prices },
  })

  /**
   * 价表（可选部署选择）：挂载期同步读一次小 JSON——成本上限必须从第一轮起生效，
   * 若等异步 IO 回来再装配，窗口期内结束的 turn 会漏判。不可读/结构非法 → 降级留痕并置
   * `undefined`：成本口径如实 `unavailable` 且**不判**上限（MUST NOT 以 0 冒充，也不静默吞掉配置错误）。
   */
  function loadPrices(): PriceTable | undefined {
    const file = config.priceTable
    if (file === undefined) {
      return undefined
    }
    try {
      const parsed: unknown = JSON.parse(readFileSync(path.resolve(file), 'utf8'))
      if (typeof parsed !== 'object' || parsed === null || Array.isArray(parsed)) {
        throw new Error('价表须为对象：{ "<model>": { centsPerInK, centsPerOutK } }')
      }
      const table: Record<string, { centsPerInK: number; centsPerOutK: number }> = {}
      for (const [model, entry] of Object.entries(parsed as Record<string, unknown>)) {
        const record = entry as { centsPerInK?: unknown; centsPerOutK?: unknown } | null
        if (typeof record?.centsPerInK !== 'number' || typeof record?.centsPerOutK !== 'number') {
          throw new Error(`价表条目 ${model} 须含数字 centsPerInK/centsPerOutK`)
        }
        table[model] = { centsPerInK: record.centsPerInK, centsPerOutK: record.centsPerOutK }
      }
      return table
    } catch (error) {
      noteDegradation(
        null,
        GUARD_RULE_IDS.turnCost,
        `价表不可用（${file}）：${error instanceof Error ? error.message : String(error)}`,
      )
      return undefined
    }
  }

  /** 降级留痕：日志 + 有界证据表，按（会话, 规则码）去重，避免每次调用重复告警。 */
  function noteDegradation(sessionId: string | null, ruleId: GuardRuleId, reason: string): void {
    const dedupeKey = `${sessionId ?? 'no-agent'}\u0000${ruleId}`
    if (degradedKeys.has(dedupeKey)) {
      return
    }
    degradedKeys.add(dedupeKey)
    ctx.logger.warn(`guard-alioth: 降级 [rule:${ruleId}] ${reason}`)
    degradations.push({ sessionId, ruleId, reason, time: Date.now() })
    if (degradations.length > DEGRADATION_LIMIT) {
      degradations.shift()
    }
  }

  const resolver = new RunScopeResolver({
    modelDir: async () => (await ctx.aliothEnv.ready()).modelDir,
    workflowRoot: () => path.join(path.resolve(config.dataRoot), 'workflows'),
    adapter,
    onDegrade: (sessionId, key, reason) =>
      noteDegradation(sessionId, GUARD_RULE_IDS.unknownScope, `${key.namespace}/${key.app}: ${reason}`),
  })

  function budgetFor(sessionId: string): RetryBudget {
    const existing = budgets.get(sessionId)
    if (existing !== undefined) {
      return existing
    }
    const created = new RetryBudget({
      ...config.maxRepeatCall === undefined ? {} : { maxRepeatCall: config.maxRepeatCall },
      ...config.maxRepeatError === undefined ? {} : { maxRepeatError: config.maxRepeatError },
    })
    budgets.set(sessionId, created)
    return created
  }

  /** 修复墙的证据面：本会话当前未完成的范围与步骤（读不出则如实说明）。 */
  async function wallTarget(sessionId: string): Promise<string> {
    const scope = await resolver.activeScope(sessionId)
    if (scope === null) {
      return '未完成项：本会话尚无 alioth_workflow_* 调用记录，无法解析运行范围'
    }
    return scope.stepId === null
      ? `未完成项：${scope.namespace}/${scope.app} 的运行已结束（无当前步骤）`
      : `未完成项：${scope.namespace}/${scope.app} 的当前步骤 ${scope.stepId} 仍未通过`
  }

  const aliothGuard: AliothGuardService = {
    async whitelistSource(): Promise<WhitelistSourceReport> {
      return readWhitelistSource((await ctx.aliothEnv.ready()).modelDir)
    },
    activeScope: (sessionId: string) => resolver.activeScope(sessionId),
    usage: (sessionId: string) => ledger.usage(sessionId),
    degradations: () => [...degradations],
  }

  // ── 观察面：会话事件（唯一的观测入口） ──────────────────────────────────
  ctx.on('session/event', (session: Session, event) => {
    const sessionId = String(session.id)
    ledger.observe(sessionId, event)
    if (event.type === 'tool/call') {
      resolver.observeCall(sessionId, event.data.name, parseArguments(event.data.arguments))
    }
    if (event.type === 'turn/start') {
      budgets.delete(sessionId)
    }
  })

  // ── 执行前：工具面 / 写沙箱 / plan 写面 ────────────────────────────────
  ctx.on('tools/pre-execute', async (exec: ToolExecution, next) => {
    const writePath = writePathOf(exec.name, exec.arguments)
    if (writePath === undefined && !enforceToolSurface) {
      return next()
    }
    const sessionId = exec.agent === undefined ? null : String(exec.agent.id)
    if (sessionId === null) {
      noteDegradation(
        null,
        GUARD_RULE_IDS.unknownScope,
        `${exec.name}: 调用没有 agent（headless），无法解析 {ns,app}——按未知范围放行`,
      )
      return next()
    }
    const scope = await resolver.activeScope(sessionId)
    if (scope === null) {
      // workflow 元工具本身就是范围建立调用（参数带 {ns,app}）：它们无需被判定，
      // 缓存尚未登记也不构成降级证据。
      if (!WORKFLOW_SCOPE_TOOLS.includes(exec.name)) {
        noteDegradation(
          sessionId,
          GUARD_RULE_IDS.unknownScope,
          `${exec.name}: 本会话没有 alioth_workflow_* 调用记录，无法解析 {ns,app}——按未知范围放行（不猜 ns）`,
        )
      }
      return next()
    }
    if (enforceToolSurface && scope.stepId !== null) {
      const verdict = toolSurfaceVerdict(exec.name, scope.allowedTools)
      if (!verdict.ok) {
        ctx.logger.warn(`guard-alioth: 拒绝 ${exec.name}（工具面）`)
        return { kind: 'deny', reason: verdict.reason }
      }
    }
    if (writePath !== undefined) {
      const sandbox = sandboxVerdict(writePath, scope.namespace, preProcRoot)
      if (!sandbox.ok) {
        ctx.logger.warn(`guard-alioth: 拒绝 ${exec.name}（写沙箱）`)
        return { kind: 'deny', reason: sandbox.reason }
      }
      if (scope.stepId !== null && scope.phase === 'plan') {
        const plan = planWriteVerdict(writePath, scope.namespace, preProcRoot, scope.planWriteGlobs)
        if (!plan.ok) {
          ctx.logger.warn(`guard-alioth: 拒绝 ${exec.name}（plan 写面）`)
          return { kind: 'deny', reason: plan.reason }
        }
      }
    }
    return next()
  })

  // ── 执行后：修复墙 ────────────────────────────────────────────────────
  ctx.on('tools/post-execute', async (exec, result, next) => {
    if (exec.agent === undefined) {
      return next()
    }
    const sessionId = String(exec.agent.id)
    const budget = budgetFor(sessionId)
    budget.recordCall(exec.name, exec.arguments)
    if (result.isError) {
      budget.recordError(exec.name, result.error.message)
    }
    const decision = budget.decide()
    if (decision === 'allow') {
      return next()
    }
    // 错误签名墙只在失败结果上生效：修复墙不该拦下同一 turn 里成功的调用。
    if (!result.isError && decision !== 'terminate') {
      return next()
    }
    const target = await wallTarget(sessionId)
    const evidence = result.isError
      ? `${target}；原失败：${result.error.message}`
      : `${target}；触发：同一调用签名连续重复`
    const text = wallFeedback(decision, exec.name, evidence)
    ctx.logger.warn(`guard-alioth: 修复墙 ${decision}（${exec.name}）`)
    return { kind: 'block', feedback: [{ type: 'text', text }] }
  })

  // ── 执行前（步骤级）：turn 预算阻断与闭环证据追问 ────────────────────────
  ctx.on('agent/pre-step', async ({ agent, turn }, next) => {
    const sessionId = String(agent.id)
    const violation: TurnViolation | null = ledger.takeViolation(sessionId)
    if (violation !== null) {
      ctx.logger.warn(`guard-alioth: 阻断下一步（turn 预算）${violation.reason}`)
      return { kind: 'reject' }
    }
    const decision: NudgeDecision = decideClosureNudge(
      nudgeEnabled,
      ledger.turnRecord(sessionId, turn),
    )
    if (!decision.nudge) {
      return next()
    }
    const admitted = await next()
    if (admitted.kind !== 'enter') {
      return admitted
    }
    ledger.markNudged(sessionId, turn)
    ctx.logger.warn(`guard-alioth: 注入闭环证据追问（turn ${turn}）`)
    return {
      kind: 'enter',
      messages: [
        ...admitted.messages,
        createUserMessage({
          content: [{ type: 'text', text: decision.text }],
          source: { kind: 'plugin', plugin: name },
        }),
      ],
      ...admitted.startsRequestSeries === true ? { startsRequestSeries: true as const } : {},
    }
  })

  ctx.provide('aliothGuard', aliothGuard)
}

/** 会话事件里的工具参数是模型原始 JSON 字符串（未解析）；坏输入按「无参数」处理。 */
function parseArguments(raw: string): unknown {
  try {
    return JSON.parse(raw)
  } catch {
    return undefined
  }
}

export { SessionLedger, type SessionLedgerOptions, type TurnLedger, type TurnViolation } from './ledger.ts'
export { decideClosureNudge, CLOSURE_NUDGE_TEXT, type NudgeDecision, type NudgeView } from './nudge.ts'
export { GUARD_RULE_IDS, guardDenyReason, guardFeedback, type GuardRuleId } from './rules.ts'
export { RunScopeResolver, WORKFLOW_SCOPE_TOOLS, type ActiveScope, type ScopeKey } from './scope.ts'
export {
  declaredToolSurface,
  pathOrigin,
  planWriteVerdict,
  sandboxVerdict,
  toolSurfaceVerdict,
  writePathOf,
  META_TOOL_PREFIX,
  SANDBOX_WRITE_BLACKLIST,
  SANDBOX_ZONES,
  WRITE_TOOL_PATH_ARG,
  type Adjudication,
  type PathOrigin,
} from './surface.ts'
export {
  classifyRuntimeMirror,
  readWhitelistSource,
  RUNTIME_MIRROR_PATH,
  type AllowedProgramsSource,
  type WhitelistSourceReport,
} from './whitelist.ts'
