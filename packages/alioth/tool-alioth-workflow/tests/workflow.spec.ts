import { describe, expect, it, beforeAll, afterAll } from 'vitest'
import { mkdir, mkdtemp, readdir, rm, writeFile } from 'node:fs/promises'
import { tmpdir } from 'node:os'
import path from 'node:path'
import { Context } from '@deepseek-ai/cordis'
import SystemPrompt from '@deepseek-ai/dsh-system-prompt'
import ToolRuntime, { defineTool } from '@deepseek-ai/dsh-tools'
import { ToolCallId } from '@deepseek-ai/dsh-llm'
import * as envAlioth from '@dsh-alioth/env-alioth'
import { GATE_PROGRAM_WHITELIST } from '@dsh-alioth/skill-alioth'
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
let contentRoot: string
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
  contentRoot = await mkdtemp(path.join(tmpdir(), 'wf-content-'))
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
  const wf = await ctx.plugin(workflow, { preProcRoot, contentRoot })
  disposers.push(() => wf.dispose())
  // A composed deployment registers the harness file tools; stand in for the
  // one the fixture adapter declares so the step payload's harness surface is
  // computed against a realistic registered set rather than an empty one.
  ctx.tools.register(defineTool({
    name: 'write',
    description: 'fixture stub for the harness write tool',
    parameters: {},
    output: {
      schema: { type: 'object', additionalProperties: true, properties: {} },
      render: () => [],
    },
    execute: async () => ({}),
  }))
}, 120_000)

afterAll(async () => {
  for (const dispose of disposers.reverse()) {
    await dispose().catch(() => {})
  }
  await rm(preProcRoot, { recursive: true, force: true })
  await rm(contentRoot, { recursive: true, force: true })
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
    // The declared adapter vocabulary is translated into the concrete harness
    // tools the deployment registers — the model calls those, not the aliases.
    expect(value.harnessTools).toEqual(['write'])
    expect(value.missingTools).toEqual([])
    expect(value.manualTools).toEqual([])
  }, 120_000)

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

    const badApp = await callTool('alioth_workflow_complete', { namespace: 'Alioth', app: 'demo_app' })
    if (!badApp.isError) throw new Error('expected alioth_workflow_complete failure')
    expect(badApp.error.message).toContain('invalid app code')
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

// ── Gate programs, step inputs and hand-built config defaults ──────────────
// A second deployment: its adapter declares program gates and engine-injected
// step inputs, and its snapshot carries no `_runtime.yaml` — the program
// allowlist has to fall back to the code-truth floor instead of widening.

const COVER_ADAPTER_YAML = `
name: alioth-app-cover
description: "门禁/输入覆盖轨道"
version: "2.0"
tracks:
  - name: 覆盖轨道
    steps:
      - id: "1.1"
        instruction: "读取步骤输入并跑程序门禁"
        tools: [write_file, run_command, visual_verify, quantum_analyzer]
        inputs:
          - "Pre-Proc/{ns}/Apps/{app}/brief.md"
          - "Pre-Proc/{ns}/Apps/{app}/big.txt"
          - "Pre-Proc/{ns}/Apps/{app}/missing.txt"
          - "{module}/notes.md"
          - "/etc/hosts"
        gates:
          - output_glob: "Pre-Proc/{ns}/Apps/{app}/app.json"
          - program: "bun"
            args: ["--version"]
          - program: "target/debug/ontology-mapping"
            args: ["--probe", "{ns}"]
            expected_exit_code: 3
            timeout_sec: 5
      - id: "1.2"
        instruction: "收尾"
        gates:
          - output_glob: "Pre-Proc/{ns}/Apps/{app}/extensions/constraints.yaml"
`

const MODEL_LIB_RS = 'pub static ALIOTH_MODEL_VERSION: LazyLock<String> =\n'
  + '    LazyLock::new(|| env::var("MODEL_VERSION").unwrap_or_else(|_| "10.0.0".to_string()));\n'

