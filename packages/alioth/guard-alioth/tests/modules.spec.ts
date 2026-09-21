/**
 * `guard-alioth` 门禁与拒绝面的纯逻辑规格（无 harness 依赖）：
 * 路径归属 / 写沙箱 / plan 写面 / 工具面判定、闭环追问判定、台账（用量与 turn 预算）、
 * 白名单生效来源（含镜像读取路径）。
 */

import { afterAll, beforeAll, describe, expect, it } from 'vitest'
import { mkdir, mkdtemp, rm, writeFile } from 'node:fs/promises'
import { tmpdir } from 'node:os'
import path from 'node:path'
import type { SessionEvent } from '@deepseek-ai/dsh-session'
import { ToolCallId } from '@deepseek-ai/dsh-llm'
import { parseAdapterDocument, ruleIdFromError } from '@dsh-alioth/skill-alioth'
import { SessionLedger } from '../src/ledger.ts'
import { decideClosureNudge, CLOSURE_NUDGE_TEXT } from '../src/nudge.ts'
import { GUARD_RULE_IDS } from '../src/rules.ts'
import {
  declaredToolSurface,
  pathOrigin,
  planWriteVerdict,
  sandboxVerdict,
  toolSurfaceVerdict,
  writePathOf,
  type Adjudication,
} from '../src/surface.ts'
import { classifyRuntimeMirror, readWhitelistSource } from '../src/whitelist.ts'

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

const ADAPTER = parseAdapterDocument(ADAPTER_YAML, 'alioth-app.yaml')

/** 断言失败判定并返回可直接断言的拒绝文本。 */
function denial(adjudication: Adjudication): string {
  if (adjudication.ok) {
    throw new Error('expected a denial, got an allow')
  }
  return adjudication.reason
}

describe('surface: 工具面判定', () => {
  it('声明工具面 = 步骤 tools ∪ default_tools 的 harness 映射并集', () => {
    expect(declaredToolSurface(ADAPTER, ADAPTER.tracks[0]?.steps[1])).toEqual(['bash', 'read', 'todo_write', 'write'])
    expect(declaredToolSurface(ADAPTER, undefined)).toEqual(['todo_write'])
  })

  it('放行声明面与元工具，拒绝面外调用并列出本步允许清单', () => {
    const allowed = declaredToolSurface(ADAPTER, ADAPTER.tracks[0]?.steps[0])
    expect(toolSurfaceVerdict('write', allowed).ok).toBe(true)
    expect(toolSurfaceVerdict('alioth_workflow_step', []).ok).toBe(true)
    const reason = denial(toolSurfaceVerdict('bash', allowed))
    expect(ruleIdFromError(reason)).toBe(GUARD_RULE_IDS.toolSurface)
    expect(reason).toContain('本步允许：read, todo_write, write')
  })

  it('无当前步骤（空声明面）时面外调用同样被列出为空清单', () => {
    const reason = denial(toolSurfaceVerdict('bash', []))
    expect(reason).toContain('本步未声明任何 harness 工具')
  })

  it('写类工具 → 路径参数名（read/bash 不做路径推断）', () => {
    expect(writePathOf('write', { file_path: 'a.ts' })).toBe('a.ts')
    expect(writePathOf('edit', { file_path: 'a.ts' })).toBe('a.ts')
    expect(writePathOf('str_replace_editor', { path: 'a.ts' })).toBe('a.ts')
    expect(writePathOf('read', { file_path: 'a.ts' })).toBeUndefined()
    expect(writePathOf('bash', { command: 'rm -rf /Pre-Proc/Alioth' })).toBeUndefined()
    expect(writePathOf('write', { file_path: 42 })).toBeUndefined()
  })
})

