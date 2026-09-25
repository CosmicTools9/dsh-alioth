import { mkdtemp, readFile, rm } from 'node:fs/promises'
import { tmpdir } from 'node:os'
import path from 'node:path'
import { afterAll, describe, expect, it } from 'vitest'
import { Context } from '@deepseek-ai/cordis'
import SystemPrompt from '@deepseek-ai/dsh-system-prompt'
import ToolRuntime from '@deepseek-ai/dsh-tools'
import { ToolCallId } from '@deepseek-ai/dsh-llm'
import * as verifyTools from '../src/index.ts'

/**
 * Mirrors the upstream `record_mapping_verdict` sink (`dialog_tools/record_verdict.rs`):
 * `keep_gap` refuses, an out-of-catalog table refuses (anti-stale), an unjudgeable catalog refuses
 * (fail-closed — and the compile-time lifecycle list is never accepted as "catalog usable").
 */
const signal = new AbortController().signal
const COLLECTIONS = ['zc_id_unit', 'zc_id_contract']

/** Registry stub: answers the two catalog queries, or throws to model an unreachable database. */
function stubSql(options: { readonly collections?: readonly string[]; readonly fail?: boolean }) {
  return async (text: string, values?: readonly unknown[]) => {
    if (options.fail === true) {
      throw new Error('connection error and is not queryable')
    }
    if (text.includes('count(*)')) {
      return { rows: [{ n: (options.collections ?? COLLECTIONS).length }] }
    }
    const table = String(values?.[0] ?? '')
    return { rows: (options.collections ?? COLLECTIONS).includes(table) ? [{ ok: 1 }] : [] }
  }
}

let root: string
let counter = 0
const disposers: Array<() => Promise<void>> = []

async function makeContext(options: { readonly collections?: readonly string[]; readonly fail?: boolean }): Promise<Context> {
  root = await mkdtemp(path.join(tmpdir(), 'verdict-'))
  const ctx = new Context()
  ctx.provide('aliothEnv', { dataRoot: () => root, sql: stubSql(options) } as never)
  const system = await ctx.plugin(SystemPrompt)
  disposers.push(() => system.dispose())
  const tools = await ctx.plugin(ToolRuntime)
  disposers.push(() => tools.dispose())
  const plugin = await ctx.plugin(verifyTools, { preProcRoot: root })
  disposers.push(() => plugin.dispose())
  return ctx
}

function call(ctx: Context, args: Record<string, unknown>) {
  return ctx.tools.execute({
    signal,
    callId: ToolCallId(`verdict-${++counter}`),
    name: 'alioth_mapping_verdict',
    arguments: args,
  })
}

afterAll(async () => {
  for (const dispose of disposers.reverse()) await dispose().catch(() => {})
  await rm(root, { recursive: true, force: true })
})

describe('alioth_mapping_verdict', () => {
  it('sediments a valid verdict and recalls it, newest first', async () => {
    const ctx = await makeContext({})
    const first = await call(ctx, { namespace: 'U-x', domain: 'material_plan', table: 'zc_id_unit', verdict_text: '先按 unit' })
    expect(first.isError).toBe(false)
    expect((first.value as { verdict: { source: string; confidence: number } }).verdict).toMatchObject({ source: 'user', confidence: 1 })
    await call(ctx, { namespace: 'U-x', domain: 'material_plan', table: 'zc_id_contract', verdict_text: '合同走 contract' })

    const recalled = await call(ctx, { namespace: 'U-x', action: 'recall', domain: 'material_plan' })
    const verdicts = (recalled.value as { verdicts: Array<{ table: string }> }).verdicts
    expect(verdicts.map(entry => entry.table)).toEqual(['zc_id_contract', 'zc_id_unit'])

    const ledger = JSON.parse(await readFile(path.join(root, 'mapping-verdicts', 'U-x.json'), 'utf8')) as unknown[]
    expect(ledger).toHaveLength(2)
  })

  it('refuses keep_gap and sediments nothing', async () => {
    const ctx = await makeContext({})
    const result = await call(ctx, { namespace: 'U-gap', domain: 'material_plan', table: 'zc_id_unit', keep_gap: true })
    expect(result.isError).toBe(true)
    expect(result.isError ? result.error.message : '').toContain('keep the gap open')
    await expect(readFile(path.join(root, 'mapping-verdicts', 'U-gap.json'), 'utf8')).rejects.toThrow()
  })

  it('refuses a table outside the platform catalog (anti-stale)', async () => {
    const ctx = await makeContext({})
    const result = await call(ctx, { namespace: 'U-x', domain: 'material_plan', table: 'zc_id_retired' })
    expect(result.isError).toBe(true)
    expect(result.isError ? result.error.message : '').toContain('not in the platform catalog')
  })

  it('fails closed when the catalog cannot be judged, even for a lifecycle entity', async () => {
    const ctx = await makeContext({ collections: [] })
    const result = await call(ctx, { namespace: 'U-x', domain: 'material_plan', table: 'zc_id_bill' })
    expect(result.isError).toBe(true)
    expect(result.isError ? result.error.message : '').toContain('catalog unusable')
  })

  it('fails closed when the registry is unreachable and writes nothing', async () => {
    const ctx = await makeContext({ fail: true })
    const result = await call(ctx, { namespace: 'U-down', domain: 'material_plan', table: 'zc_id_unit' })
    expect(result.isError).toBe(true)
    expect(result.isError ? result.error.message : '').toContain('catalog unavailable')
    await expect(readFile(path.join(root, 'mapping-verdicts', 'U-down.json'), 'utf8')).rejects.toThrow()
  })

  it('accepts a lifecycle entity when the catalog is usable', async () => {
    const ctx = await makeContext({})
    const result = await call(ctx, { namespace: 'U-x', domain: 'bill', table: 'zc_id_bill' })
    expect(result.isError).toBe(false)
  })
})
