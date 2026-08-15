/**
 * `alioth_schema_semantic_search` tool: multilingual semantic grounding over
 * registry terms (synonyms + cross-language). Deterministic embedding — no
 * LLM in the retrieval path; complex concept mapping is the model's decision
 * on top of these hits.
 * @module @dsh-alioth/tool-alioth-meta/semantic-search
 */

import type { Context } from '@deepseek-ai/cordis'
import { defineTool } from '@deepseek-ai/dsh-tools'
import { ensureSemanticIndex, TransformersEmbedder, type SemanticEntry } from './embedding.ts'

/** Register the `alioth_schema_semantic_search` tool. */
export function registerSemanticSearch(ctx: Context, loadEntries: (ctx: Context) => Promise<readonly SemanticEntry[]>): void {
  ctx.tools.register(defineTool({
    name: 'alioth_schema_semantic_search',
    description:
      'Semantic search over the Alioth entity registry — covers synonyms and cross-language '
      + 'terms that literal matching misses (e.g. "库存余额" or "inventory balance" hits 库存-账户金额). '
      + 'Embeds registry terms with a multilingual model; first use downloads the model and builds '
      + 'an index under the data root (offline afterwards). Use for near-miss concepts; for exact '
      + 'names use alioth_schema_info.',
    parameters: {
      query: {
        type: 'string',
        required: true,
        description: 'Free-form concept description in any language (e.g. "库存余额", "purchase order").',
      },
      topK: {
        type: 'number',
        description: 'Max hits (default 10, max 50).',
      },
      kind: {
        type: 'string',
        description: 'Restrict to "entity" or "field".',
      },
    },
    output: {
      schema: {
        type: 'object',
        additionalProperties: false,
        properties: {
          query: { type: 'string', required: true },
          hits: {
            type: 'array',
            items: {
              type: 'object',
              additionalProperties: false,
              properties: {
                score: { type: 'number', required: true },
                kind: { type: 'string', required: true },
                table: { type: 'string', required: true },
                name: { type: 'string', required: true },
                title: { type: 'string', required: true },
              },
            },
          },
        },
      },
      render: (_args, value) => [{
        type: 'text',
        text: `Semantic hits for ${JSON.stringify(String(value.query))}: `
          + `${(value.hits ?? []).map((hit: { score: number; table: string; name: string }) => `${hit.table} (${hit.name}, ${hit.score.toFixed(2)})`).join(', ')}`,
      }],
    },
    async execute(args) {
      if (typeof args.query !== 'string' || args.query.trim().length === 0) {
        throw new Error('alioth_schema_semantic_search: requires "query"')
      }
      const topK = args.topK === undefined ? 10 : args.topK
      if (!Number.isInteger(topK) || topK < 1 || topK > 50) {
        throw new Error(`alioth_schema_semantic_search: topK must be an integer in [1, 50]`)
      }
      const kind = args.kind
      if (kind !== undefined && kind !== 'entity' && kind !== 'field') {
        throw new Error('alioth_schema_semantic_search: kind must be "entity" or "field"')
      }
      await ctx.aliothEnv.ready()
      const entries = await loadEntries(ctx)
      const index = await ensureSemanticIndex(ctx.aliothEnv.dataRoot(), entries, new TransformersEmbedder())
      const hits = await index.search(args.query.trim(), topK, kind)
      return {
        query: args.query,
        hits: hits.map(hit => ({
          score: Number(hit.score.toFixed(4)),
          kind: hit.entry.kind,
          table: hit.entry.table,
          name: hit.entry.name,
          title: hit.entry.title,
        })),
      }
    },
    presentCall: args => ({
      card: 'generic',
      title: `Semantic Alioth search: ${String(args.query)}`,
      kind: 'other',
      rawInput: args as Record<string, unknown>,
    }),
  }))
}
