/**
 * `guard-alioth` 插件面规格：三条 harness 缝的真实行为（工具面 / 写沙箱 / plan 写面 /
 * 修复墙 / 闭环追问 / turn 阻断）与 `ctx.aliothGuard` 服务面。
 *
 * 环境边界用 `ctx.provide('aliothEnv', …)` 替身（只提供 `ready().modelDir` 与
 * `dataRoot()`）：守卫真正依赖的模型分发（adapter）与 run state 全部落在临时目录里按
 * repo 真实格式读写，因此判定路径是真的，只有环境服务这一层是替身。
 */

import { afterAll, beforeAll, describe, expect, it } from 'vitest'
import { mkdir, mkdtemp, rm, writeFile } from 'node:fs/promises'
import { tmpdir } from 'node:os'
import path from 'node:path'
import { Context } from '@deepseek-ai/cordis'
import SystemPrompt from '@deepseek-ai/dsh-system-prompt'
import ToolRuntime, { defineTool, type ToolExecutionResult } from '@deepseek-ai/dsh-tools'
import { ToolCallId } from '@deepseek-ai/dsh-llm'
import type { Agent } from '@deepseek-ai/dsh-agent'
import type { Session, SessionEvent, SessionId } from '@deepseek-ai/dsh-session'
import type { AliothEnv } from '@dsh-alioth/env-alioth'
import { ruleIdFromError } from '@dsh-alioth/skill-alioth'
import * as guard from '../src/index.ts'

const ADAPTER_YAML = `
name: alioth-app
description: "App 级原型集成"
version: "2.0"
default_tools: [todo]
tracks:
  - name: App 构建
    steps:
      - id: "1.1"
        instruction: "plan — 出方案"
        phase: plan
        tools: [read_file, write_file]
        gates:
          - output_glob: "Pre-Proc/{ns}/Apps/{app}/plans/*.json"
      - id: "1.2"
        instruction: "apply — 落地"
        tools: [read_file, write_file, run_command]
        gates:
          - output_glob: "Pre-Proc/{ns}/Apps/{app}/app.json"
`

/** 修复墙用的稳定错误文本（带规则码信封 ⇒ 错误签名按规则码聚合）。 */
const FAIL_TEXT = '[rule:tool-write-outside-sandbox] class=not-fixable · fixture 失败'

const signal = new AbortController().signal
let ctx: Context
const disposers: Array<() => Promise<void>> = []
let modelDir = ''
let dataRoot = ''
let preProcRoot = ''
let counter = 0

function callTool(toolName: string, args: unknown, agent?: Agent) {
  return ctx.tools.execute({
    signal,
    callId: ToolCallId(`guard-${++counter}`),
    name: toolName,
    arguments: args,
    ...(agent === undefined ? {} : { agent }),
  })
}

function fakeAgent(sessionId: string): Agent {
  const session = { id: sessionId as SessionId } as Session
  return { id: sessionId as SessionId, session } as unknown as Agent
}

let seq = 0

/** 用真实 `session/event` 火灾软管投递一条 durable 事件（`time` 即台账的时间口径）。 */
function emit(sessionId: string, type: string, data: unknown, time: number): void {
  seq += 1
  const event = { type, seq, time, data } as unknown as SessionEvent
  ctx.emit('session/event', { id: sessionId as SessionId } as Session, event)
}

function stepDecision(agent: Agent, turn: number) {
  return ctx.waterfall(
    'agent/pre-step',
    { agent, messages: [], turn, step: 1, signal },
    async () => ({ kind: 'enter' as const, messages: [] }),
  )
}

/** 注入进 `agent/pre-step` 的文本块（未注入 → 空数组）。 */
function injectedTexts(decision: unknown): string[] {
  const record = decision as { kind?: unknown; messages?: unknown }
  if (record.kind !== 'enter' || !Array.isArray(record.messages)) {
    return []
  }
  return record.messages.flatMap(message => {
    const blocks = (message as { content?: unknown }).content
    if (!Array.isArray(blocks)) {
      return []
    }
    return blocks.flatMap(block => {
      const item = block as { type?: unknown; text?: unknown }
      return item.type === 'text' && typeof item.text === 'string' ? [item.text] : []
    })
  })
}

