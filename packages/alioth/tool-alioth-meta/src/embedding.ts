/**
 * Semantic grounding for the entity registry: embeddings over registered
 * entity/field terms (Chinese + English + pinyin-free cross-lingual coverage)
 * with a disk cache under the env data root. The runtime model is
 * transformers.js + a multilingual embedding model (default Xenova/bge-small-zh-v1.5,
 * 384 dims); the endpoint defaults to the hf-mirror CDN (override with
 * `DSH_HF_ENDPOINT`). First use downloads the model and builds the index —
 * afterwards everything is offline.
 * @module @dsh-alioth/tool-alioth-meta/embedding
 */

import { createHash } from 'node:crypto'
import { mkdir, readFile, writeFile } from 'node:fs/promises'
import path from 'node:path'
import { env as transformersEnv, pipeline } from '@huggingface/transformers'

export interface SemanticEntry {
  readonly kind: 'entity' | 'field'
  readonly table: string
  readonly name: string
  readonly title: string
}

export interface SemanticHit {
  readonly score: number
  readonly entry: SemanticEntry
}

export interface Embedder {
  /** Embed texts, returning L2-normalized vectors. */
  embed(texts: readonly string[]): Promise<readonly Float32Array[]>
}

const DEFAULT_MODEL = 'Xenova/bge-small-zh-v1.5'
const DEFAULT_ENDPOINT = 'https://hf-mirror.com'

/** Human-readable search text for one registry entry. */
export function entryText(entry: SemanticEntry): string {
  return entry.kind === 'entity'
    ? `${entry.name} ${entry.table}`
    : `${entry.title} ${entry.name} ${entry.table}`
}

function normalize(vector: Float32Array): Float32Array {
  let norm = 0
  for (const value of vector) {
    norm += value * value
  }
  norm = Math.sqrt(norm) || 1
  const out = new Float32Array(vector.length)
  for (let index = 0; index < vector.length; index += 1) {
    out[index] = (vector[index] ?? 0) / norm
  }
  return out
}

/** transformers.js feature-extraction embedder. */
/**
 * transformers.js feature-extraction embedder. The model resolves in this
 * order: `DSH_EMBEDDING_MODEL` (local model directory or HF model id) →
 * `DSH_HF_ENDPOINT` mirror → hf-mirror default. A local path makes the
 * semantic library fully offline (semantic library = dict snapshots + model
 * files, shipped with the release).
 */
export class TransformersEmbedder implements Embedder {
  private readonly ready: Promise<(texts: readonly string[]) => Promise<readonly Float32Array[]>>

  constructor(
    model: string = process.env.DSH_EMBEDDING_MODEL ?? DEFAULT_MODEL,
    endpoint: string = process.env.DSH_HF_ENDPOINT ?? DEFAULT_ENDPOINT,
  ) {
    transformersEnv.remoteHost = endpoint
    this.ready = (async () => {
      const extract = await pipeline('feature-extraction', model, { dtype: 'fp32' })
      return async (texts: readonly string[]): Promise<readonly Float32Array[]> => {
        const output = await extract([...texts], { pooling: 'mean', normalize: false })
        const data = output.data as Float32Array
        const dims = output.dims
        const dimension = dims === undefined ? 384 : (dims.at(-1) ?? 384)
        const vectors: Float32Array[] = []
        for (let offset = 0; offset < data.length; offset += dimension) {
          vectors.push(normalize(data.subarray(offset, offset + dimension)))
        }
        return vectors
      }
    })()
  }

  async embed(texts: readonly string[]): Promise<readonly Float32Array[]> {
    const run = await this.ready
    return run(texts)
  }
}

interface SemanticIndexMeta {
  readonly model: string
  readonly entriesHash: string
  readonly count: number
  readonly dimension: number
}

