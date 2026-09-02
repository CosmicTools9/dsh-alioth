import { describe, expect, it, beforeAll, afterAll } from 'vitest'
import { mkdir, mkdtemp, rm, writeFile } from 'node:fs/promises'
import { tmpdir } from 'node:os'
import path from 'node:path'
import { Context } from '@deepseek-ai/cordis'
import SystemPrompt from '@deepseek-ai/dsh-system-prompt'
import ToolRuntime from '@deepseek-ai/dsh-tools'
import { ToolCallId } from '@deepseek-ai/dsh-llm'
import { Session, SessionId } from '@deepseek-ai/dsh-session'
import * as envAlioth from '@dsh-alioth/env-alioth'
import * as toolMeta from '../src/index.ts'

const signal = new AbortController().signal

/** Registry DDL mirroring the real `002_isahl_meta_schema.sql` column shapes. */
const SCHEMA_DDL = `
CREATE TYPE isahl_meta.collection_type AS ENUM ('table', 'view');
CREATE TYPE isahl_meta.field_category AS ENUM ('scalar', 'reference', 'computed', 'auto');
CREATE TYPE isahl_meta.field_data_type AS ENUM ('text', 'decimal', 'bigint');
CREATE TABLE isahl_meta.meta_collections (
    table_name      text                        NOT NULL,
    created_at      timestamptz                 NOT NULL DEFAULT now(),
    updated_at      timestamptz                 NOT NULL DEFAULT now(),
    created_by_id   bigint       DEFAULT 1,
    updated_by_id   bigint       DEFAULT 1,
    name            text                        NOT NULL,
    type            isahl_meta.collection_type,
    config          jsonb        DEFAULT '{}'::jsonb,
    data_source     text,
    schema          text         DEFAULT 'isahl'::text,
    biz_description text
);
ALTER TABLE isahl_meta.meta_collections ADD PRIMARY KEY (table_name);
CREATE TABLE isahl_meta.meta_fields (
    fk_collection text NOT NULL REFERENCES isahl_meta.meta_collections(table_name) ON DELETE CASCADE,
    name          text NOT NULL,
    created_at    timestamptz NOT NULL DEFAULT now(),
    updated_at    timestamptz NOT NULL DEFAULT now(),
    created_by_id bigint DEFAULT 1,
    updated_by_id bigint DEFAULT 1,
    category      isahl_meta.field_category,
    data_type     isahl_meta.field_data_type,
    is_required   boolean DEFAULT false,
    default_value text,
    config        jsonb DEFAULT '{}'::jsonb,
    title         text NOT NULL DEFAULT ''::text
);
ALTER TABLE isahl_meta.meta_fields ADD PRIMARY KEY (fk_collection, name);
`

const SEEDS_DDL = `
INSERT INTO isahl_meta.meta_collections (table_name, name, type, config, biz_description) VALUES
 ('zc_id_inventory', '库存', 'table', '{"depth": 3, "source": "sync_from_database", "category": {"name": "交易信息", "sort": 11, "color": "blue"}, "inherits": ["zc_id_object"]}', '库存台账'),
 ('zc_id_demand', '需求', 'table', '{"depth": 3, "category": {"name": "协作"}, "inherits": ["zc_id_object"]}', NULL),
 ('zc_ad_dimension', '抽象-维度', 'table', '{"depth": 3, "category": {"name": "抽象结构"}, "inherits": ["zc_ad_vector"]}', NULL),
 ('zc_id_task-testing', '任务-测试', 'table', '{"depth": 3, "category": {"name": "交易过程"}, "inherits": ["zc_id_task"]}', NULL),
 ('zc_id_oper-test', '操作-测试', 'table', '{"depth": 3, "category": {"name": "交易过程"}, "inherits": ["zc_id_oper"]}', NULL);
INSERT INTO isahl_meta.meta_fields (fk_collection, name, category, data_type, is_required, title) VALUES
 ('zc_id_inventory', 'name', 'scalar', 'text', true, '名称'),
 ('zc_id_inventory', 'owner_qk_user', 'reference', 'bigint', false, '负责人'),
 ('zc_id_inventory', 'qty', 'scalar', 'decimal', false, '数量'),
 ('zc_id_demand', 'title', 'scalar', 'text', true, '标题'),
 ('zc_ad_dimension', 'name', 'scalar', 'text', false, '维度名'),
 ('zc_id_task-testing', 'name', 'scalar', 'text', false, '测试名');
`

