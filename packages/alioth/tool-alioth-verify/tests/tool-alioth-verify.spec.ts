/**
 * T5 规格测试：模型面验证工具的行为/边界/不变量。
 *
 * 覆盖（对齐切片验收）：
 * - `confirmed !== true` 拒绝 `alioth_version rollback` 与 `alioth_patch_assets apply`，且**不写盘**
 *   （断言字节级不变）；
 * - 降级扩展验证 MUST NOT 报通过（degraded ≠ passed，且降级报告不写 canonical 位）；
 * - 能力广告六组字段恒在场，缺源显式 `unknown(reason)`（守卫缺失 / 工具面 / 漂移基线）；
 * - stage 门禁对缺失产物诚实失败（带 `[rule:<id>]` 证据），产物齐备后通过。
 * @module @dsh-alioth/tool-alioth-verify/tests
 */

import { createHash } from 'node:crypto'
import { mkdir, mkdtemp, readFile, rm, writeFile } from 'node:fs/promises'
import { tmpdir } from 'node:os'
import path from 'node:path'
import { afterAll, beforeAll, describe, expect, it } from 'vitest'
import { Context } from '@deepseek-ai/cordis'
import SystemPrompt from '@deepseek-ai/dsh-system-prompt'
import ToolRuntime from '@deepseek-ai/dsh-tools'
import type { ToolExecutionInput, ToolExecutionResult } from '@deepseek-ai/dsh-tools'
import { ToolCallId } from '@deepseek-ai/dsh-llm'
import { aggregateUsage } from '@dsh-alioth/verify-alioth'
import * as verifyTools from '../src/index.ts'

const signal = new AbortController().signal

/** fixture 适配器：首步为 plan 相位（能力广告的 plansPending 判据），次步为 apply。 */
const ADAPTER_YAML = `
name: alioth-app
description: "fixture"
version: "2.2"
tracks:
  - name: App 构建
    steps:
      - id: "1.1"
        instruction: "preflight — 方案步"
        tools: [read_file]
        phase: plan
        schema: {type: object, required: [ns, app]}
        gates:
          - output_glob: "Pre-Proc/{ns}/Apps/{app}/app.json"
      - id: "1.2"
        instruction: "落地"
        tools: [write_file]
        schema: {type: object, required: [ns, app]}
        gates:
          - output_glob: "Pre-Proc/{ns}/Apps/{app}/app.json"
`

const APP_JSON = `${JSON.stringify({
  id: 'demo-app',
  code: 'demo-app',
  namespace: 'Alioth',
  name: 'Demo App',
  version: '1.0.0',
  status: 'developing',
}, null, 2)}\n`

const PROTO_HTML = '<!doctype html><html><body><h1>demo</h1></body></html>\n'

/** 加载器认可的 constraint 声明（必需键 entity/expression/message 齐备）。 */
const CONSTRAINTS_YAML = `
- entity: zc_id_inventory
  expression: "qty >= 0"
  message: "库存不得为负"
`

/** 加载器不认的文件名：永不加载 = 未覆盖（声明从未进入运行时）。 */
const UNKNOWN_DECL_YAML = 'name: never-loaded\n'

const MODEL = 'deepseek-flash'

let ctx: Context
let bareCtx: Context
const disposers: Array<() => Promise<void>> = []
let preProcRoot = ''
let dataRoot = ''
let modelDir = ''
let bareRoot = ''
let priceTablePath = ''
let counter = 0

function appDir(): string {
  return path.join(preProcRoot, 'Alioth', 'Apps', 'demo-app')
}

function callTool(target: Context, toolName: string, args: unknown, sessionId?: string) {
  return target.tools.execute({
    signal,
    callId: ToolCallId(`verify-${++counter}`),
    name: toolName,
    arguments: args,
    ...(sessionId === undefined
      ? {}
      : { agent: { id: sessionId } as unknown as NonNullable<ToolExecutionInput['agent']> }),
  })
}

function expectOk(result: ToolExecutionResult): Record<string, unknown> {
  if (result.isError) throw new Error(`expected success, got: ${result.error.message}`)
  return result.value as Record<string, unknown>
}

function expectError(result: ToolExecutionResult): string {
  if (!result.isError) throw new Error('expected an error result, got success')
  return result.error.message
}

