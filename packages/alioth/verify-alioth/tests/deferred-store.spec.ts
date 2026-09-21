/**
 * deferred store 的边界行为：损坏登记文件与 RFC 6901 触发谓词。
 * 这些断言钉住两条 fail-closed 纪律——损坏登记 MUST NOT 被当作「无阻塞」
 * （`all()` 合成显式阻塞项、`sweep()` 不炸、`open()` 对本会话仍 fail-loud），
 * 以及非法 JSON pointer（前导零数组下标）一律判 not-found，绝不宽松解析。
 */
import { mkdtemp, mkdir, writeFile } from 'node:fs/promises'
import { tmpdir } from 'node:os'
import path from 'node:path'
import { describe, expect, it } from 'vitest'
import { createDeferredStore, type DeferredItem } from '../src/deferred.ts'

function item(overrides: Partial<DeferredItem> & { readonly id: string; readonly sessionId: string }): DeferredItem {
  return {
    reason: 'r',
    adjudication: 'a',
    successors: [],
    createdTs: new Date().toISOString(),
    trigger: { kind: 'artifact-exists', path: '/nonexistent/probe.json' },
    ...overrides,
  }
}

describe('deferred store: 损坏登记文件', () => {
  it('all() 把不可解析的登记合成为显式未决项，而不是跳过', async () => {
    const root = await mkdtemp(path.join(tmpdir(), 'deferred-corrupt-'))
    const store = createDeferredStore(root)
    await store.register(item({ id: 'g1', sessionId: 's1', app: 'demo', namespace: 'U-a' }))
    await mkdir(path.join(root, 'deferred'), { recursive: true })
    await writeFile(path.join(root, 'deferred', 'corrupt.json'), '{not json', 'utf8')

    const all = await store.all()
    const synthetic = all.find(entry => entry.id === 'corrupt:corrupt')
    expect(synthetic).toBeDefined()
    expect(synthetic?.app).toBeUndefined()
    expect(synthetic?.adjudication).toContain('MUST NOT 被当作「无阻塞」')
    expect(all.some(entry => entry.id === 'g1')).toBe(true)
  })

  it('sweep() 对损坏文件不抛、不解除；open() 对该会话仍 fail-loud', async () => {
    const root = await mkdtemp(path.join(tmpdir(), 'deferred-sweep-'))
    const store = createDeferredStore(root)
    await store.register(item({ id: 'g1', sessionId: 's1', app: 'demo', namespace: 'U-a' }))
    await mkdir(path.join(root, 'deferred'), { recursive: true })
    await writeFile(path.join(root, 'deferred', 'corrupt.json'), 'not json at all', 'utf8')

    await expect(store.sweep()).resolves.toHaveLength(0)
    await expect(store.open('corrupt')).rejects.toThrow(/不可解析/)
    expect((await store.all()).some(entry => entry.id === 'corrupt:corrupt')).toBe(true)
  })
})

describe('deferred store: JSON pointer 触发谓词（RFC 6901）', () => {
  it('转义与嵌套取值正确；非法下标判 not-found', async () => {
    const root = await mkdtemp(path.join(tmpdir(), 'deferred-ptr-'))
    const store = createDeferredStore(root)
    const doc = path.join(root, 'doc.json')
    await writeFile(doc, JSON.stringify({ 'a/b': 1, 'a~b': 2, arr: [{ x: 1 }, { x: 2 }], status: 'passed' }), 'utf8')

    const cases: readonly (readonly [string, unknown, boolean])[] = [
      ['/status', 'passed', true],
      ['/a~1b', 1, true],
      ['/a~0b', 2, true],
      ['/arr/1/x', 2, true],
      ['/arr/01/x', 2, false],
      ['/arr/0/x', 1, true],
      ['/missing', 1, false],
      // 空 pointer = 整份文档的**精确**深比较（非子集匹配）：只给一个键的
      // 期望对象与全文档不等 → 不解除。
      ['', { status: 'passed' }, false],
    ]
    for (const [index, [pointer, equals]] of cases.entries()) {
      await store.register(item({ id: `p${index}`, sessionId: 's2', trigger: { kind: 'artifact-json-pointer', path: doc, pointer, equals } }))
    }
    await store.sweep()
    const remaining = new Set((await store.open('s2')).map(entry => entry.id))
    for (const [index, [pointer, , shouldSatisfy]] of cases.entries()) {
      // 满足 → 被 sweep 解除（不在 remaining）；不满足 → 仍在。
      expect(remaining.has(`p${index}`), `pointer ${pointer}`).toBe(!shouldSatisfy)
    }
  })

  it('非 JSON 内容 / 文件缺失 → 未解除（判据是内容说通过，不是文件在那儿）', async () => {
    const root = await mkdtemp(path.join(tmpdir(), 'deferred-bad-'))
    const store = createDeferredStore(root)
    const bad = path.join(root, 'bad.json')
    await writeFile(bad, 'not-json', 'utf8')
    await store.register(item({ id: 'b1', sessionId: 's3', trigger: { kind: 'artifact-json-pointer', path: bad, pointer: '/status', equals: 'passed' } }))
    await store.register(item({ id: 'b2', sessionId: 's3', trigger: { kind: 'artifact-json-pointer', path: path.join(root, 'gone.json'), pointer: '/status', equals: 'passed' } }))

    await expect(store.sweep()).resolves.toHaveLength(0)
    expect((await store.open('s3')).map(entry => entry.id).sort()).toEqual(['b1', 'b2'])
  })
})