describe('surface: 写沙箱', () => {
  const root = '/srv/alioth/Pre-Proc'

  it('允许沙箱内的四个区（绝对与相对两种口径）', () => {
    for (const zone of ['Sources', 'Prototypes', 'Apps', 'AppAgentTraces']) {
      expect(sandboxVerdict(`${root}/Alioth/${zone}/x.ts`, 'Alioth', root).ok).toBe(true)
      expect(sandboxVerdict(`Pre-Proc/Alioth/${zone}/x.ts`, 'Alioth', root).ok).toBe(true)
    }
  })

  it('拒绝区外目录、越界路径、跨 namespace 与写黑名单', () => {
    const outsideZone = denial(sandboxVerdict(`${root}/Alioth/Secrets/x.ts`, 'Alioth', root))
    expect(ruleIdFromError(outsideZone)).toBe(GUARD_RULE_IDS.writeSandbox)
    expect(outsideZone).toContain('写面限于 Pre-Proc/Alioth/{Sources,Prototypes,Apps,AppAgentTraces}/')
    expect(denial(sandboxVerdict(`${root}/Alioth/Sources/../etc/passwd`, 'Alioth', root))).toContain('拒绝穿越')
    expect(denial(sandboxVerdict('Pre-Proc/Alioth/./Sources/x.ts', 'Alioth', root))).toContain('拒绝穿越')
    expect(denial(sandboxVerdict(`${root}/Other/Sources/x.ts`, 'Alioth', root))).toContain('不在 Pre-Proc/Alioth/ 下')
    expect(denial(sandboxVerdict('Pre-Proc/Other/Sources/x.ts', 'Alioth', root))).toContain('不在 Pre-Proc/Alioth/ 下')
    expect(denial(sandboxVerdict(`${root}/Alioth/Sources/autonomy.yaml`, 'Alioth', root))).toContain('写黑名单')
  })

  it('拒绝配置根之外的绝对路径（同形状但不属本部署沙箱）', () => {
    expect(denial(sandboxVerdict('/tmp/elsewhere/Pre-Proc/Alioth/Sources/x.ts', 'Alioth', root)))
      .toContain('不在 Pre-Proc/Alioth/ 下')
  })

  it('路径归属：给出 adapter 口径的相对路径供 glob 匹配', () => {
    const origin = pathOrigin(`${root}/Alioth/Apps/demo/app.json`, 'Alioth', root)
    expect(origin).toEqual({ ok: true, relative: 'Pre-Proc/Alioth/Apps/demo/app.json', segments: ['Apps', 'demo', 'app.json'] })
    const shallow = pathOrigin('Pre-Proc/Alioth/x.ts', 'Alioth', root)
    expect(shallow.ok && shallow.segments).toEqual(['x.ts'])
  })
})

describe('surface: plan 写面收窄', () => {
  const root = '/srv/alioth/Pre-Proc'
  const globs = ['Pre-Proc/Alioth/Apps/demo/plans/*.json']

  it('命中本步 output_glob → 放行（绝对与相对目标均可）', () => {
    expect(planWriteVerdict(`${root}/Alioth/Apps/demo/plans/proposal.json`, 'Alioth', root, globs).ok).toBe(true)
    expect(planWriteVerdict('Pre-Proc/Alioth/Apps/demo/plans/x.json', 'Alioth', root, globs).ok).toBe(true)
  })

  it('沙箱内但不在方案产物位 → 拒绝并列出本步写面', () => {
    const reason = denial(planWriteVerdict(`${root}/Alioth/Sources/main.ts`, 'Alioth', root, globs))
    expect(ruleIdFromError(reason)).toBe(GUARD_RULE_IDS.planWriteScope)
    expect(reason).toContain('plan 步写面：Pre-Proc/Alioth/Apps/demo/plans/*.json')
  })

  it('plan 步未声明 output_glob → fail-closed', () => {
    expect(denial(planWriteVerdict(`${root}/Alioth/Apps/demo/plans/x.json`, 'Alioth', root, [])))
      .toContain('本步未声明 output_glob')
  })
})

describe('nudge: 闭环证据追问判定', () => {
  const base = {
    turn: 3,
    artifactCalls: 1,
    evidenceCalls: 0,
    clarification: false,
    terminalReport: false,
    nudged: false,
  }

  it('产物调用而无闭环证据 → 追问（文本带规则码前缀）', () => {
    const decision = decideClosureNudge(true, base)
    expect(decision.nudge).toBe(true)
    expect(decision.nudge && decision.text).toBe(CLOSURE_NUDGE_TEXT)
    expect(decision.nudge && decision.text.startsWith(`[rule:${GUARD_RULE_IDS.closureNudge}]`)).toBe(true)
  })

  it('上限 1 次/turn 与三条互斥/无谓路径各自给出不同原因', () => {
    const reasons = [
      decideClosureNudge(true, { ...base, nudged: true }),
      decideClosureNudge(true, { ...base, clarification: true }),
      decideClosureNudge(true, { ...base, terminalReport: true }),
      decideClosureNudge(true, { ...base, evidenceCalls: 1 }),
      decideClosureNudge(true, { ...base, artifactCalls: 0 }),
      decideClosureNudge(false, base),
      decideClosureNudge(true, null),
    ]
    for (const decision of reasons) {
      expect(decision.nudge).toBe(false)
    }
    const texts = reasons.map(decision => (decision.nudge ? '' : decision.reason))
    expect(new Set(texts).size).toBe(texts.length)
    expect(texts[0]).toContain('上限 1 次/turn')
  })
})

