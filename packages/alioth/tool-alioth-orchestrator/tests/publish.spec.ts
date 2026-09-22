/**
 * publish 前置（五项，全部 fail-closed）+ publish 影子 + 受控并行原语。
 *
 * 前置证据全部由 verify-alioth 的生产者产出（扩展验证 / quality 评估报告 / 版本快照 /
 * 独立结束审计），因此夹具同时是「生产链」的行为测试。
 */
import { describe, expect, it, beforeAll, afterAll } from 'vitest'
import { mkdir, mkdtemp, readFile, rm, writeFile } from 'node:fs/promises'
import { tmpdir } from 'node:os'
import path from 'node:path'
import { Context } from '@deepseek-ai/cordis'
import { Config as OrchestratorConfig } from '../src/index.ts'
import SystemPrompt from '@deepseek-ai/dsh-system-prompt'
import ToolRuntime, { type ToolExecutionToken, type ToolRunContext } from '@deepseek-ai/dsh-tools'
import { ToolCallId } from '@deepseek-ai/dsh-llm'
import * as envAlioth from '@dsh-alioth/env-alioth'
import * as toolAlioth from '@dsh-alioth/tool-alioth'
import * as toolMeta from '@dsh-alioth/tool-alioth-meta'
import {
  appendClosureVerdict,
  artifactFingerprint,
  buildEvalReport,
  createDeferredStore,
  listSnapshots,
  snapshotArtifacts,
  verifyExtensions,
  writeEvalReport,
  writeExtensionVerify,
} from '@dsh-alioth/verify-alioth'
import { buildPlan, buildPrimitives } from '../src/primitives.ts'
import { PIPELINE_SHARED_SURFACES, pathTouchesSurface, runControlledParallel } from '../src/parallel.ts'

const signal = new AbortController().signal

/** The primitives read only `signal`/`agent`; the registry-owned members cannot
 * exist outside the tool runtime, so they are stubbed (the spec's one cast). */