/** 登记一条 fixture 工具：被拒绝的调用不会走到 body，故替身只需覆盖放行路径。 */
function registerStub(toolName: string, behaviour: () => void = () => {}): void {
  ctx.tools.register(defineTool({
    name: toolName,
    description: `fixture stub for ${toolName}`,
    parameters: {},
    output: {
      schema: { type: 'object', additionalProperties: true, properties: {} },
      render: () => [],
    },
    execute: async () => {
      behaviour()
      return {}
    },
  }))
}

function denyMessage(result: ToolExecutionResult): string {
  if (!result.isError) {
    throw new Error('expected an error result, got success')
  }
  return result.error.message
}

beforeAll(async () => {
  modelDir = await mkdtemp(path.join(tmpdir(), 'guard-model-'))
  dataRoot = await mkdtemp(path.join(tmpdir(), 'guard-data-'))
  preProcRoot = await mkdtemp(path.join(tmpdir(), 'guard-preproc-'))
  await mkdir(path.join(modelDir, 'skill-adapters'), { recursive: true })
  await writeFile(path.join(modelDir, 'skill-adapters', 'alioth-app.yaml'), ADAPTER_YAML)
  await writeFile(
    path.join(modelDir, 'skill-adapters', '_runtime.yaml'),
    'allowed_programs:\n  - bun\n  - cargo\n  - bash\n',
  )
  // run state：与 workspace.ts 同格式、同落点（<dataRoot>/workflows/{ns}/{app}/run-state.json）。
  const runs: Record<string, { trackIndex: number; stepIndex: number }> = {
    'plan-app': { trackIndex: 0, stepIndex: 0 },
    'apply-app': { trackIndex: 0, stepIndex: 1 },
    'finished-app': { trackIndex: 1, stepIndex: 0 },
  }
  for (const [app, position] of Object.entries(runs)) {
    const dir = path.join(dataRoot, 'workflows', 'Alioth', app)
    await mkdir(dir, { recursive: true })
    await writeFile(path.join(dir, 'run-state.json'), `${JSON.stringify({ position, completed: [] })}\n`)
  }
  // 损坏的 run state：loadRun 解析失败 → 守卫必须显式降级而不是静默跳过。
  const brokenDir = path.join(dataRoot, 'workflows', 'Alioth', 'broken-app')
  await mkdir(brokenDir, { recursive: true })
  await writeFile(path.join(brokenDir, 'run-state.json'), '{\n')

  ctx = new Context()
  ctx.provide('aliothEnv', {
    dataRoot: () => dataRoot,
    ready: async () => ({ modelDir, databaseUrl: '', sourceRef: '', modelVersion: '', bootstrap: {} }),
  } as unknown as AliothEnv)
  const system = await ctx.plugin(SystemPrompt)
  disposers.push(() => system.dispose())
  const tools = await ctx.plugin(ToolRuntime)
  disposers.push(() => tools.dispose())
  const plugin = await ctx.plugin(guard, {
    preProcRoot,
    dataRoot,
    turnTimeoutSec: 1,
    maxStepsPerTurn: 2,
  })
  disposers.push(() => plugin.dispose())

  // 组合部署会注册的 harness 工具面 + 本组元工具（替身）。
  for (const toolName of [
    'read', 'write', 'edit', 'bash',
    'alioth_workflow_step', 'alioth_workflow_complete', 'alioth_verify', 'alioth_closure',
    'alioth_app_write', 'present', 'ask_user_question',
  ]) {
    registerStub(toolName, toolName === 'bash' ? () => { throw new Error(FAIL_TEXT) } : () => ({}))
  }
  // 声明范围：本会话最近一次 workflow 调用决定 {ns, app}（run state 的键）。
  for (const [sessionId, app] of [
    ['surface-session', 'plan-app'],
    ['sandbox-session', 'apply-app'],
    ['plan-session', 'plan-app'],
    ['finished-session', 'finished-app'],
    ['broken-session', 'broken-app'],
    ['flip-session', 'flip-app'],
  ] as const) {
    emit(sessionId, 'tool/call', {
      turn: 1, step: 1, callId: `c-${sessionId}`, name: 'alioth_workflow_step',
      arguments: JSON.stringify({ namespace: 'Alioth', app }),
    }, 1)
  }
}, 60_000)