/** 一条 app 级人工门（`alioth_deferred list` 的可见面）。 */
interface AppGate {
  readonly id: string
  readonly sessionId: string
  readonly reason: string
  readonly adjudication: string
  readonly trigger: { readonly kind: string; readonly path: string; readonly pointer?: string; readonly equals?: unknown }
  readonly successors: readonly string[]
}

/** 读当前登记的全部 app 级人工门（不带 session 的 list，验证「门与调用方会话无关」）。 */
async function readAppGates(): Promise<AppGate[]> {
  const value = expectOk(await callTool(ctx, 'alioth_deferred', { action: 'list' }))
  return (value.items as AppGate[]).filter(item => item.sessionId.startsWith('app-extensions-'))
}

/** 重置 App 产物目录为「健康」基线（app.json + prototype.html + 一条合法 constraint 声明）。 */
async function resetApp(): Promise<void> {
  await rm(appDir(), { recursive: true, force: true })
  await mkdir(path.join(appDir(), 'extensions'), { recursive: true })
  await writeFile(path.join(appDir(), 'app.json'), APP_JSON)
  await writeFile(path.join(appDir(), 'prototype.html'), PROTO_HTML)
  await writeFile(path.join(appDir(), 'extensions', 'constraints.yaml'), CONSTRAINTS_YAML)
}

const guardStub = {
  whitelistSource: async () => ({ source: 'file', reason: 'fixture 运行时镜像', programs: ['bun', 'cargo'] }),
  degradations: () => [{ sessionId: 'guard-session', ruleId: 'unknown-scope', reason: 'fixture 降级证据', time: 1 }],
  usage: () => aggregateUsage(
    [{ step: 1, model: MODEL, tokensIn: 1000, tokensOut: 500, latencyMs: 250, turn: 1 }],
    undefined,
  ),
}

async function makeContext(options: { readonly guard: boolean; readonly root: string }): Promise<Context> {
  const context = new Context()
  context.provide('aliothEnv', {
    dataRoot: () => options.root,
    ready: async () => ({ modelDir, databaseUrl: '', sourceRef: '', modelVersion: '', bootstrap: {} }),
  } as never)
  const system = await context.plugin(SystemPrompt)
  disposers.push(() => system.dispose())
  const tools = await context.plugin(ToolRuntime)
  disposers.push(() => tools.dispose())
  if (options.guard) {
    context.provide('aliothGuard', guardStub as never)
    // llm 服务替身：只有 listProviders() 是可广告的就绪性来源。
    context.provide('llm', { listProviders: () => [] } as never)
  }
  const plugin = await context.plugin(verifyTools, { preProcRoot, priceTable: priceTablePath })
  disposers.push(() => plugin.dispose())
  return context
}

beforeAll(async () => {
  preProcRoot = await mkdtemp(path.join(tmpdir(), 'verify-preproc-'))
  dataRoot = await mkdtemp(path.join(tmpdir(), 'verify-data-'))
  bareRoot = await mkdtemp(path.join(tmpdir(), 'verify-bare-'))
  modelDir = await mkdtemp(path.join(tmpdir(), 'verify-model-'))
  priceTablePath = path.join(await mkdtemp(path.join(tmpdir(), 'verify-price-')), 'prices.json')
  await writeFile(priceTablePath, `${JSON.stringify({ [MODEL]: { centsPerInK: 1, centsPerOutK: 2 } }, null, 2)}\n`)

  // 模型快照：适配器 + 漂移基线（PROVENANCE.json 的 sha256 即字节指纹）。
  await mkdir(path.join(modelDir, 'skill-adapters'), { recursive: true })
  await writeFile(path.join(modelDir, 'skill-adapters', 'alioth-app.yaml'), ADAPTER_YAML)
  await writeFile(path.join(modelDir, 'skill-adapters', '_runtime.yaml'), 'allowed_programs:\n  - bun\n')
  const adapterSha = createHash('sha256').update(ADAPTER_YAML).digest('hex')
  const runtimeSha = createHash('sha256').update('allowed_programs:\n  - bun\n').digest('hex')
  await writeFile(path.join(modelDir, 'PROVENANCE.json'), `${JSON.stringify({
    files: {
      'skill-adapters/alioth-app.yaml': adapterSha,
      'skill-adapters/_runtime.yaml': runtimeSha,
    },
  }, null, 2)}\n`)

  // run state：当前步 = 首步（plan 相位，未完成）→ plansPending = 1。
  const runDir = path.join(dataRoot, 'workflows', 'Alioth', 'demo-app')
  await mkdir(runDir, { recursive: true })
  await writeFile(path.join(runDir, 'run-state.json'), `${JSON.stringify({ position: { trackIndex: 0, stepIndex: 0 }, completed: [] })}\n`)

  await resetApp()
  ctx = await makeContext({ guard: true, root: dataRoot })
  bareCtx = await makeContext({ guard: false, root: bareRoot })
}, 60_000)

