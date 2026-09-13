import { describe, expect, it, beforeAll, afterAll } from 'vitest'
import { mkdir, mkdtemp, readFile, rm, writeFile } from 'node:fs/promises'
import { tmpdir } from 'node:os'
import path from 'node:path'
import { Context } from '@deepseek-ai/cordis'
import SystemPrompt from '@deepseek-ai/dsh-system-prompt'
import ToolRuntime, { type ToolExecutionToken, type ToolRunContext } from '@deepseek-ai/dsh-tools'
import { ToolCallId } from '@deepseek-ai/dsh-llm'
import * as envAlioth from '@dsh-alioth/env-alioth'
import * as toolAlioth from '@dsh-alioth/tool-alioth'
import * as toolMeta from '@dsh-alioth/tool-alioth-meta'
import * as orchestrator from '../src/index.ts'
import * as workflowTool from '@dsh-alioth/tool-alioth-workflow'
import type { Agent } from '@deepseek-ai/dsh-agent'
import { buildPlan, buildPrimitives } from '../src/primitives.ts'

const signal = new AbortController().signal

  /**
 * Stand-in run context for the stage guards. The primitives read only
 * `signal`/`agent`; the registry-owned members (`token` is a branded symbol,
 * plus nested-dispatch bookkeeping) cannot exist outside the tool runtime, so
 * they are stubbed here — the single place a cast is warranted in this spec.
 */
function stageExec(callId: string, agent?: Agent): ToolRunContext {
  const id = ToolCallId(callId)
  return {
    callId: id,
    rootCallId: id,
    name: 'alioth_app_create',
    arguments: {},
    signal,
    token: Symbol(callId) as ToolExecutionToken,
    deferContext: () => {},
    concludeTurn: () => {},
    ...(agent === undefined ? {} : { agent }),
  }
}


const SCHEMA_DDL = `
CREATE TYPE isahl_meta.collection_type AS ENUM ('table', 'view');
CREATE TYPE isahl_meta.field_category AS ENUM ('scalar', 'reference', 'computed', 'auto');
CREATE TYPE isahl_meta.field_data_type AS ENUM ('text', 'decimal', 'bigint');
CREATE TABLE isahl_meta.meta_collections (
    table_name text NOT NULL,
    name text NOT NULL,
    type isahl_meta.collection_type,
    config jsonb DEFAULT '{}'::jsonb,
    data_source text,
    schema text DEFAULT 'isahl'::text,
    biz_description text,
    PRIMARY KEY (table_name)
);
CREATE TABLE isahl_meta.meta_fields (
    fk_collection text NOT NULL REFERENCES isahl_meta.meta_collections(table_name) ON DELETE CASCADE,
    name text NOT NULL,
    category isahl_meta.field_category,
    data_type isahl_meta.field_data_type,
    is_required boolean DEFAULT false,
    default_value text,
    config jsonb DEFAULT '{}'::jsonb,
    title text NOT NULL DEFAULT ''::text,
    PRIMARY KEY (fk_collection, name)
);
`

let ctx: Context
const disposers: Array<() => Promise<void>> = []
let preProcRoot: string
let counter = 0

function callCreate(args: unknown) {
  return ctx.tools.execute({
    signal,
    callId: ToolCallId(`create-${++counter}`),
    name: 'alioth_app_create',
    arguments: args,
  })
}