let ctx: Context
const disposers: Array<() => Promise<void>> = []
let counter = 0

function callSchemaInfo(args: unknown) {
  return ctx.tools.execute({
    signal,
    callId: ToolCallId(`call-${++counter}`),
    name: 'alioth_schema_info',
    arguments: args,
  })
}

beforeAll(async () => {
  const modelDir = await mkdtemp(path.join(tmpdir(), 'dsh-alioth-meta-model-'))
  const dataRoot = await mkdtemp(path.join(tmpdir(), 'dsh-alioth-meta-data-'))
  await mkdir(path.join(modelDir, 'backend', 'ddl'), { recursive: true })
  await mkdir(path.join(modelDir, 'backend', 'vendor', 'alioth-gen', 'src'), { recursive: true })
  await mkdir(path.join(modelDir, 'skill-adapters'), { recursive: true })
  await mkdir(path.join(modelDir, 'Pre-Proc', 'Alioth', '_schema'), { recursive: true })
  await writeFile(path.join(modelDir, 'backend', 'ddl', '002_isahl_meta_schema.sql'), SCHEMA_DDL)
  await writeFile(path.join(modelDir, 'backend', 'ddl', '003_isahl_meta_seed.sql'), SEEDS_DDL)
  await writeFile(path.join(modelDir, 'skill-adapters', 'alioth-app.yaml'), 'track: app\n')
  await writeFile(path.join(modelDir, 'Pre-Proc', 'Alioth', '_schema', 'app.schema.json'), '{}\n')
  await writeFile(
    path.join(modelDir, 'backend', 'vendor', 'alioth-gen', 'src', 'lib.rs'),
    'pub static ALIOTH_MODEL_VERSION: LazyLock<String> =\n'
    + '    LazyLock::new(|| env::var("MODEL_VERSION").unwrap_or_else(|_| "10.0.0".to_string()));\n',
  )

  ctx = new Context()
  const systemFiber = await ctx.plugin(SystemPrompt)
  disposers.push(() => systemFiber.dispose())
  const toolsFiber = await ctx.plugin(ToolRuntime)
  disposers.push(() => toolsFiber.dispose())
  const envFiber = await ctx.plugin(envAlioth, { modelSource: modelDir, dataRoot })
  disposers.push(() => envFiber.dispose())
  const metaFiber = await ctx.plugin(toolMeta, {})
  disposers.push(() => metaFiber.dispose())
}, 120_000)

afterAll(async () => {
  for (const dispose of disposers.reverse()) {
    await dispose().catch(() => {})
  }
})

function expectSuccess(result: Awaited<ReturnType<typeof callSchemaInfo>>): Record<string, unknown> {
  if (result.isError) {
    throw new Error(`expected alioth_schema_info success, got: ${result.error.message}`)
  }
  return result.value as Record<string, unknown>
}