afterAll(async () => {
  for (const dispose of disposers.reverse()) {
    await dispose().catch(() => {})
  }
  await rm(modelDir, { recursive: true, force: true })
  await rm(dataRoot, { recursive: true, force: true })
  await rm(preProcRoot, { recursive: true, force: true })
})

describe('工具面强制', () => {
  it('拒绝面外调用并列出本步允许清单，放行声明面与元工具', async () => {
    const agent = fakeAgent('surface-session')
    const denied = denyMessage(await callTool('edit', { file_path: `${preProcRoot}/Alioth/Sources/x.ts` }, agent))
    expect(ruleIdFromError(denied)).toBe(guard.GUARD_RULE_IDS.toolSurface)
    expect(denied).toContain('本步允许：read, todo_write, write')

    const allowed = await callTool('write', { file_path: `${preProcRoot}/Alioth/Apps/plan-app/plans/p.json` }, agent)
    expect(allowed.isError).toBe(false)
    expect((await callTool('alioth_workflow_step', { namespace: 'Alioth', app: 'plan-app' }, agent)).isError).toBe(false)
  })

  it('范围已知但无当前步骤（运行已结束）→ 不按空清单禁掉一切，写沙箱仍生效', async () => {
    const agent = fakeAgent('finished-session')
    expect((await callTool('edit', { file_path: `${preProcRoot}/Alioth/Sources/x.ts` }, agent)).isError).toBe(false)
    const scope = await ctx.aliothGuard.activeScope('finished-session')
    expect(scope).toMatchObject({ namespace: 'Alioth', app: 'finished-app', stepId: null })
    const denied = denyMessage(await callTool('write', { file_path: '/etc/passwd' }, agent))
    expect(ruleIdFromError(denied)).toBe(guard.GUARD_RULE_IDS.writeSandbox)
  })

  it('未知范围（本会话无 workflow 调用）→ 放行并留降级证据，不猜 ns', async () => {
    const agent = fakeAgent('fresh-session')
    // 范围建立调用自身（参数带 {ns,app}）不产生降级噪声。
    expect((await callTool('alioth_workflow_step', { namespace: 'Alioth', app: 'plan-app' }, agent)).isError).toBe(false)
    expect(ctx.aliothGuard.degradations().filter(entry => entry.sessionId === 'fresh-session')).toEqual([])

    expect((await callTool('edit', { file_path: '/etc/passwd' }, agent)).isError).toBe(false)
    const evidence = ctx.aliothGuard.degradations().filter(entry => entry.sessionId === 'fresh-session')
    expect(evidence).toHaveLength(1)
    expect(evidence[0]?.ruleId).toBe(guard.GUARD_RULE_IDS.unknownScope)
    expect(evidence[0]?.reason).toContain('无法解析 {ns,app}')
  })
})

