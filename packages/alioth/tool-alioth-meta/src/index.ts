/**
 * Model-facing tools over the Alioth entity registry (`isahl_meta`), grounded
 * in `ctx.aliothEnv`. Main paths are deterministic (SQL, validators, embedding
 * retrieval — no LLM); semantic alignment of natural-language concepts to
 * registry terms is served by embedding search, with the model making the
 * final mapping decision.
 * @module @dsh-alioth/tool-alioth-meta
 */

import type { Context } from '@deepseek-ai/cordis'
import z from '@deepseek-ai/schemastery'
import { registerSchemaInfo } from './schema-info.ts'
import { registerSemanticSearch } from './semantic-search.ts'
import { registerEntityWrite } from './entity-write.ts'
import type { SemanticEntry } from './embedding.ts'
export { ensureSemanticIndex, TransformersEmbedder, entryText, type Embedder, type SemanticEntry, type SemanticHit, type SemanticIndex } from './embedding.ts'

export const name = 'tool-alioth-meta'
export const inject = ['tools', 'aliothEnv']

export interface Config {
  /**
   * Write-approval mode for `alioth_entity_write`. `'required'` fails the
   * write without a composed ApprovalService and routes every write through
   * it (grant = `allowed-once`); `'bypass'` writes without asking — choose it
   * only for unattended/CI deployments.
   */
  approvalMode?: 'required' | 'bypass'
}

export const Config: z<Config> = z.object({
  approvalMode: z.union(['required', 'bypass'] as const).default('bypass'),
})

/** Registry terms for embedding: entities + titled fields (excludes test entities). */
export async function loadSemanticEntries(ctx: Context): Promise<readonly SemanticEntry[]> {
  const entities = await ctx.aliothEnv.sql<{ table_name: string; name: string; biz_description: string | null }>(
    `SELECT table_name, name, biz_description
     FROM isahl_meta.meta_collections
     WHERE table_name NOT LIKE '%-testing' AND table_name NOT LIKE '%-test'
     ORDER BY table_name`,
  )
  const fields = await ctx.aliothEnv.sql<{ fk_collection: string; name: string; title: string }>(
    `SELECT fk_collection, name, title
     FROM isahl_meta.meta_fields
     WHERE title != ''
     ORDER BY fk_collection, name`,
  )
  return [
    ...entities.rows.map(row => ({
      kind: 'entity' as const,
      table: row.table_name,
      name: row.name,
      title: row.biz_description ?? '',
    })),
    ...fields.rows.map(row => ({
      kind: 'field' as const,
      table: row.fk_collection,
      name: row.name,
      title: row.title,
    })),
  ]
}

export function apply(ctx: Context, config: Config): void {
  const approvalMode = config.approvalMode ?? 'bypass'
  registerSchemaInfo(ctx)
  registerSemanticSearch(ctx, loadSemanticEntries)
  registerEntityWrite(ctx, approvalMode)
}