afterAll(async () => {
  for (const dispose of disposers.reverse()) {
    await dispose().catch(() => {})
  }
  for (const dir of [preProcRoot, dataRoot, bareRoot, modelDir, path.dirname(priceTablePath)]) {
    await rm(dir, { recursive: true, force: true })
  }
})

describe('alioth_verify', () => {
  it('artifacts：健康产物可评估且落盘 eval-report.json', async () => {
    await resetApp()
    const value = expectOk(await callTool(ctx, 'alioth_verify', { action: 'artifacts', namespace: 'Alioth', app: 'demo-app' }))
    expect(value.action).toBe('artifacts')
    expect(value.passed).toBe(true)
    expect(String(value.reportPath)).toBe(path.join(appDir(), 'eval-report.json'))
    const report = JSON.parse(await readFile(String(value.reportPath), 'utf8')) as { passed: boolean }
    expect(report.passed).toBe(true)
  })

  it('artifacts：缺失 app.json 记 0 分并留 violation（缺失 ≠ 满分）', async () => {
    await resetApp()
    await rm(path.join(appDir(), 'app.json'))
    const value = expectOk(await callTool(ctx, 'alioth_verify', { action: 'artifacts', namespace: 'Alioth', app: 'demo-app' }))
    expect(value.passed).toBe(false)
    const report = value.report as { violations: { rule: string }[]; dimensions: Record<string, number> }
    expect(report.dimensions['schema_validity']).toBe(0)
    expect(report.violations.map(violation => violation.rule)).toContain('schema_validity')
  })

  it('extensions：声明齐备 → passed；出现加载器不认的文件名 → degraded（MUST NOT 报通过）', async () => {
    await resetApp()
    const healthy = expectOk(await callTool(ctx, 'alioth_verify', { action: 'extensions', namespace: 'Alioth', app: 'demo-app' }))
    expect(healthy.status).toBe('passed')
    expect(healthy.passed).toBe(true)
    expect(String(healthy.reportPath).endsWith('extension-verify.json')).toBe(true)

    await writeFile(path.join(appDir(), 'extensions', 'bogus.yaml'), UNKNOWN_DECL_YAML)
    const degraded = expectOk(await callTool(ctx, 'alioth_verify', { action: 'extensions', namespace: 'Alioth', app: 'demo-app' }))
    expect(degraded.status).toBe('degraded')
    expect(degraded.passed).toBe(false)
    // 降级不得占据 canonical 位（否则 publish 前置会读到一份「看起来通过」的证据）。
    expect(String(degraded.reportPath).endsWith('extension-verify.degraded.json')).toBe(true)
    // 陈旧 canonical 被作废：上一轮的 passed 不得为这一轮降级背书。
    expect(await readFile(path.join(appDir(), 'extension-verify.json'), 'utf8').catch(() => null)).toBeNull()
    // 降级必须留下一条按 app 的人工门（解除条件 = canonical `/status == "passed"`）。
    expect(degraded.gateRegistered).toBe(true)
    expect(degraded.gateScope).toBe('app-extensions-Alioth-demo-app')
    const gate = (await readAppGates()).find(entry => entry.id === 'extensions-degraded:Alioth/demo-app')
    expect(gate).toBeDefined()
    expect(gate?.trigger).toMatchObject({
      kind: 'artifact-json-pointer',
      pointer: '/status',
      equals: 'passed',
    })
    expect(gate?.adjudication.length).toBeGreaterThan(0)
    // 门按 app 作用域登记：任一其它会话 list 都能看到，不会因会话过滤而「看不见」。
    const fromOtherSession = expectOk(await callTool(ctx, 'alioth_deferred', {
      action: 'list',
      sessionId: 'some-other-session',
    }, 'some-other-session'))
    expect((fromOtherSession.items as AppGate[]).map(item => item.id)).toContain('extensions-degraded:Alioth/demo-app')
    expect(String(fromOtherSession.scope)).toContain('app-gates')

    const report = degraded.report as { uncovered: number; declarations: { status: string; evidence: string }[] }
    expect(report.uncovered).toBeGreaterThan(0)
    expect(report.declarations.some(entry => entry.status === 'uncovered' && entry.evidence.includes('永不加载'))).toBe(true)
    expect(JSON.stringify(degraded.allowedForms)).toContain('constraints')

    // 真实通过是唯一解除路径：修好声明后再跑 → 门解除。
    await resetApp()
    const cleared = expectOk(await callTool(ctx, 'alioth_verify', { action: 'extensions', namespace: 'Alioth', app: 'demo-app' }))
    expect(cleared.status).toBe('passed')
    expect(cleared.gatesCleared).toBe(1)
    expect((await readAppGates()).find(entry => entry.id === 'extensions-degraded:Alioth/demo-app')).toBeUndefined()
  })

  it('extensions：干跑（write:false）不登记门（证据面未被改动，门会立刻可解除）', async () => {
    await resetApp()
    await writeFile(path.join(appDir(), 'extensions', 'bogus.yaml'), UNKNOWN_DECL_YAML)
    const dry = expectOk(await callTool(ctx, 'alioth_verify', {
      action: 'extensions',
      namespace: 'Alioth',
      app: 'demo-app',
      write: false,
    }))
    expect(dry.status).toBe('degraded')
    expect(dry.reportPath).toBe('')
    expect(dry.gateRegistered).toBe(false)
    expect((await readAppGates()).find(entry => entry.id === 'extensions-degraded:Alioth/demo-app')).toBeUndefined()
  })

  it('extensions：顶层形状不符（数组期望）同样判未覆盖，且门登记幂等', async () => {
    await resetApp()
    await writeFile(path.join(appDir(), 'extensions', 'constraints.yaml'), 'entity: zc_id_inventory\n')
    const value = expectOk(await callTool(ctx, 'alioth_verify', { action: 'extensions', namespace: 'Alioth', app: 'demo-app' }))
    expect(value.status).toBe('degraded')
    const report = value.report as { declarations: { evidence: string }[] }
    expect(report.declarations[0]?.evidence).toContain('非数组')
    // 重复降级不堆积：门 id 稳定（同一 app 只在登记表里留一条）。
    expectOk(await callTool(ctx, 'alioth_verify', { action: 'extensions', namespace: 'Alioth', app: 'demo-app' }))
    const gates = (await readAppGates()).filter(entry => entry.id === 'extensions-degraded:Alioth/demo-app')
    expect(gates).toHaveLength(1)
    expect(gates[0]?.reason).toContain('未被 Gateway loader 覆盖')
  })

  it('stage：缺失产物诚实失败（带规则码证据），产物齐备后通过', async () => {
    await resetApp()
    const failed = expectError(
      await callTool(ctx, 'alioth_verify', { action: 'stage', namespace: 'Alioth', app: 'demo-app', stage: 'quality' }),
    )
    expect(failed).toContain('[rule:')
    expect(failed).toContain('eval-report.json')

    expectOk(await callTool(ctx, 'alioth_verify', { action: 'artifacts', namespace: 'Alioth', app: 'demo-app' }))
    const passed = expectOk(
      await callTool(ctx, 'alioth_verify', { action: 'stage', namespace: 'Alioth', app: 'demo-app', stage: 'quality' }),
    )
    expect(passed.ok).toBe(true)
    expect(passed.stage).toBe('quality')

    // appagent-ready 只认 pipeline_manifest.json / app.json。
    expect(expectOk(await callTool(ctx, 'alioth_verify', { action: 'stage', namespace: 'Alioth', app: 'demo-app', stage: 'appagent-ready' })).ok).toBe(true)
    expect(expectError(
      await callTool(ctx, 'alioth_verify', { action: 'stage', namespace: 'Alioth', app: 'demo-app', stage: 'ontology-mapping' }),
    )).toContain('ontology-output.json')
  })

  it('拒绝未知 action / 非法 stage / 非法 namespace（不猜、不放行）', async () => {
    expect(expectError(await callTool(ctx, 'alioth_verify', { action: 'nope', namespace: 'Alioth', app: 'demo-app' })))
      .toContain('invalid action')
    expect(expectError(await callTool(ctx, 'alioth_verify', { action: 'stage', namespace: 'Alioth', app: 'demo-app', stage: 'nope' })))
      .toContain('invalid stage')
    expect(expectError(await callTool(ctx, 'alioth_verify', { action: 'stage', namespace: 'lowercase', app: 'demo-app', stage: 'quality' })))
      .toContain('非法 namespace')
  })
})

