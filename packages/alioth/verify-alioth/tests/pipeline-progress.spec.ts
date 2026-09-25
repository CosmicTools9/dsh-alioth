import { readFile } from 'node:fs/promises'
import path from 'node:path'
import { afterEach, describe, expect, it } from 'vitest'
import {
  PIPELINE_STAGES,
  artifactEntry,
  pipelineProgressJson,
  scanStageProgress,
  writePipelineManifest,
} from '../src/index.ts'
import { VALID_APP_JSON, createTempApp, type TempApp } from './fixture.ts'

/**
 * 诚实投影的三态语义（上游 `pipeline/progress.rs`）：
 * 缺失 = pending（MUST NOT 谎报 completed）、通过 = completed、有产物但不过 = failures。
 */
const opened: TempApp[] = []
async function app(files: Readonly<Record<string, string>> = {}): Promise<TempApp> {
  const created = await createTempApp(files)
  opened.push(created)
  return created
}
afterEach(async () => {
  await Promise.all(opened.splice(0).map(entry => entry.cleanup()))
})

function scanOf(created: TempApp, extra: { readonly atManifestWrite?: boolean } = {}) {
  return scanStageProgress({
    appDir: created.appDir,
    preProcRoot: created.preProcRoot,
    namespace: 'TestNS',
    app: 'tm-app',
    ...extra,
  })
}

describe('scanStageProgress', () => {
  it('reports every stage pending on an empty tree, never completed', async () => {
    const created = await app()
    const scan = await scanOf(created)
    expect(scan.stages).toHaveLength(7)
    expect(scan.stages.every(stage => stage.status === 'pending' && stage.gate === 'not_attempted')).toBe(true)
    // 缺失 stage MUST 带出「声明产物」取证，供消费方看清缺的是什么。
    expect(scan.stages[0]?.evidence).toContain('尚未尝试')
    expect(scan.currentStage).toBe('appagent-ready')
    expect(scan.allCompleted).toBe(false)
    expect(scan.failures).toEqual([])
  })

  it('completes a stage whose declared artifact parses, others stay pending', async () => {
    const created = await app({ 'app.json': VALID_APP_JSON })
    const scan = await scanOf(created)
    const ready = scan.stages.find(stage => stage.id === 'appagent-ready')
    expect(ready?.status).toBe('completed')
    expect(ready?.completedAt).toBeTypeOf('string')
    expect(scan.stages.find(stage => stage.id === 'quality')?.status).toBe('pending')
    expect(scan.currentStage).toBe('module-design')
    expect(scan.failures).toEqual([])
  })

  it('honours the at_manifest_write exemption for appagent-ready only', async () => {
    const created = await app()
    const scan = await scanOf(created, { atManifestWrite: true })
    const ready = scan.stages.find(stage => stage.id === 'appagent-ready')
    expect(ready?.status).toBe('completed')
    expect(ready?.evidence).toContain('at_manifest_write')
    // 其余 stage 不享受豁免（上游唯一豁免）。
    expect(scan.stages.find(stage => stage.id === 'quality')?.status).toBe('pending')
    expect(scan.failures).toEqual([])
  })

  it('reports a present-but-broken artifact as a failure, not as pending', async () => {
    const created = await app({ 'app.json': '{ not json' })
    const scan = await scanOf(created)
    expect(scan.failures.map(failure => failure.id)).toEqual(['appagent-ready'])
    expect(scan.stages.find(stage => stage.id === 'appagent-ready')?.status).toBe('pending')
  })

  it('projects the description of every stage from the declared pattern set', () => {
    expect(PIPELINE_STAGES.map(stage => stage.id)).toEqual([
      'appagent-ready', 'module-design', 'block-extract', 'block-refinement', 'ontology-mapping', 'factor-dev', 'quality',
    ])
    expect(PIPELINE_STAGES.filter(stage => stage.hasHumanGate).map(stage => stage.id))
      .toEqual(['module-design', 'block-refinement', 'ontology-mapping', 'factor-dev'])
  })
})

describe('pipeline_progress segment + manifest', () => {
  it('emits the upstream key shape with honest per-stage status', async () => {
    const created = await app({ 'app.json': VALID_APP_JSON })
    const scan = await scanOf(created)
    const progress = pipelineProgressJson(scan, { stageSource: '/pre-proc/TestNS' })
    expect(progress.current_stage).toBe('module-design')
    expect(progress.stages['appagent-ready']).toMatchObject({ status: 'completed', via: 'appagent' })
    expect(progress.stages['quality']).toEqual({ status: 'pending' })
    expect(progress.stage_source).toBe('/pre-proc/TestNS')
    expect(progress.projection.scope).toBe('metadata')
    expect(progress.projection.projected_stages).toHaveLength(7)
    expect(progress.projection.non_projected_stages).toBeNull()
  })

  it('writes an atomic manifest and skips entries that are not readable', async () => {
    const created = await app({ 'app.json': VALID_APP_JSON })
    const scan = await scanOf(created)
    const entry = await artifactEntry(created.preProcRoot, path.join(created.appDir, 'app.json'), 'appagent-ready')
    expect(entry?.content_hash).toMatch(/^sha256:[0-9a-f]{64}$/)
    expect(entry?.path).toBe('Apps/tm-app/app.json')
    expect(await artifactEntry(created.preProcRoot, path.join(created.appDir, 'missing.json'), 'quality')).toBeNull()

    const written = await writePipelineManifest(created.appDir, {
      pipeline_progress: pipelineProgressJson(scan, { stageSource: created.preProcRoot }),
      artifact_manifest: { entries: entry === null ? [] : [entry] },
      open_human_gates: ['block-interaction-form:TestNS/tm-app:orders'],
    })
    const body = JSON.parse(await readFile(written, 'utf8')) as {
      pipeline_progress: { stages: Record<string, { status: string }> }
      artifact_manifest: { entries: Array<{ path: string }> }
      open_human_gates: string[]
    }
    expect(body.pipeline_progress.stages['appagent-ready']?.status).toBe('completed')
    expect(body.artifact_manifest.entries.map(item => item.path)).toEqual(['Apps/tm-app/app.json'])
    expect(body.open_human_gates).toEqual(['block-interaction-form:TestNS/tm-app:orders'])
  })
})
