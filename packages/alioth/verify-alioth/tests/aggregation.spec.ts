/**
 * 运行时聚合：能力广告的组级错误隔离、阻塞登记的裁决必填与触发解除、用量成本不可得。
 * 〔契约 §4/T3 必需测试〕
 * @module @dsh-alioth/verify-alioth/tests/aggregation
 */

import { afterEach, describe, expect, it } from 'vitest'
import { createHash } from 'node:crypto'
import { mkdir, readFile, writeFile } from 'node:fs/promises'
import path from 'node:path'
import { collectCapabilities } from '../src/capabilities.ts'
import { createDeferredStore, type DeferredItem } from '../src/deferred.ts'
import { aggregateUsage, estimateCost } from '../src/usage.ts'
import { createTempApp, type TempApp } from './fixture.ts'

const opened: TempApp[] = []
async function app(): Promise<TempApp> {
  const created = await createTempApp()
  opened.push(created)
  return created
}
afterEach(async () => {
  await Promise.all(opened.splice(0).map(entry => entry.cleanup()))
})

describe('collectCapabilities — 组级错误隔离 + 不可得显式 unknown', () => {
  const goodGroups = {
    skills: () => [{ name: 'alioth-app', version: '2.0.0', drifted: true }],
    gates: () => ({ degraded: ['verify:extensions'], human: ['publish'], deferredOpen: 2, plansPending: 1 }),
    autonomy: () => ({ level: 'implement', failClosed: false }),
    llm: () => ({ ready: true, probe: 'deepseek-v3' }),
    budget: () => ({ maxSteps: 40, turnsUsed: 3, turnTimeoutSec: 900 }),
  }

  it('单组失败只把该组置 unknown(reason)，其余组照常给出值', async () => {
    const report = await collectCapabilities({
      ...goodGroups,
      tools: () => {
        throw new Error('工具面扫描失败：权限不足')
      },
    })

    expect(report.tools).toEqual({ kind: 'unknown', reason: 'tools 组采集失败：工具面扫描失败：权限不足' })
    expect(report.skills).toEqual({ kind: 'value', value: [{ name: 'alioth-app', version: '2.0.0', drifted: true }] })
    expect(report.gates).toEqual({
      kind: 'value',
      value: { degraded: ['verify:extensions'], human: ['publish'], deferredOpen: 2, plansPending: 1 },
    })
    expect(report.budget.kind).toBe('value')
  })

  it('未提供采集器 → 该组显式 unknown（不得省略或默认）', async () => {
    const report = await collectCapabilities({ tools: () => ['read', 'write'] })
    expect(report.tools).toEqual({ kind: 'value', value: ['read', 'write'] })
    expect(report.llm).toEqual({ kind: 'unknown', reason: expect.stringContaining('未提供 llm 组采集器') })
    expect(report.autonomy.kind).toBe('unknown')
  })

  it('采集器返回形态不符 → unknown（MUST NOT 猜测），不污染其余组', async () => {
    const report = await collectCapabilities({
      ...goodGroups,
      tools: () => ({ tools: ['read'] }),
      budget: () => ({ maxSteps: 'many', turnsUsed: 0, turnTimeoutSec: 900 }),
    })
    expect(report.tools).toEqual({ kind: 'unknown', reason: expect.stringContaining('形态非法') })
    expect(report.budget.kind).toBe('unknown')
    expect(report.llm.kind).toBe('value')
  })
})

