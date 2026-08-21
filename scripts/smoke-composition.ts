/**
 * Composition smoke test: mounts the full Alioth plugin group (bundle) on a
 * real Context and verifies, in order:
 *   1. all five plugins register (the 8 model-facing tools are present)
 *   2. env-alioth ready() boots the builtin frozen model (zero network)
 *   3. one real tool call round-trips (schema_info entities)
 *   4. doctor reports the expected health state
 * Usage: node --import tsx scripts/smoke-composition.ts [--verbose]
 * Exit 0 = group works end to end.
 */
import { mkdtemp } from 'node:fs/promises'
import { tmpdir } from 'node:os'
import path from 'node:path'
import { Context } from '@deepseek-ai/cordis'
import SystemPrompt from '@deepseek-ai/dsh-system-prompt'
import ToolRuntime from '@deepseek-ai/dsh-tools'
import { CallId } from '@deepseek-ai/dsh-llm'
import * as envAlioth from '@dsh-alioth/env-alioth'
import * as toolAlioth from '@dsh-alioth/tool-alioth'
import * as toolMeta from '@dsh-alioth/tool-alioth-meta'
import * as workflow from '@dsh-alioth/tool-alioth-workflow'
import * as orchestrator from '@dsh-alioth/tool-alioth-orchestrator'
import * as authAlioth from '@dsh-alioth/auth-alioth'
import * as authWebAlioth from '@dsh-alioth/auth-web-alioth'
import * as landingAlioth from '@dsh-alioth/landing-alioth'
import * as billingAlioth from '@dsh-alioth/billing-alioth'
import * as billingWebAlioth from '@dsh-alioth/billing-web-alioth'
import * as feedbackAlioth from '@dsh-alioth/feedback-alioth'
import * as feedbackWebAlioth from '@dsh-alioth/feedback-web-alioth'
import * as toolFeedbackAlioth from '@dsh-alioth/tool-feedback-alioth'

const verbose = process.argv.includes('--verbose')
const log = (msg: string): void => { if (verbose) console.log(msg) }

const EXPECTED_TOOLS = [
  'alioth_app_list', 'alioth_app_inspect', 'alioth_app_write', 'alioth_app_configure', 'alioth_app_delete',
  'alioth_schema_info', 'alioth_schema_semantic_search', 'alioth_entity_write',
  'alioth_workflow_step', 'alioth_workflow_complete', 'alioth_workflow_info', 'alioth_app_create',
  'alioth_workspace_current',
  'alioth_feedback_pending', 'alioth_feedback_ack', 'alioth_feedback_resolve', 'alioth_feedback_dismiss',
]

const ctx = new Context()
const disposers: Array<() => Promise<void>> = []
const dataRoot = await mkdtemp(path.join(tmpdir(), 'smoke-data-'))
const preProcRoot = await mkdtemp(path.join(tmpdir(), 'smoke-preproc-'))

try {
  const system = await ctx.plugin(SystemPrompt)
  disposers.push(() => system.dispose())
  const tools = await ctx.plugin(ToolRuntime)
  disposers.push(() => tools.dispose())
  const env = await ctx.plugin(envAlioth, {
    modelSource: 'builtin',
    dataRoot,
    ...(process.env.ALIOTH_DATABASE_URL === undefined ? {} : { databaseUrl: process.env.ALIOTH_DATABASE_URL }),
  })
  disposers.push(() => env.dispose())
  const appTool = await ctx.plugin(toolAlioth, { preProcRoot })
  disposers.push(() => appTool.dispose())
  const meta = await ctx.plugin(toolMeta, {})
  disposers.push(() => meta.dispose())
  const wf = await ctx.plugin(workflow, { preProcRoot })
  disposers.push(() => wf.dispose())
  const orch = await ctx.plugin(orchestrator, {})
  disposers.push(() => orch.dispose())
  const landingPlugin = await ctx.plugin(landingAlioth, {})
  disposers.push(() => landingPlugin.dispose())
  const authPlugin = await ctx.plugin(authAlioth, { mode: 'open' })
  disposers.push(() => authPlugin.dispose())
  const authWebPlugin = await ctx.plugin(authWebAlioth, { port: 3902 })
  disposers.push(() => authWebPlugin.dispose())
  const billingPlugin = await ctx.plugin(billingAlioth, {})
  disposers.push(() => billingPlugin.dispose())
  const billingWebPlugin = await ctx.plugin(billingWebAlioth, {})
  disposers.push(() => billingWebPlugin.dispose())
  const fbStore = await ctx.plugin(feedbackAlioth, {})
  disposers.push(() => fbStore.dispose())
  const fbWeb = await ctx.plugin(feedbackWebAlioth, { port: 14747 })
  disposers.push(() => fbWeb.dispose())
  const fbTools = await ctx.plugin(toolFeedbackAlioth, {})
  disposers.push(() => fbTools.dispose())


  // 1. Tool registration
  const registered = new Set(ctx.tools.schemas().map(schema => schema.name))
  const missing = EXPECTED_TOOLS.filter(name => !registered.has(name))
  if (missing.length > 0) {
    throw new Error(`tool registration failed — missing: ${missing.join(', ')}`)
  }
  log(`tools registered: ${EXPECTED_TOOLS.length}/${EXPECTED_TOOLS.length}`)

  // 2. Env ready (builtin, zero network)
  const info = await ctx.aliothEnv.ready()
  if (!info.sourceRef.startsWith('builtin-v')) {
    throw new Error(`expected builtin model, got ${info.sourceRef}`)
  }
  log(`env ready: ${info.sourceRef} @ model ${info.modelVersion}`)

  // 3. Real tool call round-trip
  const result = await ctx.tools.execute({
    signal: new AbortController().signal,
    callId: CallId('smoke-schema-info'),
    name: 'alioth_schema_info',
    arguments: { action: 'entities', limit: 5 },
  })
  if (result.isError) {
    throw new Error(`schema_info round-trip failed: ${result.error.message}`)
  }
  const entities = (result.value as { entities?: unknown[] }).entities
  if (!Array.isArray(entities) || entities.length === 0) {
    throw new Error('schema_info returned no entities — registry empty?')
  }
  log(`schema_info round-trip: ${entities.length} entities`)

  // 4. Doctor health
  const report = await ctx.aliothEnv.doctor()
  const coreOk = ['model-snapshot', 'database', 'isahl-meta', 'model-stamp']
    .every(name => report.checks.find(check => check.name === name)?.ok === true)
  if (!coreOk) {
    throw new Error(`doctor core checks not green: ${JSON.stringify(report.checks)}`)
  }
  log(`doctor: core green (semantic-index=${report.checks.find(c => c.name === 'semantic-index')?.ok})`)

  console.log(`SMOKE PASS: group mounted, ${EXPECTED_TOOLS.length} tools registered, builtin env ready, tool round-trip ok, doctor core green`)
} finally {
  for (const dispose of disposers.reverse()) {
    await dispose().catch(() => {})
  }
  await import('node:fs/promises').then(fs => Promise.all([
    fs.rm(dataRoot, { recursive: true, force: true }),
    fs.rm(preProcRoot, { recursive: true, force: true }),
  ]))
}
