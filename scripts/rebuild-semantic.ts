/**
 * One-shot semantic-index rebuild: load registry terms through env-alioth,
 * force-re-embed them, and report the index. The main pipeline is
 * deterministic — this tool maintains the semantic space; LLM is never
 * involved in embedding or retrieval.
 *
 * Env: ALIOTH_MODEL_SOURCE / ALIOTH_DATABASE_URL / ALIOTH_DATA_ROOT
 * (same overrides as `mise run alioth:doctor`).
 */
import { Context } from '@deepseek-ai/cordis'
import * as envAlioth from '@dsh-alioth/env-alioth'
import { loadSemanticEntries, TransformersEmbedder, ensureSemanticIndex } from '@dsh-alioth/tool-alioth-meta'

const databaseUrl = process.env.ALIOTH_DATABASE_URL
const dataRoot = process.env.ALIOTH_DATA_ROOT
const config: envAlioth.Config = {
  modelSource: process.env.ALIOTH_MODEL_SOURCE ?? 'builtin',
  ...(databaseUrl === undefined ? {} : { databaseUrl }),
  ...(dataRoot === undefined ? {} : { dataRoot }),
}

const ctx = new Context()
const fiber = await ctx.plugin(envAlioth, config)
try {
  await ctx.aliothEnv.ready()
  const entries = await loadSemanticEntries(ctx)
  const started = Date.now()
  const index = await ensureSemanticIndex(ctx.aliothEnv.dataRoot(), entries, new TransformersEmbedder(), 'Xenova/bge-small-zh-v1.5', true)
  const seconds = ((Date.now() - started) / 1000).toFixed(1)
  console.log(`semantic index: ${index.meta.count} entries, ${index.meta.dimension} dims, model ${index.meta.model}`)
  console.log(`rebuilt in ${seconds}s under ${ctx.aliothEnv.dataRoot()}/semantic/`)
} finally {
  await fiber.dispose()
}
