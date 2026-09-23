/**
 * 步骤载荷与修复契约：`alioth_workflow_step` 必须透出 phase / planWriteGlobs / 门禁完整
 * 形态（含内容谓词），门禁失败必须回灌带 rule 前缀的结构化修复契约。用一份专用适配器
 * （plan 步 + 内容谓词门）驱动，避免污染既有夹具的步骤序列。
 */
import { describe, expect, it, beforeAll, afterAll } from 'vitest'
import { mkdir, mkdtemp, rm, writeFile } from 'node:fs/promises'
import { tmpdir } from 'node:os'
import path from 'node:path'
import { Context } from '@deepseek-ai/cordis'
import SystemPrompt from '@deepseek-ai/dsh-system-prompt'
import ToolRuntime, { defineTool } from '@deepseek-ai/dsh-tools'
import { ToolCallId } from '@deepseek-ai/dsh-llm'
import * as envAlioth from '@dsh-alioth/env-alioth'
import { createTestDatabase, type TestDatabase } from '../../env-alioth/tests/test-db.ts'
import * as workflow from '../src/index.ts'

const signal = new AbortController().signal

/** plan 步：只写本步 output_glob 的方案产物；内容谓词要求 /draft/ready === true。 */
const ADAPTER_YAML = `
name: plan-predicate
version: "2.0"
tracks:
  - name: 方案轨道
    steps:
      - id: "1.1"
        phase: plan
        instruction: "只产出方案草案"
        tools: [write_file]
        gates:
          - output_glob: "Pre-Proc/{ns}/Apps/{app}/plans/draft.json"
            require_json_pointer: "/draft/ready"
            require_json_equals: true
      - id: "1.2"
        phase: apply
        instruction: "落地"
        gates:
          - output_glob: "Pre-Proc/{ns}/Apps/{app}/plans/draft.json"
`

const SCHEMA_DDL = `
CREATE TYPE isahl_meta.collection_type AS ENUM ('table', 'view');
CREATE TABLE isahl_meta.meta_collections (
    table_name text NOT NULL,
    name text NOT NULL,
    PRIMARY KEY (table_name)
);
`

let ctx: Context
const disposers: Array<() => Promise<void>> = []
let preProcRoot: string
let contentRoot: string
let counter = 0

function callTool(name: string, args: unknown) {
  return ctx.tools.execute({
    signal,
    callId: ToolCallId(`pp-${++counter}`),
    name,
    arguments: args,
  })
}

function expectOk(result: Awaited<ReturnType<typeof callTool>>): Record<string, unknown> {
  if (result.isError) throw new Error(`expected success, got: ${result.error.message}`)
  return result.value as Record<string, unknown>
}

let testDb: TestDatabase

beforeAll(async () => {
  testDb = await createTestDatabase('payload')
  const modelDir = await mkdtemp(path.join(tmpdir(), 'pp-model-'))
  const dataRoot = await mkdtemp(path.join(tmpdir(), 'pp-data-'))
  preProcRoot = await mkdtemp(path.join(tmpdir(), 'pp-preproc-'))
  contentRoot = await mkdtemp(path.join(tmpdir(), 'pp-content-'))
  await mkdir(path.join(modelDir, 'backend', 'ddl'), { recursive: true })
  await mkdir(path.join(modelDir, 'backend', 'vendor', 'alioth-gen', 'src'), { recursive: true })
  await mkdir(path.join(modelDir, 'skill-adapters'), { recursive: true })
  await mkdir(path.join(modelDir, 'Pre-Proc', 'Alioth', '_schema'), { recursive: true })
  await writeFile(path.join(modelDir, 'backend', 'ddl', '002_isahl_meta_schema.sql'), SCHEMA_DDL)
  await writeFile(path.join(modelDir, 'skill-adapters', 'plan.yaml'), ADAPTER_YAML)
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
  const env = await ctx.plugin(envAlioth, { modelSource: modelDir, dataRoot, databaseUrl: testDb.url })
  disposers.push(() => env.dispose())
  const wf = await ctx.plugin(workflow, { preProcRoot, contentRoot, adapter: 'plan.yaml' })
  disposers.push(() => wf.dispose())
  ctx.tools.register(defineTool({
    name: 'write',
    description: 'fixture stub for the harness write tool',
    parameters: {},
    output: { schema: { type: 'object', additionalProperties: true, properties: {} }, render: () => [] },
    execute: async () => ({}),
  }))
}, 120_000)

afterAll(async () => {
  await testDb.dispose()
  for (const dispose of disposers.reverse()) {
    await dispose().catch(() => {})
  }
  await rm(preProcRoot, { recursive: true, force: true })
  await rm(contentRoot, { recursive: true, force: true })
})

describe('step payload: phase, write surface and the full gate form', () => {
  it('exposes the plan phase, the resolved write globs and the content predicate', async () => {
    const step = expectOk(await callTool('alioth_workflow_step', { namespace: 'Alioth', app: 'plan-app' }))
    expect(step).toMatchObject({
      finished: false,
      stepId: '1.1',
      phase: 'plan',
      planWriteGlobs: ['Pre-Proc/Alioth/Apps/plan-app/plans/draft.json'],
      gates: [{
        kind: 'output-glob',
        outputGlob: 'Pre-Proc/{ns}/Apps/{app}/plans/draft.json',
        requireJsonPointer: '/draft/ready',
        // The expected value is any JSON value; presented as JSON text.
        requireJsonEquals: 'true',
      }],
    })
  }, 120_000)

  it('fails a missing plan artifact with the glob rule as a structured contract', async () => {
    const result = await callTool('alioth_workflow_complete', { namespace: 'Alioth', app: 'plan-app' })
    if (!result.isError) throw new Error('expected alioth_workflow_complete failure')
    expect(result.error.message).toContain('gates failed for step 1.1')
    expect(result.error.message).toContain('[rule:gate-output-glob-miss] class=fixable')
    expect(result.error.message).toContain('证据：')
  }, 120_000)

  it('fails a content predicate that does not hold, naming the predicate rule', async () => {
    const appDir = path.join(preProcRoot, 'Alioth', 'Apps', 'plan-app', 'plans')
    await mkdir(appDir, { recursive: true })
    // The artifact exists but the predicate is not satisfied (指针缺失/值不等).
    await writeFile(path.join(appDir, 'draft.json'), '{ "draft": { "ready": false } }\n')

    const result = await callTool('alioth_workflow_complete', { namespace: 'Alioth', app: 'plan-app' })
    if (!result.isError) throw new Error('expected the predicate gate to fail')
    expect(result.error.message).toContain('[rule:gate-json-predicate-fail] class=fixable')
    expect(result.error.message).toContain('/draft/ready')
  }, 120_000)

  it('advances the plan step once the predicate holds and moves to the apply step', async () => {
    const appDir = path.join(preProcRoot, 'Alioth', 'Apps', 'plan-app', 'plans')
    await writeFile(path.join(appDir, 'draft.json'), '{ "draft": { "ready": true } }\n')

    const done = expectOk(await callTool('alioth_workflow_complete', { namespace: 'Alioth', app: 'plan-app' }))
    expect(done).toMatchObject({ completedStep: '1.1', nextStep: '1.2' })

    const next = expectOk(await callTool('alioth_workflow_step', { namespace: 'Alioth', app: 'plan-app' }))
    expect(next).toMatchObject({ stepId: '1.2', phase: 'apply' })
  }, 120_000)
})