describe('createDeferredStore — adjudication 必填 / 触发解除', () => {
  const item = (overrides: Partial<DeferredItem> = {}): DeferredItem => ({
    id: 'defer-1',
    sessionId: 'session-1',
    reason: 'E2E 浏览器层环境不可达',
    adjudication: '环境不可达属外部依赖，先推进其余范围',
    trigger: { kind: 'artifact-exists', path: 'Pre-Proc/TestNS/Apps/tm-app/e2e-report.json' },
    successors: ['重跑 e2e_verify'],
    createdTs: '2026-09-22T00:00:00.000Z',
    ...overrides,
  })

  it('adjudication 空串 → throw 且不落盘（无裁决的挂起 = 遗忘的阻塞）', async () => {
    const target = await app()
    const store = createDeferredStore(target.root)
    await expect(store.register(item({ adjudication: '   ' }))).rejects.toThrow(/adjudication 必填/)
    await expect(store.open('session-1')).resolves.toEqual([])
    await expect(readFile(path.join(target.root, 'deferred', 'session-1.json'), 'utf8')).rejects.toThrow()
  })

  it('触发条件未满足 → 保持 open；产物出现后 unlockDue 解除并返回', async () => {
    const target = await app()
    const store = createDeferredStore(target.root)
    await store.register(item())

    expect(await store.unlockDue('session-1')).toEqual([])
    expect((await store.open('session-1')).map(entry => entry.id)).toEqual(['defer-1'])

    await mkdir(path.join(target.root, 'Pre-Proc', 'TestNS', 'Apps', 'tm-app'), { recursive: true })
    await writeFile(path.join(target.root, 'Pre-Proc', 'TestNS', 'Apps', 'tm-app', 'e2e-report.json'), '{"passed":true}\n')

    const unlocked = await store.unlockDue('session-1')
    expect(unlocked.map(entry => entry.id)).toEqual(['defer-1'])
    expect(await store.open('session-1')).toEqual([])
    expect(await store.all()).toEqual([])
  })

  it('指纹触发：内容不符不解除，指纹一致才解除', async () => {
    const target = await app()
    const store = createDeferredStore(target.root)
    const rel = 'artifacts/extension-verify.json'
    await mkdir(path.join(target.root, 'artifacts'), { recursive: true })
    await writeFile(path.join(target.root, rel), '{"status":"degraded"}\n')

    const wrong = createHash('sha256').update('other').digest('hex')
    await store.register(item({ id: 'defer-wrong', trigger: { kind: 'artifact-fingerprint', path: rel, sha256: wrong } }))
    expect(await store.unlockDue('session-1')).toEqual([])

    await store.register(
      item({
        id: 'defer-right',
        trigger: {
          kind: 'artifact-fingerprint',
          path: rel,
          sha256: createHash('sha256').update('{"status":"passed"}\n').digest('hex'),
        },
      }),
    )
    expect((await store.open('session-1')).map(entry => entry.id)).toEqual(['defer-wrong', 'defer-right'])

    await writeFile(path.join(target.root, rel), '{"status":"passed"}\n')
    expect((await store.unlockDue('session-1')).map(entry => entry.id)).toEqual(['defer-right'])
    expect((await store.open('session-1')).map(entry => entry.id)).toEqual(['defer-wrong'])
  })

  it('sessionId 参与落盘路径 → 拒绝穿越与空值', async () => {
    const target = await app()
    const store = createDeferredStore(target.root)
    await expect(store.open('../../etc/passwd')).rejects.toThrow(/sessionId 非法/)
    await expect(store.register(item({ sessionId: '..' }))).rejects.toThrow(/sessionId 非法/)
  })
})

describe('aggregateUsage / estimateCost', () => {
  const events = [
    { step: 1, model: 'model-a', tokensIn: 1000, tokensOut: 500, latencyMs: 1200, turn: 1 },
    { step: 2, model: 'model-a', tokensIn: 500, tokensOut: 250, latencyMs: 800, turn: 1 },
    { step: 3, model: 'model-b', tokensIn: 2000, tokensOut: 100, latencyMs: 300, turn: 2 },
    { step: 4, model: 'model-b', tokensIn: 100, tokensOut: 100, latencyMs: 100 },
  ]

  it('三维聚合：按模型 / 总量 / 按 turn（无 turn 归属 → turn 0）', () => {
    const summary = aggregateUsage(events)
    expect(summary.byModel['model-a']).toEqual({ tokensIn: 1500, tokensOut: 750, calls: 2 })
    expect(summary.byModel['model-b']).toEqual({ tokensIn: 2100, tokensOut: 200, calls: 2 })
    expect(summary.total).toEqual({ tokensIn: 3600, tokensOut: 950, calls: 4 })
    expect(summary.turns).toEqual([
      { turn: 0, wallMs: 100, calls: 1 },
      { turn: 1, wallMs: 2000, calls: 2 },
      { turn: 2, wallMs: 300, calls: 1 },
    ])
  })

  it('无价表 → cost 显式 unavailable（MUST NOT 记 0）', () => {
    const summary = aggregateUsage(events)
    expect(summary.cost).toEqual({ kind: 'unavailable', reason: expect.stringContaining('未配置单价表') })
    expect(estimateCost(summary)).toEqual({ kind: 'unavailable', reason: expect.stringContaining('未配置单价表') })
  })

  it('存在无单价模型 → 仍 unavailable（避免少算成「更便宜」）', () => {
    const summary = aggregateUsage(events, { 'model-a': { centsPerInK: 1, centsPerOutK: 2 } })
    expect(summary.cost).toEqual({ kind: 'unavailable', reason: expect.stringContaining('无单价模型：model-b') })
  })

  it('全模型有单价 → 给出估算（cents/1K tokens）', () => {
    const prices = {
      'model-a': { centsPerInK: 1, centsPerOutK: 2 },
      'model-b': { centsPerInK: 4, centsPerOutK: 8 },
    }
    const summary = aggregateUsage(events, prices)
    // model-a: 1.5*1 + 0.75*2 = 3；model-b: 2.1*4 + 0.2*8 = 10 → 13
    expect(summary.cost).toEqual({ kind: 'estimated', totalCents: 13 })
  })
})