beforeAll(async () => {
  const modelDir = await mkdtemp(path.join(tmpdir(), 'ptc-model-'))
  const dataRoot = await mkdtemp(path.join(tmpdir(), 'ptc-data-'))
  preProcRoot = await mkdtemp(path.join(tmpdir(), 'ptc-preproc-'))
  await mkdir(path.join(modelDir, 'backend', 'ddl'), { recursive: true })
  await mkdir(path.join(modelDir, 'backend', 'vendor', 'alioth-gen', 'src'), { recursive: true })
  await mkdir(path.join(modelDir, 'skill-adapters'), { recursive: true })
  await mkdir(path.join(modelDir, 'Pre-Proc', 'Alioth', '_schema'), { recursive: true })
  await writeFile(path.join(modelDir, 'backend', 'ddl', '002_isahl_meta_schema.sql'), SCHEMA_DDL)
  await writeFile(path.join(modelDir, 'skill-adapters', 'a.yaml'), 'x\n')
  await writeFile(path.join(modelDir, 'Pre-Proc', 'Alioth', '_schema', 'a.schema.json'), '{}\n')
  await writeFile(
    path.join(modelDir, 'backend', 'vendor', 'alioth-gen', 'src', 'lib.rs'),
    'pub static ALIOTH_MODEL_VERSION: LazyLock<String> =\n    LazyLock::new(|| env::var("MODEL_VERSION").unwrap_or_else(|_| "10.0.0".to_string()));\n',
  )

  ctx = new Context()
  const system = await ctx.plugin(SystemPrompt)
  disposers.push(() => system.dispose())
  const tools = await ctx.plugin(ToolRuntime)
  disposers.push(() => tools.dispose())
  const env = await ctx.plugin(envAlioth, { modelSource: modelDir, dataRoot })
  disposers.push(() => env.dispose())
  const appTool = await ctx.plugin(toolAlioth, { preProcRoot })
  disposers.push(() => appTool.dispose())
  const meta = await ctx.plugin(toolMeta, {})
  disposers.push(() => meta.dispose())
  const orchestration = await ctx.plugin(orchestrator, { preProcRoot })
  disposers.push(() => orchestration.dispose())
}, 120_000)

afterAll(async () => {
  for (const dispose of disposers.reverse()) {
    await dispose().catch(() => {})
  }
  await rm(preProcRoot, { recursive: true, force: true })
})

describe('alioth_app_create (PTC orchestrator)', () => {
  it('runs the full pipeline: entity register → artifact write → verify', async () => {
    const result = await callCreate({
      namespace: 'Demo',
      code: 'ptc-app',
      name: 'PTC 应用',
      modules: [{ id: 'inventory', name: '库存' }],
      blocks: [],
      entities: [{
        table: 'zc_id_deta-bill-check',
        name: '账单核查',
        inherits: ['zc_id_object'],
        coordinates: { scene: 'CA', factor: 'GBA', function: '↑_AA' },
        fields: [
          { name: 'notice', category: 'scalar', dataType: 'text', title: '名称', required: true },
        ],
      }],
    })
    if (result.isError) throw new Error(`expected alioth_app_create success: ${result.error.message}`)
    expect(result.value).toMatchObject({
      namespace: 'Demo',
      code: 'ptc-app',
      entitiesRegistered: 1,
      filesWritten: 6,
      verified: true,
    })

    // The entity and the app are both on disk / in the registry.
    const appJson = await readFile(path.join(preProcRoot, 'Demo', 'Apps', 'ptc-app', 'app.json'), 'utf8')
    expect(appJson).toContain('"code": "ptc-app"')
  }, 120_000)

  it('fails atomically before writing when an entity definition is invalid', async () => {
    const result = await callCreate({
      namespace: 'Demo',
      code: 'ptc-broken',
      name: '坏应用',
      modules: [{ id: 'm1', name: 'M1' }],
      entities: [{
        table: 'zc_id_deta-bill-check',
        name: '重复注册',
        inherits: [],
        fields: [],
      }],
    })
    if (!result.isError) throw new Error('expected alioth_app_create failure')
    expect(result.error.message).toContain('alioth_entity_write')

    // Nothing written: the app does not exist.
    await expect(
      readFile(path.join(preProcRoot, 'Demo', 'Apps', 'ptc-broken', 'app.json'), 'utf8'),
    ).rejects.toThrow()
  }, 120_000)

  it('creates an app with no new entities', async () => {
    const result = await callCreate({
      namespace: 'Demo',
      code: 'ptc-plain',
      name: '普通应用',
      modules: [{ id: 'orders', name: '订单' }],
    })
    if (result.isError) throw new Error(`expected alioth_app_create success: ${result.error.message}`)
    expect(result.value).toMatchObject({ entitiesRegistered: 0, verified: true })
  })
})