describe('ledger: 用量与 turn 预算', () => {
  let seq = 0
  function event<T extends SessionEvent['type']>(
    type: T,
    time: number,
    data: Extract<SessionEvent, { type: T }>['data'],
  ): SessionEvent {
    seq += 1
    return { type, seq, time, data } as SessionEvent
  }

  it('按会话聚合模型用量（成本口径不可得时显式 unavailable）', () => {
    const ledger = new SessionLedger()
    ledger.observe('s1', event('turn/start', 1000, { turn: 1 }))
    ledger.observe('s1', event('step/start', 1000, { turn: 1, step: 1 }))
    ledger.observe('s1', event('assistant/message', 1500, {
      turn: 1,
      step: 1,
      message: { source: { kind: 'model', provider: 'p', model: 'deepseek-chat' } } as never,
      stream: [],
      usage: { inputTokens: 100, outputTokens: 20, totalTokens: 120 },
    }))
    ledger.observe('s1', event('turn/end', 4000, { turn: 1, reason: { kind: 'completed' } }))

    const summary = ledger.usage('s1')
    expect(summary.total).toEqual({ tokensIn: 100, tokensOut: 20, calls: 1 })
    expect(summary.byModel['deepseek-chat']).toEqual({ tokensIn: 100, tokensOut: 20, calls: 1 })
    expect(summary.turns).toEqual([{ turn: 1, wallMs: 500, calls: 1 }])
    expect(summary.cost.kind).toBe('unavailable')
    expect(ledger.usage('s2').total).toEqual({ tokensIn: 0, tokensOut: 0, calls: 0 })
  })

  it('按 turn 台账统计产物/证据/澄清/终态调用', () => {
    const ledger = new SessionLedger()
    ledger.observe('s1', event('turn/start', 0, { turn: 7 }))
    ledger.observe('s1', event('tool/call', 1, { turn: 7, step: 1, name: 'write', arguments: '{}', callId: ToolCallId('c-write') }))
    ledger.observe('s1', event('tool/call', 2, { turn: 7, step: 1, name: 'alioth_app_write', arguments: '{}', callId: ToolCallId('c-app') }))
    ledger.observe('s1', event('tool/call', 3, { turn: 7, step: 1, name: 'alioth_verify', arguments: '{}', callId: ToolCallId('c-verify') }))
    ledger.observe('s1', event('tool/call', 4, { turn: 7, step: 1, name: 'ask_user_question', arguments: '{}', callId: ToolCallId('c-ask') }))
    ledger.observe('s1', event('tool/call', 5, { turn: 7, step: 1, name: 'present', arguments: '{}', callId: ToolCallId('c-present') }))
    expect(ledger.turnRecord('s1', 7)).toMatchObject({
      turn: 7,
      calls: 5,
      artifactCalls: 2,
      evidenceCalls: 1,
      clarification: true,
      terminalReport: true,
      nudged: false,
      closed: false,
    })
    // 别的 turn 没有台账：判定不得串轮。
    expect(ledger.turnRecord('s1', 8)).toBeNull()
    ledger.markNudged('s1', 8)
    expect(ledger.turnRecord('s1', 7)?.nudged).toBe(false)
  })

  it('turn/end 超墙钟即记一条违规，消费一次后清空', () => {
    const ledger = new SessionLedger({ turnTimeoutSec: 1, maxStepsPerTurn: 2 })
    ledger.observe('s1', event('turn/start', 10_000, { turn: 1 }))
    ledger.observe('s1', event('step/start', 10_000, { turn: 1, step: 1 }))
    ledger.observe('s1', event('turn/end', 12_500, { turn: 1, reason: { kind: 'completed' } }))
    const violation = ledger.takeViolation('s1')
    expect(violation).not.toBeNull()
    expect(violation?.wallMs).toBe(2500)
    expect(ruleIdFromError(violation?.reason ?? '')).toBe(GUARD_RULE_IDS.turnBudget)
    expect(violation?.reason).toContain('超过 turnTimeoutSec=1s')
    expect(ledger.takeViolation('s1')).toBeNull()
  })

  it('步数超限同样记违规，且未超限的 turn 不记', () => {
    const ledger = new SessionLedger({ maxStepsPerTurn: 2 })
    ledger.observe('s1', event('turn/start', 0, { turn: 1 }))
    for (const step of [1, 2, 3]) {
      ledger.observe('s1', event('step/start', step, { turn: 1, step }))
    }
    ledger.observe('s1', event('turn/end', 10, { turn: 1, reason: { kind: 'completed' } }))
    const violation = ledger.takeViolation('s1')
    expect(violation?.steps).toBe(3)
    expect(violation?.reason).toContain('maxStepsPerTurn=2')

    ledger.observe('s2', event('turn/start', 0, { turn: 1 }))
    ledger.observe('s2', event('step/start', 0, { turn: 1, step: 1 }))
    ledger.observe('s2', event('turn/end', 10, { turn: 1, reason: { kind: 'completed' } }))
    expect(ledger.takeViolation('s2')).toBeNull()
  })

  it('配了价表与成本上限时，超限的 turn 记违规；未配价表则不判', () => {
    const prices = { 'deepseek-chat': { centsPerInK: 100, centsPerOutK: 200 } }
    const capped = new SessionLedger({ maxTurnCostCents: 5, prices })
    capped.observe('s1', event('turn/start', 0, { turn: 1 }))
    capped.observe('s1', event('step/start', 0, { turn: 1, step: 1 }))
    capped.observe('s1', event('assistant/message', 10, {
      turn: 1,
      step: 1,
      message: { source: { kind: 'model', provider: 'p', model: 'deepseek-chat' } } as never,
      stream: [],
      usage: { inputTokens: 100, outputTokens: 100, totalTokens: 200 },
    }))
    capped.observe('s1', event('turn/end', 20, { turn: 1, reason: { kind: 'completed' } }))
    const violation = capped.takeViolation('s1')
    // 100/1000*100 + 100/1000*200 = 10 + 20 = 30 分 > 5 分上限
    expect(violation?.reason).toContain('30 分超过 maxTurnCostCents=5 分')
    expect(ruleIdFromError(violation?.reason ?? '')).toBe(GUARD_RULE_IDS.turnCost)

    // 同一份事件流，未配价表 → 成本口径不可得，上限不判（不得以 0 冒充成本）。
    const uncapped = new SessionLedger({ maxTurnCostCents: 5 })
    uncapped.observe('s1', event('turn/start', 0, { turn: 1 }))
    uncapped.observe('s1', event('assistant/message', 10, {
      turn: 1,
      step: 1,
      message: { source: { kind: 'model', provider: 'p', model: 'deepseek-chat' } } as never,
      stream: [],
      usage: { inputTokens: 100, outputTokens: 100, totalTokens: 200 },
    }))
    uncapped.observe('s1', event('turn/end', 20, { turn: 1, reason: { kind: 'completed' } }))
    expect(uncapped.takeViolation('s1')).toBeNull()
    expect(uncapped.usage('s1').cost.kind).toBe('unavailable')
  })
})