describe('alioth_schema_info', () => {
  it('lists collections with category/depth/inherits from config jsonb', async () => {
    const value = expectSuccess(await callSchemaInfo({ action: 'entities', query: '库存' }))
    expect(value).toMatchObject({
      action: 'entities',
      entities: [{
        table: 'zc_id_inventory',
        name: '库存',
        type: 'table',
        category: '交易信息',
        depth: 3,
        inherits: ['zc_id_object'],
        description: '库存台账',
      }],
    })
  })

  it('filters collections by exact category', async () => {
    const value = expectSuccess(await callSchemaInfo({ action: 'entities', category: '协作' }))
    expect(value).toMatchObject({
      entities: [{ table: 'zc_id_demand', name: '需求' }],
    })
  })

  it('describes one collection with ordered fields and flags', async () => {
    const value = expectSuccess(await callSchemaInfo({ action: 'entity', collection: 'zc_id_inventory' }))
    expect(value).toMatchObject({
      action: 'entity',
      collection: { table: 'zc_id_inventory', category: '交易信息', inherits: ['zc_id_object'] },
      fields: [
        { name: 'name', title: '名称', category: 'scalar', dataType: 'text', required: true, defaultValue: '' },
        { name: 'owner_qk_user', title: '负责人', category: 'reference', dataType: 'bigint', required: false },
        { name: 'qty', title: '数量', category: 'scalar', dataType: 'decimal', required: false },
      ],
    })
  })

  it('fails loud on unknown collections with a routing hint', async () => {
    const result = await callSchemaInfo({ action: 'entity', collection: 'no_such' })
    if (!result.isError) throw new Error('expected alioth_schema_info failure')
    expect(result.error.message).toContain('unknown collection "no_such"')
  })

  it('searches fields by name across collections', async () => {
    const value = expectSuccess(await callSchemaInfo({ action: 'search-fields', query: 'name' }))
    expect(value).toMatchObject({
      action: 'search-fields',
      fields: [
        { collection: 'zc_ad_dimension', name: 'name' },
        { collection: 'zc_id_inventory', name: 'name' },
      ],
    })
  })

  it('rejects invalid action, bad limit, and missing query', async () => {
    const badAction = await callSchemaInfo({ action: 'drop' })
    if (!badAction.isError) throw new Error('expected alioth_schema_info failure')
    expect(badAction.error.message).toContain('invalid action')

    const badLimit = await callSchemaInfo({ action: 'entities', limit: 0 })
    if (!badLimit.isError) throw new Error('expected alioth_schema_info failure')
    expect(badLimit.error.message).toContain('limit')

    const noQuery = await callSchemaInfo({ action: 'search-fields' })
    if (!noQuery.isError) throw new Error('expected alioth_schema_info failure')
    expect(noQuery.error.message).toContain('requires "query"')
  })

  it('hides dev-seed test entities by default and reports the count', async () => {
    const hidden = expectSuccess(await callSchemaInfo({ action: 'entities' }))
    expect(hidden.filteredTesting).toBe(2)
    const tables = (hidden.entities as Array<{ table: string }>).map(entry => entry.table)
    expect(tables).not.toContain('zc_id_task-testing')
    expect(tables).not.toContain('zc_id_oper-test')
    expect(tables).toContain('zc_id_inventory')
  })

  it('shows test entities when includeTesting is set', async () => {
    const visible = expectSuccess(await callSchemaInfo({ action: 'entities', includeTesting: true }))
    expect(visible.filteredTesting).toBe(0)
    const tables = (visible.entities as Array<{ table: string }>).map(entry => entry.table)
    expect(tables).toContain('zc_id_task-testing')
  })

  it('filters search-fields results from test collections unless asked', async () => {
    const hidden = expectSuccess(await callSchemaInfo({ action: 'search-fields', query: '测试' }))
    expect((hidden.fields as Array<{ collection: string }>)).toEqual([])
    const visible = expectSuccess(await callSchemaInfo({ action: 'search-fields', query: '测试', includeTesting: true }))
    expect(visible.fields).toMatchObject([
      { collection: 'zc_id_task-testing', name: 'name' },
    ])
  })

  it('describes a test entity directly when named explicitly', async () => {
    const value = expectSuccess(await callSchemaInfo({ action: 'entity', collection: 'zc_id_task-testing' }))
    expect(value.collection).toMatchObject({ table: 'zc_id_task-testing' })
  })
})

// ── alioth_entity_write ──────────────────────────────────────────────────

let entityCounter = 0