export interface SemanticIndex {
  readonly meta: SemanticIndexMeta
  readonly entries: readonly SemanticEntry[]
  readonly vectors: Float32Array
  /** Cosine search (vectors pre-normalized → dot product), top-k by kind filter. */
  search(query: string, topK: number, kind?: 'entity' | 'field'): Promise<readonly SemanticHit[]>
}

function hashEntries(entries: readonly SemanticEntry[]): string {
  const digest = createHash('sha1')
  for (const entry of entries) {
    digest.update(`${entry.kind}|${entry.table}|${entry.name}|${entry.title}\n`)
  }
  return digest.digest('hex').slice(0, 16)
}

/**
 * Build or load the semantic index for `entries` under `dataRoot/semantic/`.
 * Cache is keyed by the entries hash; a changed registry re-embeds. `force`
 * skips the cache check and re-embeds unconditionally (the explicit rebuild
 * path for the maintenance tool).
 */
export async function ensureSemanticIndex(
  dataRoot: string,
  entries: readonly SemanticEntry[],
  embedder: Embedder,
  model: string = DEFAULT_MODEL,
  force: boolean = false,
): Promise<SemanticIndex> {
  const dir = path.join(dataRoot, 'semantic')
  const metaFile = path.join(dir, 'meta.json')
  const entriesFile = path.join(dir, 'entries.json')
  const vectorsFile = path.join(dir, 'vectors.bin')
  const entriesHash = hashEntries(entries)
  if (!force) {
    try {
      const meta = JSON.parse(await readFile(metaFile, 'utf8')) as SemanticIndexMeta
      if (meta.entriesHash === entriesHash && meta.model === model) {
        const cachedEntries = JSON.parse(await readFile(entriesFile, 'utf8')) as SemanticEntry[]
        const buffer = await readFile(vectorsFile)
        const flat = new Float32Array(buffer.buffer, buffer.byteOffset, buffer.byteLength / 4)
        return {
          meta,
          entries: cachedEntries,
          vectors: flat,
          search: makeSearch(cachedEntries, flat, embedder),
        }
      }
    } catch {
      // Missing or stale cache: rebuild below.
    }
  }
  await mkdir(dir, { recursive: true })
  const texts = entries.map(entryText)
  const vectors = await embedder.embed(texts)
  const dimension = vectors[0]?.length ?? 0
  const flat = new Float32Array(entries.length * dimension)
  vectors.forEach((vector, index) => {
    flat.set(vector, index * dimension)
  })
  const meta: SemanticIndexMeta = { model, entriesHash, count: entries.length, dimension }
  await writeFile(metaFile, `${JSON.stringify(meta, null, 2)}\n`)
  await writeFile(entriesFile, `${JSON.stringify(entries, null, 2)}\n`)
  await writeFile(vectorsFile, Buffer.from(flat.buffer))
  return { meta, entries, vectors: flat, search: makeSearch(entries, flat, embedder) }
}

function makeSearch(
  entries: readonly SemanticEntry[],
  vectors: Float32Array,
  embedder: Embedder,
) {
  return async (query: string, topK: number, kind?: 'entity' | 'field'): Promise<readonly SemanticHit[]> => {
    const queryVectors = await embedder.embed([query])
    const queryVector = queryVectors[0]
    if (queryVector === undefined) {
      return []
    }
    const dimension = queryVector.length
    const scores: Array<{ score: number; index: number }> = []
    for (let index = 0; index < entries.length; index += 1) {
      if (kind !== undefined && entries[index]?.kind !== kind) {
        continue
      }
      let dot = 0
      const offset = index * dimension
      for (let d = 0; d < dimension; d += 1) {
        dot += (queryVector[d] ?? 0) * (vectors[offset + d] ?? 0)
      }
      scores.push({ score: dot, index })
    }
    scores.sort((a, b) => b.score - a.score)
    return scores.slice(0, topK).map(({ score, index }) => {
      const entry = entries[index]
      if (entry === undefined) {
        throw new Error('semantic index: entry index out of range')
      }
      return { score, entry }
    })
  }
}