describe('写沙箱与 plan 写面', () => {
  it('apply 步：沙箱内放行，越界/穿越/黑名单拒绝', async () => {
    const agent = fakeAgent('sandbox-session')
    expect((await callTool('write', { file_path: `${preProcRoot}/Alioth/Sources/apps/main.ts` }, agent)).isError).toBe(false)
    expect((await callTool('write', { file_path: 'Pre-Proc/Alioth/AppAgentTraces/1.json' }, agent)).isError).toBe(false)

    // 拒绝结果同样是失败结果，会累积修复预算；每个独立场景开一个新 turn
    // （RetryBudget 的轮次作用域，与 turn 边界的重建口径一致）。
    let turn = 1
    const newTurn = (): void => {
      turn += 1
      emit('sandbox-session', 'turn/start', { turn }, 10 * turn)
    }

    newTurn()
    const outside = denyMessage(await callTool('write', { file_path: `${preProcRoot}/Alioth/Secrets/x.ts` }, agent))
    expect(ruleIdFromError(outside)).toBe(guard.GUARD_RULE_IDS.writeSandbox)
    newTurn()
    expect(denyMessage(await callTool('write', { file_path: `${preProcRoot}/Alioth/Sources/../etc/x` }, agent)))
      .toContain('拒绝穿越')
    newTurn()
    expect(denyMessage(await callTool('write', { file_path: 'Pre-Proc/Other/Sources/x.ts' }, agent)))
      .toContain('不在 Pre-Proc/Alioth/ 下')
    newTurn()
    expect(denyMessage(await callTool('write', { file_path: `${preProcRoot}/Alioth/Sources/autonomy.yaml` }, agent)))
      .toContain('写黑名单')
  })

  it('plan 步：只放行本步 output_glob 声明的方案产物', async () => {
    const agent = fakeAgent('plan-session')
    expect((await callTool('write', { file_path: `${preProcRoot}/Alioth/Apps/plan-app/plans/proposal.json` }, agent)).isError).toBe(false)
    expect((await callTool('write', { file_path: 'Pre-Proc/Alioth/Apps/plan-app/plans/x.json' }, agent)).isError).toBe(false)

    const denied = denyMessage(await callTool('write', { file_path: `${preProcRoot}/Alioth/Sources/main.ts` }, agent))
    expect(ruleIdFromError(denied)).toBe(guard.GUARD_RULE_IDS.planWriteScope)
    expect(denied).toContain('plan 步写面：Pre-Proc/Alioth/Apps/plan-app/plans/*.json')
  })

  it('每次判定现读 run state（无过期缓存）', async () => {
    const runFile = path.join(dataRoot, 'workflows', 'Alioth', 'flip-app', 'run-state.json')
    await mkdir(path.dirname(runFile), { recursive: true })
    await writeFile(runFile, `${JSON.stringify({ position: { trackIndex: 0, stepIndex: 0 }, completed: [] })}\n`)
    expect((await ctx.aliothGuard.activeScope('flip-session'))?.phase).toBe('plan')
    await writeFile(runFile, `${JSON.stringify({ position: { trackIndex: 0, stepIndex: 1 }, completed: ['1.1'] })}\n`)
    expect((await ctx.aliothGuard.activeScope('flip-session'))?.phase).toBe('apply')
  })

  it('run state 不可读 → 显式降级（跳过工具面/plan，沙箱仍按已知 namespace 生效）', async () => {
    const agent = fakeAgent('broken-session')
    const denied = denyMessage(await callTool('write', { file_path: '/etc/passwd' }, agent))
    expect(ruleIdFromError(denied)).toBe(guard.GUARD_RULE_IDS.writeSandbox)
    expect((await ctx.aliothGuard.activeScope('broken-session'))?.stepId).toBeNull()
    expect(ctx.aliothGuard.degradations()).toContainEqual(
      expect.objectContaining({
        sessionId: 'broken-session',
        ruleId: guard.GUARD_RULE_IDS.unknownScope,
      }),
    )
    expect(ctx.aliothGuard.degradations().some(entry => entry.reason.includes('Alioth/broken-app'))).toBe(true)
  })
})

describe('修复墙', () => {
  it('三档动作映射为 block 反馈（裁剪重试 → 转人工 → 死循环）', async () => {
    const agent = fakeAgent('wall-session')
    expect((await callTool('bash', { command: 'a' }, agent)).isError).toBe(true)
    // 同一错误签名第 2 次 → 裁剪重试。
    const trimmed = denyMessage(await callTool('bash', { command: 'b' }, agent))
    expect(ruleIdFromError(trimmed)).toBe(guard.GUARD_RULE_IDS.repairWall)
    expect(trimmed).toContain('上下文已裁剪')
    // 第 3 次 → 修复墙：转人工并列出未完成项。
    const escalated = denyMessage(await callTool('bash', { command: 'c' }, agent))
    expect(escalated).toContain('已达修复墙')
    expect(escalated).toContain('未完成项')
  })

  it('同一调用签名连续重复 → 死循环终止', async () => {
    const agent = fakeAgent('loop-session')
    await callTool('bash', { command: 'same' }, agent)
    await callTool('bash', { command: 'same' }, agent)
    const terminated = denyMessage(await callTool('bash', { command: 'same' }, agent))
    expect(ruleIdFromError(terminated)).toBe(guard.GUARD_RULE_IDS.repairWall)
    expect(terminated).toContain('死循环')
  })
})

