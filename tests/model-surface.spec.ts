/**
 * Keyless snapshot of the model-visible surface (harness convention: every
 * non-trivial model-visible behavior change updates a keyless snapshot).
 *
 * Mounts the full plugin group on a real Context and captures every tool's
 * JSON schema — the exact object that reaches the model. Any change to a
 * tool name, description, or parameter schema fails this test until the
 * golden file is reviewed and refreshed:
 *
 *   UPDATE_SNAPSHOTS=1 pnpm vitest run tests/model-surface.spec.ts
 */
import { mkdtemp } from 'node:fs/promises'
import { tmpdir } from 'node:os'
import path from 'node:path'
import { fileURLToPath } from 'node:url'
import { afterAll, beforeAll, describe, expect, it } from 'vitest'
import { Context } from '@deepseek-ai/cordis'
import SystemPrompt from '@deepseek-ai/dsh-system-prompt'
import ToolRuntime from '@deepseek-ai/dsh-tools'
import * as envAlioth from '@dsh-alioth/env-alioth'
import { createTestDatabase, type TestDatabase } from '../packages/alioth/env-alioth/tests/test-db.ts'
import * as toolAlioth from '@dsh-alioth/tool-alioth'
import * as toolMeta from '@dsh-alioth/tool-alioth-meta'
import * as workflow from '@dsh-alioth/tool-alioth-workflow'
import * as orchestrator from '@dsh-alioth/tool-alioth-orchestrator'
import * as guardAlioth from '@dsh-alioth/guard-alioth'
import * as toolVerify from '@dsh-alioth/tool-alioth-verify'
import * as authAlioth from '@dsh-alioth/auth-alioth'
import * as pageFeedback from '@deepseek-ai/dsh-page-feedback'
import * as toolFeedback from '@dsh-alioth/tool-feedback-alioth'
import { assembleModelSource, FIXTURE_REGISTRY_SEED } from '../scripts/lib/model-source-fixture.ts'

const GOLDEN = new URL('./__snapshots__/model-surface.json', import.meta.url)

/** A model source assembled from the vendored kit + a tiny registry seed (no `builtin`). */
async function fixtureModelSource(): Promise<string> {
  const dir = await mkdtemp(path.join(tmpdir(), 'model-surface-src-'))
  await assembleModelSource(dir, { seedSql: FIXTURE_REGISTRY_SEED })
  return dir
}

describe('model-visible surface snapshot', () => {
  let ctx: Context
  const disposers: Array<() => Promise<void>> = []
  let testDb: TestDatabase
  beforeAll(async () => {
    testDb = await createTestDatabase('surface')
    const dataRoot = await mkdtemp(path.join(tmpdir(), 'model-surface-'))
    const preProcRoot = await mkdtemp(path.join(tmpdir(), 'model-surface-pp-'))
    ctx = new Context()
    const plugins = [
      await ctx.plugin(SystemPrompt),
      await ctx.plugin(ToolRuntime),
      await ctx.plugin(envAlioth, { modelSource: await fixtureModelSource(), dataRoot, databaseUrl: testDb.url }),
      await ctx.plugin(toolAlioth, { preProcRoot }),
      await ctx.plugin(toolMeta, {}),
      await ctx.plugin(workflow, { preProcRoot }),
      await ctx.plugin(orchestrator, { preProcRoot }),
      await ctx.plugin(guardAlioth, { preProcRoot, dataRoot }),
      await ctx.plugin(toolVerify, { preProcRoot }),
      await ctx.plugin(authAlioth, { mode: 'open' }),
      await ctx.plugin(pageFeedback, {}),
      await ctx.plugin(toolFeedback, {}),
    ]
    // Reverse-order teardown (same pattern as smoke-composition.ts).
    plugins.reverse()
    disposers.push(...plugins.map(p => () => p.dispose()))
  })

  afterAll(async () => {
    for (const dispose of disposers.reverse()) await dispose()
    await testDb.dispose()
  })

  it('tool schemas match the golden snapshot', async () => {
    const schemas = ctx.tools.schemas()
      .map(schema => ({
        name: schema.name,
        description: schema.description,
        parameters: schema.parameters,
      }))
      .sort((a, b) => a.name.localeCompare(b.name))
    expect(schemas.length).toBeGreaterThan(0)

    const current = `${JSON.stringify(schemas, null, 2)}\n`
    const { mkdir, readFile, writeFile } = await import('node:fs/promises')
    if (process.env.UPDATE_SNAPSHOTS === '1') {
      await mkdir(path.dirname(fileURLToPath(GOLDEN)), { recursive: true })
      await writeFile(GOLDEN, current, 'utf8')
      return
    }
    const golden = await readFile(GOLDEN, 'utf8')
    expect(current, 'model-visible tool surface changed — review, then refresh with UPDATE_SNAPSHOTS=1').toBe(golden)
  })

  it('registers exactly the twenty-six Alioth tools', () => {
    const names = new Set(ctx.tools.schemas().map(s => s.name))
    for (const expected of [
      'alioth_app_list', 'alioth_app_inspect', 'alioth_app_write', 'alioth_app_configure', 'alioth_app_delete',
      'alioth_schema_info', 'alioth_schema_semantic_search', 'alioth_entity_write',
      'alioth_sources_scaffold',
      'alioth_workflow_step', 'alioth_workflow_complete', 'alioth_workflow_info', 'alioth_app_create',
      'alioth_workspace_current',
      'alioth_verify', 'alioth_closure', 'alioth_version', 'alioth_patch_assets',
      'alioth_capabilities', 'alioth_deferred', 'alioth_mapping_verdict', 'alioth_usage',
      'alioth_feedback_pending', 'alioth_feedback_ack', 'alioth_feedback_resolve', 'alioth_feedback_dismiss',
    ]) {
      expect(names, `tool ${expected} must be registered`).toContain(expected)
    }
    expect([...names].filter(n => n.startsWith('alioth_'))).toHaveLength(26)
  })
})