describe('alioth_closure', () => {
  it('裁决 append-only 且指纹锁死「审计后偷改产物」', async () => {
    await resetApp()
    const first = expectOk(await callTool(ctx, 'alioth_closure', {
      action: 'verdict',
      namespace: 'Alioth',
      app: 'demo-app',
      verdict: 'approved',
      findings: [{ dimension: 'artifacts', verdict: 'pass', detail: '产物齐备' }],
      evidence: ['eval-report.json'],
    }))
    expect(first.verdict).toBe('approved')
    expect(first.seq).toBe(1)
    expect(first.escalated).toBe(false)
    expect(String(first.recordPath).endsWith(path.join('AppAgentTraces', 'closure-audit', '1.json'))).toBe(true)

    const status = expectOk(await callTool(ctx, 'alioth_closure', { action: 'status', namespace: 'Alioth', app: 'demo-app' }))
    expect(status.matched).toBe(true)
    expect(status.total).toBe(1)
    expect((status.matching as { verdict: string }).verdict).toBe('approved')

    // 审计后改产物：旧裁决必须失效（指纹不再匹配）。
    await writeFile(path.join(appDir(), 'app.json'), `${APP_JSON.replace('Demo App', 'Demo App v2')}`)
    const after = expectOk(await callTool(ctx, 'alioth_closure', { action: 'status', namespace: 'Alioth', app: 'demo-app' }))
    expect(after.matched).toBe(false)
    expect(after.total).toBe(1)

    // 连续 rejected 达阈值 → 升为 escalate（转人工，禁止同构重试）。
    for (const expected of ['rejected', 'rejected', 'escalate']) {
      const record = expectOk(await callTool(ctx, 'alioth_closure', {
        action: 'verdict',
        namespace: 'Alioth',
        app: 'demo-app',
        verdict: 'rejected',
        findings: [],
        evidence: [],
      }))
      expect(record.verdict).toBe(expected)
    }
  })

  it('fingerprint 不可得（app.json 缺失）→ 拒绝裁决而不是以空输入冒充', async () => {
    await resetApp()
    await rm(path.join(appDir(), 'app.json'))
    const message = expectError(await callTool(ctx, 'alioth_closure', { action: 'status', namespace: 'Alioth', app: 'demo-app' }))
    expect(message).toContain('产物指纹不可得')
  })
})