describe('闭环证据追问', () => {
  it('产物调用而无证据 → 追问一次；再判不重复', async () => {
    const agent = fakeAgent('nudge-session')
    emit('nudge-session', 'turn/start', { turn: 1 }, 1_000)
    emit('nudge-session', 'assistant/message', {
      turn: 1, step: 1, message: {}, stream: [],
    }, 1_100)
    expect(injectedTexts(await stepDecision(agent, 1))).toEqual([])
    emit('nudge-session', 'tool/call', {
      turn: 1, step: 1, callId: 'n1', name: 'write', arguments: JSON.stringify({ file_path: 'x' }),
    }, 1_200)
    const nudged = injectedTexts(await stepDecision(agent, 1))
    expect(nudged).toHaveLength(1)
    expect(nudged[0]?.startsWith(`[rule:${guard.GUARD_RULE_IDS.closureNudge}]`)).toBe(true)
    expect(injectedTexts(await stepDecision(agent, 1))).toEqual([])
  })

  it('与澄清提问 / 终态汇报 / 已有证据三条路径互斥', async () => {
    const cases: Array<[string, string, number]> = [
      ['nudge-ask', 'ask_user_question', 2],
      ['nudge-present', 'present', 3],
      ['nudge-evidence', 'alioth_verify', 4],
    ]
    for (const [sessionId, toolName, turn] of cases) {
      const agent = fakeAgent(sessionId)
      emit(sessionId, 'turn/start', { turn }, 2_000)
      emit(sessionId, 'tool/call', {
        turn, step: 1, callId: `${sessionId}-a`, name: 'write', arguments: '{}',
      }, 2_100)
      emit(sessionId, 'tool/call', {
        turn, step: 1, callId: `${sessionId}-b`, name: toolName, arguments: '{}',
      }, 2_200)
      expect(injectedTexts(await stepDecision(agent, turn))).toEqual([])
    }
  })
})

describe('turn 预算', () => {
  it('墙钟超限 → 阻断下一步一次（原因带规则码前缀）', async () => {
    const agent = fakeAgent('budget-session')
    emit('budget-session', 'turn/start', { turn: 1 }, 10_000)
    emit('budget-session', 'turn/end', { turn: 1, reason: { kind: 'completed' } }, 12_000)
    const blocked = await stepDecision(agent, 2)
    expect(blocked.kind).toBe('reject')
    expect((await stepDecision(agent, 2)).kind).toBe('enter')
  })

  it('步数超限 → 同样阻断下一步', async () => {
    const agent = fakeAgent('steps-session')
    emit('steps-session', 'turn/start', { turn: 1 }, 0)
    for (const step of [1, 2, 3]) {
      emit('steps-session', 'step/start', { turn: 1, step }, step)
    }
    emit('steps-session', 'turn/end', { turn: 1, reason: { kind: 'completed' } }, 10)
    expect((await stepDecision(agent, 2)).kind).toBe('reject')
  })
})

describe('服务面', () => {
  it('whitelistSource 走模型分发的 _runtime.yaml（可读 → file）', async () => {
    const report = await ctx.aliothGuard.whitelistSource()
    expect(report.source).toBe('file')
    expect(report.reason).toContain('skill-adapters/_runtime.yaml')
    expect(report.programs).toContain('bun')
  })

  it('usage 汇总本会话模型调用（成本口径不可得时显式 unavailable）', () => {
    emit('usage-session', 'turn/start', { turn: 1 }, 1_000)
    emit('usage-session', 'step/start', { turn: 1, step: 1 }, 1_000)
    emit('usage-session', 'assistant/message', {
      turn: 1,
      step: 1,
      message: { source: { kind: 'model', provider: 'fixture', model: 'deepseek-chat' } },
      stream: [],
      usage: { inputTokens: 100, outputTokens: 20, totalTokens: 120 },
    }, 1_500)
    emit('usage-session', 'turn/end', { turn: 1, reason: { kind: 'completed' } }, 1_600)
    const summary = ctx.aliothGuard.usage('usage-session')
    expect(summary.total).toEqual({ tokensIn: 100, tokensOut: 20, calls: 1 })
    expect(summary.byModel['deepseek-chat']).toEqual({ tokensIn: 100, tokensOut: 20, calls: 1 })
    expect(summary.turns).toEqual([{ turn: 1, wallMs: 500, calls: 1 }])
    expect(summary.cost.kind).toBe('unavailable')
  })
})