describe('alioth_app_create with workflow adapter', () => {
  let workflowCtx: Context
  const workflowDisposers: Array<() => Promise<void>> = []
  let workflowPreProc: string
let workflowContentRoot: string

  it('runs the workflow gate after writing artifacts', async () => {
    // app_write generates the artifacts; the workflow gate then verifies them.
    workflowPreProc = await mkdtemp(path.join(tmpdir(), 'ptc-wf-preproc-'))
    workflowContentRoot = await mkdtemp(path.join(tmpdir(), 'ptc-wf-content-'))

    const modelDir = await mkdtemp(path.join(tmpdir(), 'ptc-wf-model-'))
    const dataRoot = await mkdtemp(path.join(tmpdir(), 'ptc-wf-data-'))
    await mkdir(path.join(modelDir, 'backend', 'ddl'), { recursive: true })
    await mkdir(path.join(modelDir, 'backend', 'vendor', 'alioth-gen', 'src'), { recursive: true })
    await mkdir(path.join(modelDir, 'skill-adapters'), { recursive: true })
    await mkdir(path.join(modelDir, 'Pre-Proc', 'Alioth', '_schema'), { recursive: true })
    await writeFile(path.join(modelDir, 'backend', 'ddl', '002_isahl_meta_schema.sql'), SCHEMA_DDL)
    await writeFile(path.join(modelDir, 'skill-adapters', 'alioth-app.yaml'), `
name: alioth-app
version: "2.0"
tracks:
  - name: 构建
    steps:
      - id: "1.1"
        instruction: "preflight"
        gates:
          - output_glob: "Pre-Proc/{ns}/Apps/{app}/app.json"
`)
    await writeFile(path.join(modelDir, 'Pre-Proc', 'Alioth', '_schema', 'a.schema.json'), '{}\n')
    await writeFile(
      path.join(modelDir, 'backend', 'vendor', 'alioth-gen', 'src', 'lib.rs'),
      'pub static ALIOTH_MODEL_VERSION: LazyLock<String> =\n    LazyLock::new(|| env::var("MODEL_VERSION").unwrap_or_else(|_| "10.0.0".to_string()));\n',
    )

    workflowCtx = new Context()
    const system = await workflowCtx.plugin(SystemPrompt)
    workflowDisposers.push(() => system.dispose())
    const tools = await workflowCtx.plugin(ToolRuntime)
    workflowDisposers.push(() => tools.dispose())
    const env = await workflowCtx.plugin(envAlioth, { modelSource: modelDir, dataRoot })
    workflowDisposers.push(() => env.dispose())
    const appTool = await workflowCtx.plugin(toolAlioth, { preProcRoot: workflowPreProc })
    workflowDisposers.push(() => appTool.dispose())
    const meta = await workflowCtx.plugin(toolMeta, {})
    workflowDisposers.push(() => meta.dispose())
    const wf = await workflowCtx.plugin(workflowTool, { preProcRoot: workflowPreProc, contentRoot: workflowContentRoot })
    workflowDisposers.push(() => wf.dispose())
    const orchestration = await workflowCtx.plugin(orchestrator, { adapter: 'alioth-app.yaml', preProcRoot: workflowPreProc })
    workflowDisposers.push(() => orchestration.dispose())

    const result = await workflowCtx.tools.execute({
      signal,
      callId: ToolCallId('create-wf'),
      name: 'alioth_app_create',
      arguments: { namespace: 'Demo', code: 'wf-app', name: 'WF 应用', modules: [{ id: 'm1', name: 'M1' }] },
    })
    if (result.isError) throw new Error(`expected alioth_app_create success: ${result.error.message}`)
    expect(result.value).toMatchObject({ verified: true, workflowGate: 'step 1.1 passed' })
  }, 120_000)

  it('reports the workflow gate as finished when the run already completed', async () => {
    const args = {
      namespace: 'Demo',
      code: 'wf-finished',
      name: 'WF 完成态',
      modules: [{ id: 'm1', name: 'M1' }],
    }
    const primitives = buildPrimitives(
      workflowCtx,
      stageExec('wf-finished-exec'),
      args,
      'alioth-app.yaml',
      workflowPreProc,
    )
    const created = await primitives.appCreation('wf finished')
    expect(created.evidence).toContain('Demo/wf-finished')

    // Drive the single-step track to its end: its only gate is app.json.
    const completed = await workflowCtx.tools.execute({
      signal,
      callId: ToolCallId('wf-finished-step'),
      name: 'alioth_workflow_complete',
      arguments: { namespace: 'Demo', app: 'wf-finished' },
    })
    if (completed.isError) throw new Error(`expected the workflow gate to pass: ${completed.error.message}`)
    expect(completed.value).toMatchObject({ finished: true })

    // Publishing then reports the finished run instead of stepping again.
    const published = await primitives.publishing(buildPlan(args), 1)
    expect(published.output.evidence).toContain('workflow=finished')
    expect(published.result.runtimeValidation?.checks[1]).toMatchObject({
      name: 'workflow-gate',
      ok: true,
      detail: 'finished',
    })
  }, 120_000)

  afterAll(async () => {
    for (const dispose of workflowDisposers.reverse()) {
      await dispose().catch(() => {})
    }
    await rm(workflowPreProc, { recursive: true, force: true }).catch(() => {})
  })
})