describe('alioth_version：两段式回退', () => {
  it('confirmed !== true 拒绝回退且一个字节都不写', async () => {
    await resetApp()
    const snapshot = expectOk(await callTool(ctx, 'alioth_version', { action: 'snapshot', namespace: 'Alioth', app: 'demo-app' }))
    expect(snapshot.seq).toBe(1)
    expect(String(snapshot.dir)).toContain('0001-')

    // 快照后改产物：未确认的回退 MUST NOT 把它盖回去。
    const edited = `${APP_JSON.replace('Demo App', 'Edited After Snapshot')}`
    await writeFile(path.join(appDir(), 'app.json'), edited)
    const before = await readFile(path.join(appDir(), 'app.json'))

    const message = expectError(await callTool(ctx, 'alioth_version', {
      action: 'rollback',
      namespace: 'Alioth',
      app: 'demo-app',
      seq: 1,
    }))
    expect(message).toContain('confirmed !== true')
    expect(message).toContain('两段式')
    expect(await readFile(path.join(appDir(), 'app.json'))).toEqual(before)

    const listed = expectOk(await callTool(ctx, 'alioth_version', { action: 'list', namespace: 'Alioth', app: 'demo-app' }))
    expect(listed.versions).toEqual([1])

    const restored = expectOk(await callTool(ctx, 'alioth_version', {
      action: 'rollback',
      namespace: 'Alioth',
      app: 'demo-app',
      seq: 1,
      confirmed: true,
    }))
    expect(restored.seq).toBe(1)
    expect(await readFile(path.join(appDir(), 'app.json'), 'utf8')).toBe(APP_JSON)
  })
})

