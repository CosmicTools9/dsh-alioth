import { describe, expect, it, beforeAll, afterAll } from 'vitest'
import { mkdir, mkdtemp, readFile, rm, writeFile } from 'node:fs/promises'
import { tmpdir } from 'node:os'
import path from 'node:path'
import { Context } from '@deepseek-ai/cordis'
import SystemPrompt from '@deepseek-ai/dsh-system-prompt'
import ToolRuntime from '@deepseek-ai/dsh-tools'
import { CallId } from '@deepseek-ai/dsh-llm'
import * as envAlioth from '@dsh-alioth/env-alioth'
import * as toolAlioth from '@dsh-alioth/tool-alioth'
import * as toolMeta from '@dsh-alioth/tool-alioth-meta'
import * as orchestrator from '../src/index.ts'
import * as workflowTool from '@dsh-alioth/tool-alioth-workflow'

const signal = new AbortController().signal

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
    callId: CallId(`create-${++counter}`),
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
  const orchestration = await ctx.plugin(orchestrator, {})
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
  })

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
  })

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

  it('runs the workflow gate after writing artifacts', async () => {
    // app_write generates the artifacts; the workflow gate then verifies them.
    workflowPreProc = await mkdtemp(path.join(tmpdir(), 'ptc-wf-preproc-'))

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
    const wf = await workflowCtx.plugin(workflowTool, { preProcRoot: workflowPreProc })
    workflowDisposers.push(() => wf.dispose())
    const orchestration = await workflowCtx.plugin(orchestrator, { adapter: 'alioth-app.yaml' })
    workflowDisposers.push(() => orchestration.dispose())

    const result = await workflowCtx.tools.execute({
      signal,
      callId: CallId('create-wf'),
      name: 'alioth_app_create',
      arguments: { namespace: 'Demo', code: 'wf-app', name: 'WF 应用', modules: [{ id: 'm1', name: 'M1' }] },
    })
    if (result.isError) throw new Error(`expected alioth_app_create success: ${result.error.message}`)
    expect(result.value).toMatchObject({ verified: true, workflowGate: 'step 1.1 passed' })
  }, 120_000)

  afterAll(async () => {
    for (const dispose of workflowDisposers.reverse()) {
      await dispose().catch(() => {})
    }
    await rm(workflowPreProc, { recursive: true, force: true }).catch(() => {})
  })
})
