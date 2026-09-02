import { describe, expect, it, beforeAll, afterAll } from 'vitest'
import { mkdir, mkdtemp, rm, writeFile } from 'node:fs/promises'
import { tmpdir } from 'node:os'
import path from 'node:path'
import { Context } from '@deepseek-ai/cordis'
import SystemPrompt from '@deepseek-ai/dsh-system-prompt'
import ToolRuntime from '@deepseek-ai/dsh-tools'
import { ToolCallId } from '@deepseek-ai/dsh-llm'
import * as envAlioth from '@dsh-alioth/env-alioth'
import * as workflow from '../src/index.ts'

const signal = new AbortController().signal

/** Two-step track: step 1.1 gates on app.json; step 2.1 gates on extensions/constraints.yaml. */
const ADAPTER_YAML = `
name: alioth-app
description: "App 级原型集成"
version: "2.0"
tracks:
  - name: App 构建
    steps:
      - id: "1.1"
        instruction: "preflight — 确认 App 上下文，生成 app.json"
        tools: [write_file]
        schema: {type: object, required: [ns, app]}
        gates:
          - output_glob: "Pre-Proc/{ns}/Apps/{app}/app.json"
      - id: "2.1"
        instruction: "扩展 — 生成 extensions 骨架"
        tools: [write_file]
        gates:
          - output_glob: "Pre-Proc/{ns}/Apps/{app}/extensions/constraints.yaml"
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
let counter = 0

function callTool(name: string, args: unknown) {
  return ctx.tools.execute({
    signal,
    callId: ToolCallId(`wf-${++counter}`),
    name,
    arguments: args,
  })
}

beforeAll(async () => {
  const modelDir = await mkdtemp(path.join(tmpdir(), 'wf-model-'))
  const dataRoot = await mkdtemp(path.join(tmpdir(), 'wf-data-'))
  preProcRoot = await mkdtemp(path.join(tmpdir(), 'wf-preproc-'))
  await mkdir(path.join(modelDir, 'backend', 'ddl'), { recursive: true })
  await mkdir(path.join(modelDir, 'backend', 'vendor', 'alioth-gen', 'src'), { recursive: true })
  await mkdir(path.join(modelDir, 'skill-adapters'), { recursive: true })
  await mkdir(path.join(modelDir, 'Pre-Proc', 'Alioth', '_schema'), { recursive: true })
  await writeFile(path.join(modelDir, 'backend', 'ddl', '002_isahl_meta_schema.sql'), SCHEMA_DDL)
  await writeFile(path.join(modelDir, 'skill-adapters', 'alioth-app.yaml'), ADAPTER_YAML)
  await writeFile(path.join(modelDir, 'skill-adapters', '_runtime.yaml'),
    'allowed_programs:\n  - bun\n  - target/debug/ontology-mapping\n')
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
  const wf = await ctx.plugin(workflow, { preProcRoot })
  disposers.push(() => wf.dispose())
}, 120_000)

afterAll(async () => {
  for (const dispose of disposers.reverse()) {
    await dispose().catch(() => {})
  }
  await rm(preProcRoot, { recursive: true, force: true })
})

function expectOk(result: Awaited<ReturnType<typeof callTool>>): Record<string, unknown> {
  if (result.isError) {
    throw new Error(`expected success, got: ${result.error.message}`)
  }
  return result.value as Record<string, unknown>
}

describe('alioth workflow bridge', () => {
  it('shows the first step of a fresh run', async () => {
    const value = expectOk(await callTool('alioth_workflow_step', { namespace: 'Alioth', app: 'demo-app' }))
    expect(value).toMatchObject({
      finished: false,
      track: 'App 构建',
      stepId: '1.1',
      tools: ['write_file'],
    })
    expect(String(value.instruction)).toContain('preflight')
  })

  it('fails the gate when the artifact is missing and does not advance', async () => {
    const result = await callTool('alioth_workflow_complete', { namespace: 'Alioth', app: 'demo-app' })
    if (!result.isError) throw new Error('expected alioth_workflow_complete failure')
    expect(result.error.message).toContain('gates failed')
    expect(result.error.message).toContain('app.json')

    // Still on step 1.1 after the failure.
    const step = expectOk(await callTool('alioth_workflow_step', { namespace: 'Alioth', app: 'demo-app' }))
    expect(step.stepId).toBe('1.1')
  })

  it('advances after the artifact lands and runs the second step to completion', async () => {
    const appDir = path.join(preProcRoot, 'Alioth', 'Apps', 'demo-app')
    await mkdir(appDir, { recursive: true })
    await writeFile(path.join(appDir, 'app.json'), '{}\n')

    const first = expectOk(await callTool('alioth_workflow_complete', { namespace: 'Alioth', app: 'demo-app' }))
    expect(first).toMatchObject({ finished: false, completedStep: '1.1', nextStep: '2.1' })

    const second = expectOk(await callTool('alioth_workflow_step', { namespace: 'Alioth', app: 'demo-app' }))
    expect(second.stepId).toBe('2.1')

    // Step 2.1's gate needs the extensions skeleton.
    const blocked = await callTool('alioth_workflow_complete', { namespace: 'Alioth', app: 'demo-app' })
    if (!blocked.isError) throw new Error('expected step 2.1 gate failure')
    await mkdir(path.join(appDir, 'extensions'), { recursive: true })
    await writeFile(path.join(appDir, 'extensions', 'constraints.yaml'), '{}\n')

    const done = expectOk(await callTool('alioth_workflow_complete', { namespace: 'Alioth', app: 'demo-app' }))
    expect(done).toMatchObject({ finished: true, completedStep: '2.1', nextStep: '' })

    const finalStep = expectOk(await callTool('alioth_workflow_step', { namespace: 'Alioth', app: 'demo-app' }))
    expect(finalStep.finished).toBe(true)
  })

  it('rejects malformed namespace/app values', async () => {
    const result = await callTool('alioth_workflow_step', { namespace: 'alioth', app: 'demo-app' })
    if (!result.isError) throw new Error('expected alioth_workflow_step failure')
    expect(result.error.message).toContain('invalid namespace')
  })

  it('introspects the full adapter definition without touching files', async () => {
    const value = expectOk(await callTool('alioth_workflow_info', {}))
    expect(value.adapter).toBe('alioth-app.yaml')
    expect(value).toMatchObject({
      tracks: [
        {
          id: 'App 构建',
          name: 'App 构建',
          steps: [
            { id: '1.1', tools: ['write_file'], gates: ['output_glob: Pre-Proc/{ns}/Apps/{app}/app.json'] },
            { id: '2.1', gates: ['output_glob: Pre-Proc/{ns}/Apps/{app}/extensions/constraints.yaml'] },
          ],
        },
      ],
      runtime: { allowedPrograms: ['bun', 'target/debug/ontology-mapping'] },
    })
    const tracks = (value as { tracks: Array<{ steps: Array<{ instruction: string }> }> }).tracks
    expect(String(tracks[0]!.steps[0]!.instruction)).toContain('preflight')
  })
})