describe('alioth_patch_assets：两段式局部改动', () => {
  it('propose 不写盘；apply 未确认拒绝且字节级不变', async () => {
    await resetApp()
    const target = path.join(appDir(), 'extensions', 'constraints.yaml')
    const before = await readFile(target)

    const proposal = expectOk(await callTool(ctx, 'alioth_patch_assets', {
      action: 'propose',
      namespace: 'Alioth',
      app: 'demo-app',
      target: 'extensions/constraints.yaml',
      after: `${CONSTRAINTS_YAML}- entity: zc_id_demand\n  expression: "qty >= 0"\n  message: "需求不得为负"\n`,
    }))
    expect(String(proposal.unifiedDiff)).toContain('+')
    expect(String(proposal.baseFingerprint).startsWith('sha256:')).toBe(true)
    expect(await readFile(target)).toEqual(before)

    const message = expectError(await callTool(ctx, 'alioth_patch_assets', {
      action: 'apply',
      namespace: 'Alioth',
      app: 'demo-app',
      proposalId: proposal.proposalId,
    }))
    expect(message).toContain('confirmed !== true')
    expect(await readFile(target)).toEqual(before)

    const applied = expectOk(await callTool(ctx, 'alioth_patch_assets', {
      action: 'apply',
      namespace: 'Alioth',
      app: 'demo-app',
      proposalId: proposal.proposalId,
      confirmed: true,
    }))
    expect(applied.applied).toBe(true)
    expect(await readFile(target, 'utf8')).toContain('zc_id_demand')

    // 基准指纹过期 → 拒绝（防覆盖并发改动），且不重复套用已消费的提案。
    const stale = expectError(await callTool(ctx, 'alioth_patch_assets', {
      action: 'apply',
      namespace: 'Alioth',
      app: 'demo-app',
      proposalId: proposal.proposalId,
      confirmed: true,
    }))
    expect(stale).toContain('不在本进程登记表')
  })

  it('拒绝 App 产物目录之外的 target（不越界改文件）', async () => {
    await resetApp()
    const message = expectError(await callTool(ctx, 'alioth_patch_assets', {
      action: 'propose',
      namespace: 'Alioth',
      app: 'demo-app',
      target: '../../../../etc/hosts',
      after: 'x\n',
    }))
    expect(message).toContain('必须落在 App 产物目录内')
  })
})

