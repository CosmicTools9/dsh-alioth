/**
 * `openDegradedGates` 的扫描口径：跨会话按 App 过滤 + 缺省归属 fail-closed
 * （上游 `deferred.rs:267-286`——缺 `app_code` 的记录 `map_or(true, …)` 对任何
 * App 匹配，损坏登记文件合成的显式阻塞项因此对每个发布都可见）。
 */
import { mkdtemp, mkdir, writeFile } from 'node:fs/promises'
import { tmpdir } from 'node:os'
import path from 'node:path'
import { describe, expect, it } from 'vitest'
import { createDeferredStore, type DeferredItem } from '@dsh-alioth/verify-alioth'
import { openDegradedGates } from '../src/primitives.ts'

function gate(overrides: Partial<DeferredItem> & { readonly id: string }): DeferredItem {
  return {
    sessionId: 's1',
    reason: 'uncovered declarations',
    adjudication: '需人工确认',
    successors: [],
    createdTs: new Date().toISOString(),
    trigger: { kind: 'artifact-exists', path: '/nonexistent/x.json' },
    ...overrides,
  }
}

describe('openDegradedGates', () => {
  it('按 App + namespace 过滤，先解除已满足项', async () => {
    const root = await mkdtemp(path.join(tmpdir(), 'open-gates-'))
    const store = createDeferredStore(root)
    await store.register(gate({ id: 'mine', app: 'demo', namespace: 'U-a' }))
    await store.register(gate({ id: 'other', app: 'other-app', namespace: 'U-b' }))
    const satisfied = path.join(root, 'satisfied.json')
    await writeFile(satisfied, '{"status":"passed"}\n', 'utf8')
    await store.register(gate({ id: 'done', app: 'demo', namespace: 'U-a', trigger: { kind: 'artifact-json-pointer', path: satisfied, pointer: '/status', equals: 'passed' } }))

    const open = await openDegradedGates(root, 'U-a', 'demo')
    expect(open.map(entry => entry.id)).toEqual(['mine'])
  })

  it('缺省 app/namespace 的登记对任何 App 的发布都可见（fail-closed）', async () => {
    const root = await mkdtemp(path.join(tmpdir(), 'open-gates-legacy-'))
    const store = createDeferredStore(root)
    await store.register(gate({ id: 'legacy' }))
    await store.register(gate({ id: 'corrupt-backstop', app: 'demo', namespace: 'U-a' }))
    await mkdir(path.join(root, 'deferred'), { recursive: true })
    await writeFile(path.join(root, 'deferred', 'broken.json'), '{oops', 'utf8')

    expect((await openDegradedGates(root, 'U-a', 'demo')).map(entry => entry.id).sort())
      .toEqual(['corrupt-backstop', 'corrupt:broken', 'legacy'])
    expect((await openDegradedGates(root, 'U-z', 'unrelated')).map(entry => entry.id).sort())
      .toEqual(['corrupt:broken', 'legacy'])
  })
})