function callEntityWrite(args: unknown, over: { agent?: unknown } = {}) {
  return ctx.tools.execute({
    signal,
    callId: ToolCallId(`entity-${++entityCounter}`),
    name: 'alioth_entity_write',
    arguments: args,
    ...(over.agent === undefined ? {} : { agent: over.agent }),
  } as never)
}

describe('dsh-alioth alioth_entity_write (bypass)', () => {
  it('registers a new entity with fields and makes it queryable', async () => {
    const result = await callEntityWrite({
      table: 'zc_id_deta-bill-check',
      name: '账单核查',
      inherits: ['zc_id_object'],
      category: '交易信息',
      coordinates: { scene: 'CA', factor: 'GBA', function: '↑_AA' },
      fields: [
        { name: 'notice', category: 'scalar', dataType: 'text', title: '名称', required: true },
        { name: 'biller', category: 'reference', dataType: 'bigint', title: '开单人', targetTable: 'zc_id_subjects', localKey: 'fk_biller' },
      ],
    })
    if (result.isError) throw new Error(`expected alioth_entity_write success: ${result.error.message}`)
    expect(result.value).toMatchObject({ table: 'zc_id_deta-bill-check', name: '账单核查', fields: 2 })

    const value = expectSuccess(await callSchemaInfo({ action: 'entity', collection: 'zc_id_deta-bill-check' }))
    expect(value.collection).toMatchObject({ table: 'zc_id_deta-bill-check', name: '账单核查' })
    expect(value.fields).toMatchObject([
      { name: 'biller', title: '开单人', category: 'reference', dataType: 'bigint', required: false },
      { name: 'notice', title: '名称', category: 'scalar', dataType: 'text', required: true },
    ])
  })

  it('refuses to re-register the same physical table', async () => {
    const result = await callEntityWrite({ table: 'zc_id_deta-bill-check', name: '又一份', inherits: [], fields: [] })
    if (!result.isError) throw new Error('expected alioth_entity_write failure')
    expect(result.error.message).toContain('collection-conflict')
  })

  it('rejects invalid definitions with all issues listed, writing nothing', async () => {
    const result = await callEntityWrite({
      table: 'zc_id_bad-coord',
      name: '坏坐标',
      inherits: ['zc_id_object'],
      coordinates: { scene: 'XX', factor: 'ZZZ', function: '↓_QQ' },
      fields: [],
    })
    if (!result.isError) throw new Error('expected alioth_entity_write failure')
    expect(result.error.message).toContain('coordinate-scene')
    expect(result.error.message).toContain('coordinate-factor')
    expect(result.error.message).toContain('coordinate-function')
    // Nothing persisted: the table is not queryable.
    const missing = await callSchemaInfo({ action: 'entity', collection: 'zc_id_bad-coord' })
    if (!missing.isError) throw new Error('expected alioth_schema_info failure for unwritten entity')
  })

  it('rejects reference fields without a target table', async () => {
    const result = await callEntityWrite({
      table: 'zc_id_scene',
      name: '操作',
      inherits: [],
      fields: [{ name: 'fk_x', category: 'reference', dataType: 'bigint' }],
    })
    if (!result.isError) throw new Error('expected alioth_entity_write failure')
    expect(result.error.message).toContain('requires targetTable')
  })

  it('rejects fields on a table that is not an isahl physical table', async () => {
    const result = await callEntityWrite({ table: 'not_a_physical_table', name: '幻影', inherits: [], fields: [] })
    if (!result.isError) throw new Error('expected alioth_entity_write failure')
    expect(result.error.message).toContain('physical-table')
  })
})