describe('alioth_capabilities：六组广告', () => {
  it('守卫在位：六组字段齐备，缺源组显式 unknown(reason)', async () => {
    const value = expectOk(await callTool(ctx, 'alioth_capabilities', {}))
    for (const group of ['tools', 'skills', 'gates', 'autonomy', 'llm', 'budget']) {
      expect(value[group]).toBeDefined()
    }
    const tools = value.tools as { kind: string; value: string[] }
    expect(tools.kind).toBe('value')
    expect(tools.value).toContain('alioth_verify')
    expect(tools.value).toContain('alioth_capabilities')

    const skills = value.skills as { kind: string; value: { name: string; version: string; drifted: boolean }[] }
    expect(skills.kind).toBe('value')
    expect(skills.value).toEqual([{ name: 'alioth-app', version: '2.2', drifted: false }])

    const gates = value.gates as { kind: string; value: { degraded: string[]; human: string[]; deferredOpen: number; plansPending: number } }
    expect(gates.kind).toBe('value')
    expect(gates.value.degraded).toEqual(['[rule:unknown-scope] fixture 降级证据'])
    expect(gates.value.plansPending).toBe(1)

    const llm = value.llm as { kind: string; value: { ready: boolean; probe: string } }
    expect(llm.kind).toBe('value')
    expect(llm.value.ready).toBe(false)
    expect(llm.value.probe).toContain('listProviders')

    const autonomy = value.autonomy as { kind: string; reason: string }
    expect(autonomy.kind).toBe('unknown')
    expect(autonomy.reason.length).toBeGreaterThan(0)

    const guard = value.guard as { kind: string; value: { whitelistSource: { source: string; reason: string }; degradations: unknown[] } }
    expect(guard.kind).toBe('value')
    expect(guard.value.whitelistSource.source).toBe('file')
    expect(guard.value.degradations).toHaveLength(1)
  })

  it('守卫缺失：gates/guard 显式 unknown(reason)，tools/skills 照常（组级错误隔离）', async () => {
    const value = expectOk(await callTool(bareCtx, 'alioth_capabilities', {}))
    for (const group of ['tools', 'skills', 'gates', 'autonomy', 'llm', 'budget']) {
      expect(value[group]).toBeDefined()
    }
    expect((value.tools as { kind: string }).kind).toBe('value')
    const gates = value.gates as { kind: string; reason: string }
    expect(gates.kind).toBe('unknown')
    expect(gates.reason).toContain('aliothGuard')
    const guard = value.guard as { kind: string; reason: string }
    expect(guard.kind).toBe('unknown')
    expect(guard.reason.length).toBeGreaterThan(0)
    expect((value.llm as { kind: string }).kind).toBe('unknown')
  })

  it('漂移基线缺失 → skills 组 unknown 而不是乐观地报未漂移', async () => {
    const pristine = await readFile(path.join(modelDir, 'PROVENANCE.json'), 'utf8')
    await rm(path.join(modelDir, 'PROVENANCE.json'))
    try {
      const value = expectOk(await callTool(bareCtx, 'alioth_capabilities', {}))
      const skills = value.skills as { kind: string; reason: string }
      expect(skills.kind).toBe('unknown')
      expect(skills.reason).toContain('漂移基线不可得')
    } finally {
      await writeFile(path.join(modelDir, 'PROVENANCE.json'), pristine)
    }
  })

  it('未登记的适配器 = drifted（漂移宁可误报不可漏报）', async () => {
    const shadow = path.join(modelDir, 'skill-adapters', 'mini-write.yaml')
    await writeFile(shadow, `${ADAPTER_YAML.replace('alioth-app', 'mini-write')}`)
    try {
      const value = expectOk(await callTool(bareCtx, 'alioth_capabilities', {}))
      const skills = value.skills as { value: { name: string; drifted: boolean }[] }
      expect(skills.value.find(entry => entry.name === 'mini-write')?.drifted).toBe(true)
    } finally {
      await rm(shadow)
    }
  })
})