// ── Terminal states and stage guards ───────────────────────────────────────
// The happy path is covered above; these pin the failure and degradation
// branches: a terminal GATE-FAIL, preflight atomicity, the stage guards that
// keep a pipeline alive when an auxiliary tool is unavailable or an evidence
// file cannot land, and the E2E-root fallback chain.

describe('alioth_app_create terminal states', () => {
  it('fails the metadata gate when a declared block has no artifact', async () => {
    const result = await callCreate({
      namespace: 'Demo',
      code: 'ptc-gate-fail',
      name: '门禁失败应用',
      modules: [{ id: 'gate', name: '门禁' }],
      blocks: ['block-not-written'],
    })
    if (!result.isError) throw new Error('expected alioth_app_create failure')
    expect(result.error.message).toContain('pipeline failed at')
    expect(result.error.message).toContain('gate block-extract failed')

    // The artifacts stay on disk: the repair loop fixes them and re-runs.
    const appJson = await readFile(path.join(preProcRoot, 'Demo', 'Apps', 'ptc-gate-fail', 'app.json'), 'utf8')
    expect(appJson).toContain('"code": "ptc-gate-fail"')
  }, 120_000)
})

describe('buildPrimitives stage guards', () => {

  it('registers coordinates and reference fields declared inline', async () => {
    const args = {
      namespace: 'Demo',
      code: 'cover-entities',
      name: '实体覆盖',
      modules: [{ id: 'cover', name: '覆盖' }],
      entities: [{
        table: 'zc_id_appr-authorization',
        name: '审批授权',
        category: 'table',
        coordinates: { scene: 'CA', factor: 'GBA', function: '↑_AA' },
        fields: [
          { name: 'place', category: 'reference', dataType: 'bigint', title: '地点', required: true, targetTable: 'zc_id_place', localKey: 'fk_place' },
          { name: 'link', category: 'reference', dataType: 'bigint', targetTable: 'zc_id_place', junctionTable: 'zc_id_object' },
          { name: 'notice', category: 'scalar', dataType: 'text' },
        ],
      }, {
        // No fields declared at all: an empty registry row, not a crash.
        table: 'zc_id_appr-org-structure',
        name: '组织架构',
      }],
    }
    const primitives = buildPrimitives(ctx, stageExec('cover-entities'), args, undefined, preProcRoot)
    const created = await primitives.appCreation('cover entities')
    expect(created.evidence).toContain('Demo/cover-entities')
    expect(created.artifacts).toContain('app.json')

    const registered = await primitives.ontologyAnalysis(1, buildPlan(args))
    expect(registered.artifacts).toEqual(['zc_id_appr-authorization', 'zc_id_appr-org-structure'])

    // The inline reference declaration landed in the registry: category,
    // NOT NULL flag, title and the reference targets per field.
    const readback = await ctx.aliothEnv.sql<{ row: string }>(
      `SELECT f.name || '|' || f.category::text || '|' || f.is_required::text || '|' || f.title
              || '|' || coalesce(f.config->'reference_config'->>'targetTable', '')
              || '|' || coalesce(f.config->'reference_config'->>'localKey', '')
              || '|' || coalesce(f.config->'reference_config'->>'junctionTable', '') AS row
       FROM isahl_meta.meta_fields f
       WHERE f.fk_collection = 'zc_id_appr-authorization' ORDER BY f.name`,
    )
    expect(readback.rows.map(entry => entry.row)).toEqual([
      'link|reference|false||zc_id_place||zc_id_object',
      'notice|scalar|false||||',
      'place|reference|true|地点|zc_id_place|fk_place|',
    ])
  }, 120_000)

  it('rejects a reference field that declares no target table', async () => {
    const args = {
      namespace: 'Demo',
      code: 'cover-badref',
      name: '坏引用',
      modules: [{ id: 'cover', name: '覆盖' }],
      entities: [{
        table: 'zc_id_appr-bid-evaluation',
        name: '投标评审',
        fields: [{ name: 'place', category: 'reference', dataType: 'bigint', localKey: 'fk_place' }],
      }],
    }
    const primitives = buildPrimitives(ctx, stageExec('cover-badref'), args, undefined, preProcRoot)
    await expect(primitives.appCreation('bad reference')).rejects.toThrow('target table "" not found')
    // Atomic: the preflight runs before any artifact write.
    await expect(readFile(path.join(preProcRoot, 'Demo', 'Apps', 'cover-badref', 'app.json'), 'utf8')).rejects.toThrow()
  }, 120_000)

  it('treats a registry row whose inherits is not an array as a root entity', async () => {
    // A hand-repaired registry row (scalar `inherits`) must not break preflight.
    await ctx.aliothEnv.sql(
      `INSERT INTO isahl_meta.meta_collections (table_name, name, config)
       VALUES ('zc_id_probe-scalar', '探针集合', '{"inherits": "zc_id_object"}'::jsonb)`,
    )
    const args = {
      namespace: 'Demo',
      code: 'cover-scalar-parent',
      name: '标量父类',
      modules: [{ id: 'cover', name: '覆盖' }],
      entities: [{
        table: 'zc_id_appr-damage',
        name: '损害记录',
        inherits: ['zc_id_probe-scalar'],
        category: 'table',
        fields: [{ name: 'notice', category: 'scalar', dataType: 'text' }],
      }],
    }
    const primitives = buildPrimitives(ctx, stageExec('cover-scalar'), args, undefined, preProcRoot)
    const created = await primitives.appCreation('scalar parent')
    expect(created.evidence).toContain('cover-scalar-parent')

    const registered = await primitives.ontologyAnalysis(1, buildPlan(args))
    expect(registered.artifacts).toEqual(['zc_id_appr-damage'])
  }, 120_000)

  it('accepts the alignment preconditions when the search tool is unavailable', async () => {
    // A deployment without the schema tools degrades to the parameters the
    // dialogue already aligned — the stage is a confirmation, not a gate.
    const bareCtx = new Context()
    const system = await bareCtx.plugin(SystemPrompt)
    const tools = await bareCtx.plugin(ToolRuntime)
    try {
      const primitives = buildPrimitives(
        bareCtx,
        stageExec('cover-bare'),
        { namespace: 'Demo', code: 'bare-app', name: '空部署', modules: [] },
        undefined,
        undefined,
      )
      const output = await primitives.semanticAnalysis('对齐后的需求')
      expect(output.evidence).toContain('semantic search unavailable')
      expect(output.evidence).toContain('alignment preconditions accepted from parameters')
    } finally {
      await tools.dispose().catch(() => {})
      await system.dispose().catch(() => {})
    }
  }, 120_000)

  it('passthrough of the calling agent survives into the pipeline tools', async () => {
    const args = { namespace: 'Demo', code: 'cover-agent', name: '带代理', modules: [{ id: 'cover', name: '覆盖' }] }
    const probeAgent = { id: 'probe' } as unknown as Agent
    const primitives = buildPrimitives(
      ctx,
      { ...stageExec('cover-agent'), agent: probeAgent },
      args,
      undefined,
      preProcRoot,
    )
    const created = await primitives.appCreation('with agent')
    expect(created.evidence).toContain('Demo/cover-agent')
  }, 120_000)

  it('writes the E2E failure report when the artifacts are incomplete', async () => {
    const args = { namespace: 'Demo', code: 'cover-e2e', name: 'E2E 覆盖', modules: [] }
    const primitives = buildPrimitives(ctx, stageExec('cover-e2e'), args, undefined, preProcRoot)
    const output = await primitives.e2eVerification(2, buildPlan(args))
    expect(output.evidence).toContain('E2E failed (attempt 2)')
    expect(output.evidence).toContain('app.json=false, module.json=false')

    const report: unknown = JSON.parse(await readFile(path.join(preProcRoot, 'Demo', 'Apps', 'cover-e2e', 'e2e-report.json'), 'utf8'))
    expect(report).toMatchObject({
      app: 'cover-e2e',
      namespace: 'Demo',
      attempt: 2,
      passed: false,
      failures: [{ id: 'app-json' }, { id: 'module-json' }],
    })
  }, 120_000)

  it('keeps the artifact list honest when the E2E evidence file cannot land', async () => {
    // A file in the path of the evidence directory: every write fails.
    const blocker = path.join(await mkdtemp(path.join(tmpdir(), 'ptc-blocker-')), 'file')
    await writeFile(blocker, 'not a directory\n')
    const args = { namespace: 'Demo', code: 'cover-e2e-write', name: 'E2E 写失败', modules: [{ id: 'cover', name: '覆盖' }] }
    const primitives = buildPrimitives(ctx, stageExec('cover-e2e-write'), args, undefined, path.join(blocker, 'sub'))

    // Incomplete artifacts AND no evidence file: the failure is still reported.
    const failed = await primitives.e2eVerification(1, buildPlan(args))
    expect(failed.evidence).toContain('E2E failed (attempt 1)')
    expect(failed.evidence).toContain('evidence report write failed')

    // Complete artifacts AND no evidence file: keep the artifacts, no bogus path.
    const created = await primitives.appCreation('write failure')
    const passed = await primitives.e2eVerification(1, buildPlan(args))
    expect(passed.evidence).toContain('E2E verification (attempt 1): artifacts complete')
    expect(passed.evidence).toContain('evidence report write failed')
    expect(passed.artifacts).toEqual(created.artifacts)
  }, 120_000)

  it('anchors the E2E report under ALIOTH_PRE_PROC_ROOT when no root is configured', async () => {
    const envRoot = await mkdtemp(path.join(tmpdir(), 'ptc-envroot-'))
    const previous = process.env.ALIOTH_PRE_PROC_ROOT
    process.env.ALIOTH_PRE_PROC_ROOT = envRoot
    try {
      const args = { namespace: 'Demo', code: 'cover-env-root', name: '环境根', modules: [] }
      const primitives = buildPrimitives(ctx, stageExec('cover-env-root'), args, undefined, undefined)
      const output = await primitives.e2eVerification(1, buildPlan(args))
      expect(output.evidence).toContain('E2E failed (attempt 1)')

      const report: unknown = JSON.parse(await readFile(path.join(envRoot, 'Demo', 'Apps', 'cover-env-root', 'e2e-report.json'), 'utf8'))
      expect(report).toMatchObject({ app: 'cover-env-root', attempt: 1, passed: false })
    } finally {
      if (previous === undefined) {
        delete process.env.ALIOTH_PRE_PROC_ROOT
      } else {
        process.env.ALIOTH_PRE_PROC_ROOT = previous
      }
    }
  }, 120_000)

  it('falls back to the conventional Pre-Proc tree under $HOME', async () => {
    const home = await mkdtemp(path.join(tmpdir(), 'ptc-home-'))
    const previousHome = process.env.HOME
    const previousRoot = process.env.ALIOTH_PRE_PROC_ROOT
    process.env.HOME = home
    delete process.env.ALIOTH_PRE_PROC_ROOT
    try {
      const args = { namespace: 'Demo', code: 'cover-home-root', name: '主目录根', modules: [] }
      const primitives = buildPrimitives(ctx, stageExec('cover-home-root'), args, undefined, undefined)
      const output = await primitives.e2eVerification(1, buildPlan(args))
      expect(output.evidence).toContain('E2E failed (attempt 1)')

      const reportPath = path.join(home, '.dsh-alioth', 'Pre-Proc', 'Demo', 'Apps', 'cover-home-root', 'e2e-report.json')
      const report: unknown = JSON.parse(await readFile(reportPath, 'utf8'))
      expect(report).toMatchObject({ app: 'cover-home-root', passed: false })
    } finally {
      if (previousHome === undefined) {
        delete process.env.HOME
      } else {
        process.env.HOME = previousHome
      }
      if (previousRoot === undefined) {
        delete process.env.ALIOTH_PRE_PROC_ROOT
      } else {
        process.env.ALIOTH_PRE_PROC_ROOT = previousRoot
      }
    }
  }, 120_000)

  it('fails an unknown metadata gate and confirms a human gate deterministically', async () => {
    const args = { namespace: 'Demo', code: 'cover-gates', name: '门禁', modules: [{ id: 'cover', name: '覆盖' }] }
    const primitives = buildPrimitives(ctx, stageExec('cover-gates'), args, undefined, preProcRoot)

    const unknown = await primitives.pipelineAdvance('not-a-stage', buildPlan(args))
    expect(unknown.evidence).toBe('GATE-FAIL not-a-stage: artifact missing')
    expect(unknown.artifacts).toBeUndefined()

    expect(await primitives.resolveGate('human-review', 'approve?')).toBe('confirm')
  }, 120_000)

  it('refuses to publish an app whose artifact lacks the required fields', async () => {
    const appDir = path.join(preProcRoot, 'Demo', 'Apps', 'cover-incomplete')
    await mkdir(appDir, { recursive: true })
    await writeFile(path.join(appDir, 'app.json'), '{}\n')
    const args = { namespace: 'Demo', code: 'cover-incomplete', name: '不完整', modules: [{ id: 'cover', name: '覆盖' }] }
    const primitives = buildPrimitives(ctx, stageExec('cover-incomplete'), args, undefined, preProcRoot)

    const published = await primitives.publishing(buildPlan(args), 1)
    expect(published.result.runtimeValidation).toMatchObject({ valid: false })
    expect(published.result.runtimeValidation?.checks[0]?.detail)
      .toBe('missing: id, code, namespace, name, version, config')
    expect(published.result.runtimeValidation?.checks[1]).toMatchObject({ name: 'workflow-gate', ok: true, detail: 'not-configured' })
    expect(published.output.evidence).toContain('verified=false, workflow=not-configured')
  }, 120_000)
})