describe('tool-alioth-workflow: gates, step inputs and config defaults', () => {
  let coverCtx: Context
  const coverDisposers: Array<() => Promise<void>> = []
  let coverPreProc: string
  let coverContent: string
  let coverCounter = 0

  function callCover(name: string, args: unknown) {
    return coverCtx.tools.execute({
      signal,
      callId: ToolCallId(`cover-${++coverCounter}`),
      name,
      arguments: args,
    })
  }

  beforeAll(async () => {
    const modelDir = await mkdtemp(path.join(tmpdir(), 'wf-cover-model-'))
    const dataRoot = await mkdtemp(path.join(tmpdir(), 'wf-cover-data-'))
    coverPreProc = await mkdtemp(path.join(tmpdir(), 'wf-cover-preproc-'))
    coverContent = await mkdtemp(path.join(tmpdir(), 'wf-cover-content-'))
    await mkdir(path.join(modelDir, 'backend', 'ddl'), { recursive: true })
    await mkdir(path.join(modelDir, 'backend', 'vendor', 'alioth-gen', 'src'), { recursive: true })
    await mkdir(path.join(modelDir, 'skill-adapters'), { recursive: true })
    await mkdir(path.join(modelDir, 'Pre-Proc', 'Alioth', '_schema'), { recursive: true })
    await writeFile(path.join(modelDir, 'backend', 'ddl', '002_isahl_meta_schema.sql'), SCHEMA_DDL)
    await writeFile(path.join(modelDir, 'skill-adapters', 'cover.yaml'), COVER_ADAPTER_YAML)
    await writeFile(path.join(modelDir, 'Pre-Proc', 'Alioth', '_schema', 'a.schema.json'), '{}\n')
    await writeFile(path.join(modelDir, 'backend', 'vendor', 'alioth-gen', 'src', 'lib.rs'), MODEL_LIB_RS)

    coverCtx = new Context()
    const system = await coverCtx.plugin(SystemPrompt)
    coverDisposers.push(() => system.dispose())
    const tools = await coverCtx.plugin(ToolRuntime)
    coverDisposers.push(() => tools.dispose())
    const env = await coverCtx.plugin(envAlioth, { modelSource: modelDir, dataRoot })
    coverDisposers.push(() => env.dispose())
    const wf = await coverCtx.plugin(workflow, { preProcRoot: coverPreProc, contentRoot: coverContent, adapter: 'cover.yaml' })
    coverDisposers.push(() => wf.dispose())
    coverCtx.tools.register(defineTool({
      name: 'write',
      description: 'fixture stub for the harness write tool',
      parameters: {},
      output: {
        schema: { type: 'object', additionalProperties: true, properties: {} },
        render: () => [],
      },
      execute: async () => ({}),
    }))
  }, 120_000)

  afterAll(async () => {
    for (const dispose of coverDisposers.reverse()) {
      await dispose().catch(() => {})
    }
    await rm(coverPreProc, { recursive: true, force: true })
    await rm(coverContent, { recursive: true, force: true })
  })

  it('formats program gates, resolves the tool surface and injects the step inputs', async () => {
    const appDir = path.join(coverPreProc, 'Demo', 'Apps', 'cover-app')
    await mkdir(appDir, { recursive: true })
    await writeFile(path.join(appDir, 'brief.md'), '任务简述\n')
    await writeFile(path.join(appDir, 'big.txt'), 'x'.repeat(4200))

    const step = expectOk(await callCover('alioth_workflow_step', { namespace: 'Demo', app: 'cover-app' }))
    expect(step).toMatchObject({
      finished: false,
      track: '覆盖轨道',
      stepId: '1.1',
      tools: ['write_file', 'run_command', 'visual_verify', 'quantum_analyzer'],
      // Only the declared tools the deployment actually registers, expressed
      // through the harness vocabulary.
      harnessTools: ['write'],
      manualTools: [{ adapterTool: 'visual_verify' }],
      // Declared tools nothing satisfies — mapped-but-unregistered and unmapped.
      missingTools: ['run_command', 'quantum_analyzer'],
    })
    expect(step.gates).toEqual([
      'output_glob: Pre-Proc/{ns}/Apps/{app}/app.json',
      // expected_exit is printed only when the gate wants a non-zero code.
      'program: bun --version timeout=120s',
      'program: target/debug/ontology-mapping --probe {ns} expected_exit=3 timeout=5s',
    ])
    expect(step.inputs).toEqual([
      { path: 'Pre-Proc/Demo/Apps/cover-app/brief.md', content: '任务简述\n' },
      // Over the injection cap: reported truncated, not silently dropped.
      { path: 'Pre-Proc/Demo/Apps/cover-app/big.txt', content: `${'x'.repeat(4000)}\n…(truncated)` },
      // Unreadable: the path is still reported for the model to explore.
      { path: 'Pre-Proc/Demo/Apps/cover-app/missing.txt' },
      // An unknown template variable stays literal, and the resolved path
      // outside the Pre-Proc tree is reported without content.
      { path: '{module}/notes.md' },
      { path: '/etc/hosts' },
    ])
  })

  it('reports a gate program that cannot run as an environment failure and stays on the step', async () => {
    const appDir = path.join(coverPreProc, 'Demo', 'Apps', 'cover-app')
    await writeFile(path.join(appDir, 'app.json'), '{}\n')

    const result = await callCover('alioth_workflow_complete', { namespace: 'Demo', app: 'cover-app' })
    if (!result.isError) throw new Error('expected alioth_workflow_complete failure')
    expect(result.error.message).toContain('gates failed for step 1.1')
    // A missing gate binary is not model-fixable: classified as environment.
    expect(result.error.message).toContain('[path-missing/environment]')

    const step = expectOk(await callCover('alioth_workflow_step', { namespace: 'Demo', app: 'cover-app' }))
    expect(step.stepId).toBe('1.1')
    expect(step.finished).toBe(false)
  })

  it('falls back to the code-truth program allowlist and the default adapter', async () => {
    // A deployment that hand-builds its context passes a partial config: the
    // adapter, the content root and the program allowlist all take their
    // documented defaults (the snapshot here ships no `_runtime.yaml`).
    const root = await mkdtemp(path.join(tmpdir(), 'wf-defaults-'))
    const preProc = path.join(root, 'Pre-Proc')
    await mkdir(preProc, { recursive: true })
    const modelDir = await mkdtemp(path.join(tmpdir(), 'wf-defaults-model-'))
    const dataRoot = await mkdtemp(path.join(tmpdir(), 'wf-defaults-data-'))
    await mkdir(path.join(modelDir, 'backend', 'ddl'), { recursive: true })
    await mkdir(path.join(modelDir, 'backend', 'vendor', 'alioth-gen', 'src'), { recursive: true })
    await mkdir(path.join(modelDir, 'skill-adapters'), { recursive: true })
    await mkdir(path.join(modelDir, 'Pre-Proc', 'Alioth', '_schema'), { recursive: true })
    await writeFile(path.join(modelDir, 'backend', 'ddl', '002_isahl_meta_schema.sql'), SCHEMA_DDL)
    await writeFile(path.join(modelDir, 'skill-adapters', 'alioth-app.yaml'), ADAPTER_YAML)
    await writeFile(path.join(modelDir, 'Pre-Proc', 'Alioth', '_schema', 'a.schema.json'), '{}\n')
    await writeFile(path.join(modelDir, 'backend', 'vendor', 'alioth-gen', 'src', 'lib.rs'), MODEL_LIB_RS)

    const defaultsCtx = new Context()
    const system = await defaultsCtx.plugin(SystemPrompt)
    const tools = await defaultsCtx.plugin(ToolRuntime)
    const env = await defaultsCtx.plugin(envAlioth, { modelSource: modelDir, dataRoot })
    // No Loader: no schema defaults for adapter/contentRoot.
    workflow.apply(defaultsCtx, { preProcRoot: preProc })
    defaultsCtx.tools.register(defineTool({
      name: 'write',
      description: 'fixture stub for the harness write tool',
      parameters: {},
      output: {
        schema: { type: 'object', additionalProperties: true, properties: {} },
        render: () => [],
      },
      execute: async () => ({}),
    }))

    try {
      const info = expectOk(await defaultsCtx.tools.execute({
        signal,
        callId: ToolCallId('defaults-info'),
        name: 'alioth_workflow_info',
        arguments: {},
      }))
      expect(info.adapter).toBe('alioth-app.yaml')
      expect(info.runtime).toEqual({ allowedPrograms: [...GATE_PROGRAM_WHITELIST] })

      const appDir = path.join(preProc, 'Alioth', 'Apps', 'defaults-app')
      await mkdir(appDir, { recursive: true })
      await writeFile(path.join(appDir, 'app.json'), '{}\n')
      const completed = expectOk(await defaultsCtx.tools.execute({
        signal,
        callId: ToolCallId('defaults-complete'),
        name: 'alioth_workflow_complete',
        arguments: { namespace: 'Alioth', app: 'defaults-app' },
      }))
      expect(completed).toMatchObject({ finished: false, completedStep: '1.1', nextStep: '2.1' })

      // The content root defaulted to the parent of preProcRoot: provisioning
      // materialized the vendored repo-root layout there.
      const provisioned = await readdir(root)
      expect(provisioned).toContain('Pre-Proc')
      expect(provisioned).toContain('scripts')
      expect(provisioned).toContain('.agents')
    } finally {
      await tools.dispose().catch(() => {})
      await env.dispose().catch(() => {})
      await system.dispose().catch(() => {})
    }
  }, 120_000)

  it('returns a finished run instead of advancing it again', async () => {
    // The first-track/first-step track of the main fixture: artifact gates only.
    const appDir = path.join(preProcRoot, 'Alioth', 'Apps', 'finished-app')
    await mkdir(path.join(appDir, 'extensions'), { recursive: true })
    await writeFile(path.join(appDir, 'app.json'), '{}\n')
    await writeFile(path.join(appDir, 'extensions', 'constraints.yaml'), '{}\n')

    const first = expectOk(await callTool('alioth_workflow_complete', { namespace: 'Alioth', app: 'finished-app' }))
    expect(first).toMatchObject({ finished: false, completedStep: '1.1', nextStep: '2.1' })
    const second = expectOk(await callTool('alioth_workflow_complete', { namespace: 'Alioth', app: 'finished-app' }))
    expect(second).toMatchObject({ finished: true, completedStep: '2.1', nextStep: '' })

    // Completing a run with no current step is a no-op, not an advance.
    const third = expectOk(await callTool('alioth_workflow_complete', { namespace: 'Alioth', app: 'finished-app' }))
    expect(third).toEqual({ finished: true, completedStep: '', gateResults: [], nextStep: '' })
  })
})