describe('alioth_deferred', () => {
  it('空裁决即拒绝；触发条件满足者才解除', async () => {
    const blocker = path.join(dataRoot, 'Artifacts', 'later.json')
    const emptyAdjudication = expectError(await callTool(ctx, 'alioth_deferred', {
      action: 'register',
      sessionId: 'deferred-session',
      id: 'onto-later',
      reason: '本体输出尚未生成',
      adjudication: '   ',
      trigger: { kind: 'artifact-exists', path: blocker },
    }, 'deferred-session'))
    expect(emptyAdjudication).toContain('adjudication')

    const registered = expectOk(await callTool(ctx, 'alioth_deferred', {
      action: 'register',
      sessionId: 'deferred-session',
      id: 'onto-later',
      reason: '本体输出尚未生成',
      adjudication: '依赖的本体映射产物由上一阶段产出，缺它无法继续（已登记触发条件）',
      trigger: { kind: 'artifact-exists', path: blocker },
      successors: ['alioth_verify stage'],
    }, 'deferred-session'))
    expect(registered.count).toBe(1)
    expect((registered.item as { sessionId: string }).sessionId).toBe('deferred-session')

    const listed = expectOk(await callTool(ctx, 'alioth_deferred', { action: 'list', sessionId: 'deferred-session' }, 'deferred-session'))
    expect(listed.scope).toBe('session:deferred-session+app-gates')
    const listedIds = (listed.items as { id: string }[]).map(item => item.id)
    expect(listedIds).toContain('onto-later')
    // app 级人工门（上一组用例登记）对任何会话都可见，不受会话过滤影响。
    expect(listedIds).toContain('extensions-degraded:Alioth/demo-app')

    expect((expectOk(await callTool(ctx, 'alioth_deferred', { action: 'unlock', sessionId: 'deferred-session' }, 'deferred-session'))).count).toBe(0)

    await mkdir(path.dirname(blocker), { recursive: true })
    await writeFile(blocker, '{}\n')
    const unlocked = expectOk(await callTool(ctx, 'alioth_deferred', { action: 'unlock', sessionId: 'deferred-session' }, 'deferred-session'))
    expect(unlocked.count).toBe(1)
    expect((unlocked.unlocked as { id: string }[])[0]?.id).toBe('onto-later')
    expect((expectOk(await callTool(ctx, 'alioth_deferred', { action: 'list', sessionId: 'deferred-session' }, 'deferred-session'))).count).toBe(1)
  })

  it('拒绝非磁盘可判定的触发条件与非法 action', async () => {
    expect(expectError(await callTool(ctx, 'alioth_deferred', {
      action: 'register',
      sessionId: 'deferred-session',
      id: 'bad',
      reason: 'x',
      adjudication: 'y',
      trigger: { kind: 'expression', path: 'a && b' },
    }))).toContain('artifact-exists|artifact-fingerprint|artifact-json-pointer')
    expect(expectError(await callTool(ctx, 'alioth_deferred', { action: 'nope', sessionId: 'deferred-session' })))
      .toContain('invalid action')
  })

  it('json-pointer 触发：canonical 未达 passed 前不得解除', async () => {
    const canonical = path.join(appDir(), 'extension-verify.json')
    const item = {
      action: 'register',
      sessionId: 'pointer-session',
      id: 'pointer-gate',
      reason: '扩展验证降级',
      adjudication: '需人工确认',
      trigger: { kind: 'artifact-json-pointer', path: canonical, pointer: '/status', equals: 'passed' },
    }
    expectOk(await callTool(ctx, 'alioth_deferred', item, 'pointer-session'))
    // 文件缺失 → 未解除。
    expect((expectOk(await callTool(ctx, 'alioth_deferred', { action: 'unlock', sessionId: 'pointer-session' }, 'pointer-session'))).count).toBe(0)
    // 值为 degraded → 未解除（不等于 equals）。
    await writeFile(canonical, `${JSON.stringify({ status: 'degraded' })}\n`)
    expect((expectOk(await callTool(ctx, 'alioth_deferred', { action: 'unlock', sessionId: 'pointer-session' }, 'pointer-session'))).count).toBe(0)
    // 值达 passed → 解除（同一批次可能连带解除 app 级门：它们的触发条件同样是 canonical）。
    await writeFile(canonical, `${JSON.stringify({ status: 'passed' })}\n`)
    const unlocked = expectOk(await callTool(ctx, 'alioth_deferred', { action: 'unlock', sessionId: 'pointer-session' }, 'pointer-session'))
    expect((unlocked.unlocked as { id: string }[]).map(item => item.id)).toContain('pointer-gate')
    await rm(canonical, { force: true })
  })

  it('无会话归属：list 返回全量（不猜会话也不隐藏），unlock 拒绝', async () => {
    const value = expectOk(await callTool(ctx, 'alioth_deferred', { action: 'list' }))
    expect(value.scope).toBe('all')
    expect(Array.isArray(value.items)).toBe(true)
    expect(expectError(await callTool(ctx, 'alioth_deferred', { action: 'unlock' }))).toContain('sessionId')
  })
})

describe('alioth_usage', () => {
  it('守卫台账 + 配置价表 → 估算成本；无价表模型不得以 0 冒充', async () => {
    const value = expectOk(await callTool(ctx, 'alioth_usage', { sessionId: 'guard-session' }, 'guard-session'))
    expect(value.available).toBe(true)
    const summary = value.summary as {
      total: { tokensIn: number; tokensOut: number; calls: number }
      cost: { kind: string; totalCents: number }
    }
    expect(summary.total).toEqual({ tokensIn: 1000, tokensOut: 500, calls: 1 })
    expect(summary.cost.kind).toBe('estimated')
    expect(summary.cost.totalCents).toBe(2)
  })

  it('守卫未装配 → available:false + 原因（MUST NOT 编造用量）', async () => {
    const value = expectOk(await callTool(bareCtx, 'alioth_usage', { sessionId: 'a-session' }, 'a-session'))
    expect(value.available).toBe(false)
    expect(String(value.reason)).toContain('aliothGuard')
    expect(value.summary).toBeNull()
  })
})