describe('whitelist: 生效来源可区分', () => {
  let modelDir = ''

  beforeAll(async () => {
    modelDir = await mkdtemp(path.join(tmpdir(), 'guard-whitelist-'))
    await mkdir(path.join(modelDir, 'skill-adapters'), { recursive: true })
  })

  afterAll(async () => {
    await rm(modelDir, { recursive: true, force: true })
  })

  it('镜像缺失 → code_default（原因：缺失或不可读）', async () => {
    const report = await readWhitelistSource(modelDir)
    expect(report.source).toBe('code_default')
    expect(report.reason).toContain('缺失或不可读')
    expect(report.programs.length).toBeGreaterThan(0)
  })

  it('镜像可读且清单非空 → file', async () => {
    await writeFile(path.join(modelDir, 'skill-adapters', '_runtime.yaml'), 'allowed_programs:\n  - bun\n  - cargo\n')
    const report = await readWhitelistSource(modelDir)
    expect(report.source).toBe('file')
    expect(report.reason).toContain('skill-adapters/_runtime.yaml')
    expect(report.programs).toEqual(['bun', 'cargo'])
  })

  it('清单为空 → code_default（原因：为空），且与解析失败可区分', async () => {
    const empty = classifyRuntimeMirror({ content: 'allowed_programs: []\n' })
    const broken = classifyRuntimeMirror({ content: 'allowed_programs: [\n' })
    const missing = classifyRuntimeMirror({ error: 'ENOENT' })
    expect(empty.source).toBe('code_default')
    expect(empty.reason).toContain('为空')
    expect(broken.source).toBe('code_default')
    expect(broken.reason).toContain('解析失败')
    expect(new Set([empty.reason, broken.reason, missing.reason]).size).toBe(3)
  })
})