function stageExec(callId: string): ToolRunContext {
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
let dataRoot: string
let counter = 0

const shared = { namespace: 'Demo', name: '发布应用', modules: [{ id: 'm1', name: 'M1' }] }

function delay(ms: number): Promise<void> {
  const { promise, resolve } = Promise.withResolvers<void>()
  setTimeout(resolve, ms)
  return promise
}

function exec(): ToolRunContext {
  return stageExec(`publish-${++counter}`)
}

/** Create the app container and produce every publish-precondition evidence file. */
async function publishable(code: string) {
  const args = { ...shared, code }
  const primitives = buildPrimitives(ctx, exec(), args, undefined, preProcRoot)
  const created = await primitives.appCreation('publish fixture')
  expect(created.artifacts).toContain('app.json')
  const appDir = path.join(preProcRoot, 'Demo', 'Apps', code)

  const extension = await verifyExtensions({ app: code, namespace: 'Demo', appDir, allowedForms: ['constraints', 'rules', 'statemachines', 'workflows', 'profiles'] })
  await writeExtensionVerify(appDir, extension)
  const quality = await buildEvalReport({ app: code, namespace: 'Demo', appDir })
  await writeEvalReport(appDir, quality)
  await snapshotArtifacts(appDir)
  const fingerprint = await artifactFingerprint(appDir)
  await appendClosureVerdict(appDir, {
    app: code,
    namespace: 'Demo',
    fingerprint,
    verdict: 'approved',
    findings: [{ dimension: 'publish', verdict: 'pass', detail: 'fixture' }],
    evidence: [],
  })
  return { args, appDir, primitives, fingerprint }
}

/** The prototype the quality dimension reads (model-authored in the real flow). */
async function seedPrototype(code: string): Promise<void> {
  const dir = path.join(preProcRoot, 'Demo', 'Prototypes', 'Apps', code)
  await mkdir(dir, { recursive: true })
  await writeFile(path.join(dir, 'prototype.html'), '<!doctype html>\n<html><body></body></html>\n')
}

function checkOf(result: Awaited<ReturnType<ReturnType<typeof buildPrimitives>['publishing']>>, name: string) {
  return (result.result.runtimeValidation?.checks ?? []).find(check => check.name === name)
}

beforeAll(async () => {
  const modelDir = await mkdtemp(path.join(tmpdir(), 'pub-model-'))
  dataRoot = await mkdtemp(path.join(tmpdir(), 'pub-data-'))
  preProcRoot = await mkdtemp(path.join(tmpdir(), 'pub-preproc-'))
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
}, 120_000)

afterAll(async () => {
  for (const dispose of disposers.reverse()) {
    await dispose().catch(() => {})
  }
  await rm(preProcRoot, { recursive: true, force: true })
  await rm(dataRoot, { recursive: true, force: true })
})

describe('orchestrator mount contract', () => {
  it('requires preProcRoot: a pipeline reading a different tree than its tools cannot be configured', () => {
    // The pipeline drives the sibling tool plugins by name and resolves artifact
    // paths itself; a defaulted root let a mount that configured the tools but not
    // the pipeline run until module-creation failed, reporting a path the operator
    // never configured. The schema now refuses that configuration outright.
    expect(() => OrchestratorConfig({ preProcRoot: '/tmp/pp' })).not.toThrow()
    expect(() => OrchestratorConfig({} as never)).toThrow(/preProcRoot/)
  })
})

describe('publish preconditions (fail-closed, each on its own)', () => {
  it('passes with the full evidence chain and records the publish shadow trace', async () => {
    await seedPrototype('pub-ok')
    const { args, appDir, primitives } = await publishable('pub-ok')
    const published = await primitives.publishing(buildPlan(args), 1)
    expect(published.result.runtimeValidation).toMatchObject({ valid: true })
    const checks = published.result.runtimeValidation?.checks ?? []
    for (const name of [
      'inspect-readback',
      'publish-extension-verify',
      'publish-no-open-degraded-gates',
      'publish-quality',
      'publish-artifact-snapshot',
      'publish-closure-verdict',
      'workflow-gate',
    ]) {
      expect(checks.find(check => check.name === name)).toMatchObject({ ok: true })
    }

    // 影子只留痕：五谓词齐全、不与判定耦合。
    const trace = await readFile(path.join(appDir, 'AppAgentTraces', 'publish-shadow.jsonl'), 'utf8')
    const record = JSON.parse(trace.trim().split('\n').at(-1) ?? '{}') as { willAutoApprove: boolean; predicates: Record<string, boolean> }
    expect(Object.keys(record.predicates).sort()).toEqual([
      'artifacts_complete',
      'closure_audit_approved',
      'extension_verify_passed',
      'no_open_deferred',
      'quality_passed',
    ])
    expect(record.willAutoApprove).toBe(true)
  }, 120_000)

  it('fails closed when the canonical extension report is missing', async () => {
    await seedPrototype('pub-noext')
    const { args, appDir, primitives } = await publishable('pub-noext')
    await rm(path.join(appDir, 'extension-verify.json'))
    const published = await primitives.publishing(buildPlan(args), 1)
    expect(published.result.runtimeValidation).toMatchObject({ valid: false })
    expect(checkOf(published, 'publish-extension-verify')?.detail).toContain('[rule:publish-extension-verify-missing]')
  }, 120_000)

  it('fails closed on a degraded extension report even though the file exists', async () => {
    await seedPrototype('pub-degraded')
    const { args, appDir, primitives } = await publishable('pub-degraded')
    await writeFile(
      path.join(appDir, 'extension-verify.json'),
      `${JSON.stringify({ schema_version: '1.0', app: 'pub-degraded', namespace: 'Demo', status: 'degraded', covered: 0, uncovered: 1, declarations: [], note: 'fixture', ts: '2026-01-01T00:00:00.000Z', artifact_fingerprint: 'sha256:stale' })}\n`,
    )
    const published = await primitives.publishing(buildPlan(args), 1)
    expect(checkOf(published, 'publish-extension-verify')?.detail).toContain('[rule:publish-extension-verify-degraded]')
  }, 120_000)

  it('fails closed when the extension report is not bound to the CURRENT artifact', async () => {
    await seedPrototype('pub-stale')
    const { args, appDir, primitives } = await publishable('pub-stale')
    await writeFile(
      path.join(appDir, 'extension-verify.json'),
      `${JSON.stringify({ schema_version: '1.0', app: 'pub-stale', namespace: 'Demo', status: 'passed', covered: 1, uncovered: 0, declarations: [], note: 'fixture', ts: '2026-01-01T00:00:00.000Z', artifact_fingerprint: 'sha256:stale' })}\n`,
    )
    const published = await primitives.publishing(buildPlan(args), 1)
    const detail = checkOf(published, 'publish-extension-verify')?.detail ?? ''
    expect(detail).toContain('[rule:publish-extension-verify-unbound]')
    // 产物指纹是判据的一部分，证据里两枚指纹都在。
    expect(detail).toContain('reportFingerprint=sha256:stale')
    expect(detail).toContain('currentFingerprint=sha256:')
  }, 120_000)

  it('blocks on an unresolved degraded gate even with a canonical passed report', async () => {
    await seedPrototype('pub-gate')
    const { args, appDir, primitives } = await publishable('pub-gate')
    const store = createDeferredStore(dataRoot)
    await store.register({
      id: 'gate-visual-verify',
      sessionId: 'session-a',
      app: 'pub-gate',
      namespace: 'Demo',
      reason: 'visual_verify 未执行（harness 无视觉验证工具面）',
      adjudication: '由人工目视确认后解除',
      trigger: { kind: 'artifact-exists', path: path.join(appDir, 'visual-verify', 'report.json') },
      successors: ['human-reviewer'],
      createdTs: '2026-01-01T00:00:00.000Z',
    })
    const published = await primitives.publishing(buildPlan(args), 1)
    expect(published.result.runtimeValidation).toMatchObject({ valid: false })
    const detail = checkOf(published, 'publish-no-open-degraded-gates')?.detail ?? ''
    expect(detail).toContain('[rule:publish-degraded-gate-unresolved]')
    // 文案说明哪条声明未执行、需要谁确认。
    expect(detail).toContain('visual_verify 未执行')
    expect(detail).toContain('human-reviewer')

    // 触发条件成立 → sweep 解除 → 不再阻断。
    await mkdir(path.join(appDir, 'visual-verify'), { recursive: true })
    await writeFile(path.join(appDir, 'visual-verify', 'report.json'), '{}\n')
    const after = await primitives.publishing(buildPlan(args), 2)
    expect(checkOf(after, 'publish-no-open-degraded-gates')).toMatchObject({ ok: true })
  }, 120_000)

  it('fails closed when the quality report is missing or not passing', async () => {
    await seedPrototype('pub-quality')
    const { args, appDir, primitives } = await publishable('pub-quality')
    await rm(path.join(appDir, 'eval-report.json'))
    const missing = await primitives.publishing(buildPlan(args), 1)
    expect(checkOf(missing, 'publish-quality')?.detail).toContain('[rule:publish-quality-report-missing]')

    await writeFile(path.join(appDir, 'eval-report.json'), `${JSON.stringify({ passed: false })}\n`)
    const failing = await primitives.publishing(buildPlan(args), 2)
    expect(checkOf(failing, 'publish-quality')?.detail).toContain('[rule:publish-quality-not-passed]')
  }, 120_000)

  it('fails closed when the artifact snapshot was never written', async () => {
    await seedPrototype('pub-snap')
    const { args, appDir, primitives } = await publishable('pub-snap')
    await rm(path.join(appDir, 'versions'), { recursive: true, force: true })
    expect(await listSnapshots(appDir)).toEqual([])
    const published = await primitives.publishing(buildPlan(args), 1)
    expect(checkOf(published, 'publish-artifact-snapshot')?.detail).toContain('[rule:publish-snapshot-missing]')
  }, 120_000)

  it('fails closed when the closure verdict is missing or not approved', async () => {
    await seedPrototype('pub-closure')
    const { args, appDir, primitives, fingerprint } = await publishable('pub-closure')
    await rm(path.join(appDir, 'AppAgentTraces'), { recursive: true, force: true })
    const missing = await primitives.publishing(buildPlan(args), 1)
    expect(checkOf(missing, 'publish-closure-verdict')?.detail).toContain('[rule:publish-closure-verdict-missing]')

    await appendClosureVerdict(appDir, {
      app: 'pub-closure',
      namespace: 'Demo',
      fingerprint,
      verdict: 'rejected',
      findings: [{ dimension: 'publish', verdict: 'fail', detail: 'fixture' }],
      evidence: [],
    })
    const rejected = await primitives.publishing(buildPlan(args), 2)
    expect(checkOf(rejected, 'publish-closure-verdict')?.detail).toContain('[rule:publish-closure-not-approved]')
  }, 120_000)

  it('never auto-approves: the shadow is recorded but the decision stays with the checks', async () => {
    await seedPrototype('pub-shadow')
    const { args, appDir, primitives } = await publishable('pub-shadow')
    // 五谓词全部为真（产物齐备/质量通过/扩展通过/裁决 approved/无未决门），
    // 但版本快照缺失：判定仍然失败——影子绝不参与放行。
    await rm(path.join(appDir, 'versions'), { recursive: true, force: true })
    const published = await primitives.publishing(buildPlan(args), 1)
    expect(published.result.runtimeValidation).toMatchObject({ valid: false })
    expect(checkOf(published, 'publish-artifact-snapshot')).toMatchObject({ ok: false })

    const trace = await readFile(path.join(appDir, 'AppAgentTraces', 'publish-shadow.jsonl'), 'utf8')
    const record = JSON.parse(trace.trim().split('\n').at(-1) ?? '{}') as { willAutoApprove: boolean }
    expect(record.willAutoApprove).toBe(true)
    expect(published.output.evidence).toContain('shadow willAutoApprove=true')
  }, 120_000)
})

describe('controlled parallel dispatch', () => {
  it('matches write surfaces by path suffix with segment wildcards', () => {
    expect(pathTouchesSurface('Demo/Apps/x/app.json', 'app.json')).toBe(true)
    expect(pathTouchesSurface('Demo/Apps/x/app.json.bak', 'app.json')).toBe(false)
    expect(pathTouchesSurface('Demo/Apps/x/extensions/rules.yaml', 'extensions/*.yaml')).toBe(true)
    expect(pathTouchesSurface('Demo/Apps/x/extensions/rules.yml', 'extensions/*.yaml')).toBe(false)
    expect(pathTouchesSurface('Demo/Sources/Apps/Modules/m/module.json', 'module.json')).toBe(true)
    expect(pathTouchesSurface('anything', '')).toBe(false)
  })

  it('forces shared-write units serial (single owner) and keeps results in registration order', async () => {
    const events: string[] = []
    const unit = (id: string, writes: readonly string[]) => ({
      id,
      writes,
      run: async () => {
        events.push(`start:${id}`)
        await delay(id === 'private-a' ? 25 : 1)
        events.push(`end:${id}`)
        return id
      },
    })
    const outcome = await runControlledParallel(
      [
        unit('shared-1', ['Demo/Apps/x/app.json']),
        unit('private-a', ['Demo/Apps/x/notes/a.md']),
        unit('private-b', ['Demo/Apps/x/notes/b.md']),
        unit('shared-2', ['Demo/Apps/x/extensions/rules.yaml']),
      ],
      { concurrency: 2, sharedWriteSurfaces: PIPELINE_SHARED_SURFACES },
    )

    expect(outcome.ok).toBe(true)
    expect(outcome.exclusive).toEqual(['shared-1', 'shared-2'])
    // Deterministic ordering regardless of completion order.
    expect(outcome.results.map(result => result.id)).toEqual(['shared-1', 'private-a', 'private-b', 'shared-2'])
    expect(outcome.results.every(result => result.status === 'ok')).toBe(true)
    // Shared units never overlap with anything: each owns its window outright.
    const window = (id: string) => [events.indexOf(`start:${id}`), events.indexOf(`end:${id}`)] as const
    const [s1Start, s1End] = window('shared-1')
    const [s2Start, s2End] = window('shared-2')
    expect(s1End).toBeLessThan(s2Start)
    expect([...events.slice(s1Start + 1, s1End), ...events.slice(s2Start + 1, s2End)]).toEqual([])
    // Private units did run concurrently inside their own batch.
    const [p1Start] = window('private-a')
    const [p2Start] = window('private-b')
    expect(Math.abs(p1Start - p2Start)).toBeLessThanOrEqual(1)
  })

  it('defaults to concurrency 1 and converges failures into an explicit incomplete list', async () => {
    const order: string[] = []
    const outcome = await runControlledParallel([
      { id: 'a', writes: ['Demo/Apps/x/a.md'], run: async () => { order.push('a'); return 'a' } },
      { id: 'b', writes: ['Demo/Apps/x/b.md'], run: async () => { order.push('b'); throw new Error('boom') } },
      { id: 'c', writes: ['Demo/Apps/x/c.md'], run: async () => { order.push('c'); return 'c' } },
    ], { sharedWriteSurfaces: PIPELINE_SHARED_SURFACES })

    expect(outcome.concurrency).toBe(1)
    expect(outcome.ok).toBe(false)
    expect(order).toEqual(['a', 'b'])
    expect(outcome.results.map(result => result.status)).toEqual(['ok', 'failed', 'skipped'])
    expect(outcome.incomplete).toEqual(['b', 'c'])
    expect(outcome.results[1]?.error).toBe('boom')
  })

  it('treats a unit with no declared write surface as exclusive and rejects duplicate ids', async () => {
    const outcome = await runControlledParallel([
      { id: 'unknown-writes', writes: [], run: async () => 'x' },
    ], { concurrency: 8 })
    expect(outcome.exclusive).toEqual(['unknown-writes'])
    await expect(runControlledParallel([
      { id: 'dup', writes: ['a'], run: async () => 1 },
      { id: 'dup', writes: ['b'], run: async () => 2 },
    ])).rejects.toThrow('单元 id 重复')
  })
})
