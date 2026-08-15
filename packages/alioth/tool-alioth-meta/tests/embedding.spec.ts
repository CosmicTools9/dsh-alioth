import { describe, expect, it, beforeAll, afterAll } from 'vitest'
import { mkdtemp, readFile, rm } from 'node:fs/promises'
import { tmpdir } from 'node:os'
import path from 'node:path'
import {
  ensureSemanticIndex,
  entryText,
  type Embedder,
  type SemanticEntry,
} from '../src/embedding.ts'

/** Deterministic embedder: each character contributes to a basis bucket. */
class FakeEmbedder implements Embedder {
  async embed(texts: readonly string[]): Promise<readonly Float32Array[]> {
    return texts.map(text => {
      const vector = new Float32Array(8)
      for (const ch of text) {
        const bucket = (ch.codePointAt(0) ?? 0) % 8
        vector[bucket] = (vector[bucket] ?? 0) + 1
      }
      const norm = Math.sqrt(vector.reduce((sum, value) => sum + value * value, 0)) || 1
      return vector.map(value => value / norm)
    })
  }
}

const ENTRIES: readonly SemanticEntry[] = [
  { kind: 'entity', table: 'zc_id_inve-money', name: '库存-账户金额', title: '' },
  { kind: 'entity', table: 'zc_id_inventory', name: '库存', title: '库存台账' },
  { kind: 'field', table: 'zc_id_inventory', name: 'qty', title: '数量' },
  { kind: 'field', table: 'zc_id_inventory', name: 'owner_qk_user', title: '负责人' },
  { kind: 'entity', table: 'zc_id_demand', name: '需求', title: '' },
]

describe('semantic embedding index', () => {
  let dataRoot: string

  beforeAll(async () => {
    dataRoot = await mkdtemp(path.join(tmpdir(), 'semantic-index-'))
  })

  afterAll(async () => {
    await rm(dataRoot, { recursive: true, force: true })
  })

  it('builds an index and searches by cosine similarity', async () => {
    const index = await ensureSemanticIndex(dataRoot, ENTRIES, new FakeEmbedder(), 'fake-model')
    expect(index.meta.count).toBe(5)
    expect(index.meta.dimension).toBe(8)
    const hits = await index.search('库存', 3)
    expect(hits).toHaveLength(3)
    expect(hits[0]?.entry.table).toBe('zc_id_inventory')
    // Scores come back sorted descending.
    const scores = hits.map(hit => hit.score)
    expect(scores).toEqual([...scores].sort((a, b) => b - a))
  })

  it('filters by kind', async () => {
    const index = await ensureSemanticIndex(dataRoot, ENTRIES, new FakeEmbedder(), 'fake-model')
    const fields = await index.search('数量', 5, 'field')
    expect(fields.every(hit => hit.entry.kind === 'field')).toBe(true)
    expect(fields[0]?.entry.name).toBe('qty')
  })

  it('reuses the disk cache when entries are unchanged', async () => {
    const first = await ensureSemanticIndex(dataRoot, ENTRIES, new FakeEmbedder(), 'fake-model')
    const second = await ensureSemanticIndex(dataRoot, ENTRIES, new FakeEmbedder(), 'fake-model')
    expect(second.meta.entriesHash).toBe(first.meta.entriesHash)
    const meta = JSON.parse(await readFile(path.join(dataRoot, 'semantic', 'meta.json'), 'utf8')) as { count: number }
    expect(meta.count).toBe(5)
  })

  it('rebuilds when the entry set changes', async () => {
    const changed: readonly SemanticEntry[] = [
      ...ENTRIES,
      { kind: 'entity', table: 'zc_id_new', name: '新实体', title: '' },
    ]
    const index = await ensureSemanticIndex(dataRoot, changed, new FakeEmbedder(), 'fake-model')
    expect(index.meta.count).toBe(6)
    const meta = JSON.parse(await readFile(path.join(dataRoot, 'semantic', 'meta.json'), 'utf8')) as { entriesHash: string }
    expect(index.meta.entriesHash).toBe(meta.entriesHash)
  })

  it('composes entry search text with name + table', () => {
    expect(entryText({ kind: 'entity', table: 'zc_id_inventory', name: '库存', title: '' }))
      .toContain('库存')
    expect(entryText({ kind: 'field', table: 'zc_id_inventory', name: 'qty', title: '数量' }))
      .toContain('数量')
  })

  it('normalizes embedder output to unit length', async () => {
    const vectors = await new FakeEmbedder().embed(['测试'])
    const vector = vectors[0]!
    const norm = Math.sqrt(Array.from(vector).reduce((sum, value) => sum + value * value, 0))
    expect(norm).toBeCloseTo(1, 5)
  })

  it('force re-embeds even when the cache is fresh', async () => {
    let embedCalls = 0
    const counting: Embedder = {
      embed: async texts => {
        embedCalls += 1
        return new FakeEmbedder().embed(texts)
      },
    }
    await ensureSemanticIndex(dataRoot, ENTRIES, counting, 'fake-model')
    expect(embedCalls).toBe(1)
    await ensureSemanticIndex(dataRoot, ENTRIES, counting, 'fake-model')
    expect(embedCalls).toBe(1) // cache hit: no re-embed
    await ensureSemanticIndex(dataRoot, ENTRIES, counting, 'fake-model', true)
    expect(embedCalls).toBe(2) // force: re-embed regardless of the cache
  })
})