describe('dsh-alioth alioth_entity_write (approvalMode=required)', () => {
  // One embedded-PG cluster per test: two env-alioth instances over the same
  // data root would start a second postgres on the same dataDir.
  let approvalModelDir: string
  const disposers: Array<() => Promise<void>> = []
  const dataRoots: string[] = []

  async function bootWithApproval(answer: 'allowed-once' | 'rejected'): Promise<Context> {
    const dataRoot = await mkdtemp(path.join(tmpdir(), 'dsh-alioth-entity-data-'))
    dataRoots.push(dataRoot)
    const ctx = new Context()
    const system = await ctx.plugin(SystemPrompt)
    disposers.push(() => system.dispose())
    const tools = await ctx.plugin(ToolRuntime)
    disposers.push(() => tools.dispose())
    ctx.provide('approval')
    ctx.set('approval', { request: async () => answer } as never)
    const envFiber = await ctx.plugin(envAlioth, { modelSource: approvalModelDir, dataRoot })
    disposers.push(() => envFiber.dispose())
    const metaFiber = await ctx.plugin(toolMeta, { approvalMode: 'required' })
    disposers.push(() => metaFiber.dispose())
    return ctx
  }

  beforeAll(async () => {
    approvalModelDir = await mkdtemp(path.join(tmpdir(), 'dsh-alioth-entity-model-'))
    await mkdir(path.join(approvalModelDir, 'backend', 'ddl'), { recursive: true })
    await mkdir(path.join(approvalModelDir, 'backend', 'vendor', 'alioth-gen', 'src'), { recursive: true })
    await mkdir(path.join(approvalModelDir, 'skill-adapters'), { recursive: true })
    await mkdir(path.join(approvalModelDir, 'Pre-Proc', 'Alioth', '_schema'), { recursive: true })
    await writeFile(path.join(approvalModelDir, 'backend', 'ddl', '002_isahl_meta_schema.sql'), SCHEMA_DDL)
    await writeFile(path.join(approvalModelDir, 'backend', 'ddl', '003_isahl_meta_seed.sql'), SEEDS_DDL)
    await writeFile(path.join(approvalModelDir, 'skill-adapters', 'a.yaml'), 'x\n')
    await writeFile(path.join(approvalModelDir, 'Pre-Proc', 'Alioth', '_schema', 'a.schema.json'), '{}\n')
    await writeFile(
      path.join(approvalModelDir, 'backend', 'vendor', 'alioth-gen', 'src', 'lib.rs'),
      'pub static ALIOTH_MODEL_VERSION: LazyLock<String> =\n    LazyLock::new(|| env::var("MODEL_VERSION").unwrap_or_else(|_| "10.0.0".to_string()));\n',
    )
  }, 120_000)

  afterAll(async () => {
    for (const dispose of disposers.reverse()) {
      await dispose().catch(() => {})
    }
    await rm(approvalModelDir, { recursive: true, force: true })
    for (const dataRoot of dataRoots) {
      await rm(dataRoot, { recursive: true, force: true })
    }
  })

  it('writes when approval grants allowed-once', async () => {
    const approvalCtx = await bootWithApproval('allowed-once')
    const result = await approvalCtx.tools.execute({
      signal,
      callId: ToolCallId('entity-grant'),
      name: 'alioth_entity_write',
      arguments: { table: 'zc_id_scene', name: '操作', inherits: ['zc_id_object'], fields: [] },
      agent: { id: SessionId('parent-2'), session: Session.create(SessionId('parent-2')) } as never,
    })
    if (result.isError) throw new Error(`expected alioth_entity_write success: ${result.error.message}`)
    expect(result.value).toMatchObject({ table: 'zc_id_scene' })
  })

  it('denies the write when approval rejects', async () => {
    const approvalCtx = await bootWithApproval('rejected')
    const result = await approvalCtx.tools.execute({
      signal,
      callId: ToolCallId('entity-deny'),
      name: 'alioth_entity_write',
      arguments: { table: 'zc_id_stus-employ', name: '雇员状态', inherits: ['zc_id_object'], fields: [] },
      agent: { id: SessionId('parent-2'), session: Session.create(SessionId('parent-2')) } as never,
    })
    if (!result.isError) throw new Error('expected alioth_entity_write failure')
    expect(result.error.message).toContain('denied by approval')
  })
})
