import { mkdir, mkdtemp, rm, writeFile } from 'node:fs/promises'
import { tmpdir } from 'node:os'
import path from 'node:path'
import { afterEach, describe, expect, it } from 'vitest'
import { resolvePrecedents, type PrecedentEnv } from '../src/precedents.ts'

/**
 * 先例查询：（上游 `analyze_ontology::recall_verdicts` + `transfer_ontology::decide_verdict`）
 * 账本不可读 ⇒ 空集不阻断；目录不可读 ⇒ 如实报 `unavailable` 并退到编译期常量面。
 */
const opened: string[] = []
afterEach(async () => {
  await Promise.all(opened.splice(0).map(dir => rm(dir, { recursive: true, force: true })))
})

async function envWith(verdicts: readonly Record<string, unknown>[], sqlFails = false): Promise<PrecedentEnv> {
  const root = await mkdtemp(path.join(tmpdir(), 'precedents-'))
  opened.push(root)
  if (verdicts.length > 0) {
    await mkdir(path.join(root, 'mapping-verdicts'), { recursive: true })
    await writeFile(path.join(root, 'mapping-verdicts', 'U-x.json'), `${JSON.stringify(verdicts, null, 2)}\n`)
  }
  return {
    dataRoot: () => root,
    sql: async () => {
      if (sqlFails) {
        throw new Error('connection error and is not queryable')
      }
      return { rows: [{ table_name: 'zc_id_unit' }] }
    },
  }
}

const recorded = (over: Record<string, unknown> = {}): Record<string, unknown> => ({
  namespace: 'U-x',
  domain: 'material_plan',
  table: 'zc_id_unit',
  verdictText: '数量并入 unit',
  source: 'user',
  confidence: 1,
  recordedAt: '2026-09-25T00:00:00.000Z',
  ...over,
})

describe('resolvePrecedents', () => {
  it('returns the recorded verdict for the domain', async () => {
    const result = await resolvePrecedents(await envWith([recorded()]), { namespace: 'U-x', domain: 'material_plan' })
    expect(result).toMatchObject({
      catalog: 'used',
      precedents: 1,
      verdict: { table: 'zc_id_unit', confidence: 1, verdict_text: '数量并入 unit' },
      ignored: [],
    })
  })

  it('reports no verdict when the namespace has no ledger or no matching domain', async () => {
    expect(await resolvePrecedents(await envWith([]), { namespace: 'U-x', domain: 'material_plan' }))
      .toMatchObject({ precedents: 0, verdict: null, ignored: [] })
    expect(await resolvePrecedents(await envWith([recorded({ domain: 'billing' })]), { namespace: 'U-x', domain: 'material_plan' }))
      .toMatchObject({ precedents: 0, verdict: null, ignored: [] })
  })

  it('rejects a stale table with the reason attached', async () => {
    const result = await resolvePrecedents(await envWith([recorded({ table: 'zc_id_retired' })]), { namespace: 'U-x', domain: 'material_plan' })
    expect(result.verdict).toBeNull()
    expect(result.ignored).toEqual([{ domain: 'material_plan', table: 'zc_id_retired', reason: '表不存在', confidence: 1 }])
  })

  it('still returns a lifecycle entity when the registry read fails, and says the catalog is unavailable', async () => {
    const env = await envWith([recorded({ domain: 'bill', table: 'zc_id_bill' })], true)
    const result = await resolvePrecedents(env, { namespace: 'U-x', domain: 'bill' })
    expect(result.catalog).toBe('unavailable')
    expect(result.verdict?.table).toBe('zc_id_bill')
  })

  it('degrades to an empty ledger instead of throwing when the ledger is unreadable', async () => {
    const root = await mkdtemp(path.join(tmpdir(), 'precedents-broken-'))
    opened.push(root)
    await mkdir(path.join(root, 'mapping-verdicts'), { recursive: true })
    await writeFile(path.join(root, 'mapping-verdicts', 'U-x.json'), '{ not json')
    const result = await resolvePrecedents(
      { dataRoot: () => root, sql: async () => ({ rows: [{ table_name: 'zc_id_unit' }] }) },
      { namespace: 'U-x', domain: 'material_plan' },
    )
    expect(result).toMatchObject({ precedents: 0, verdict: null, ignored: [] })
  })
})
