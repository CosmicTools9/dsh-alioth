import { describe, expect, it } from 'vitest'
import { parseStageTag, serializeStage, PIPELINE_ORDER, STAGE_IDS } from '../src/agent-contract.ts'
import { advance, runPipeline, stageOf } from '../src/agent-machine.ts'
import type { AgentPrimitives, AgentRun } from '../src/agent-machine.ts'
import type { AgentState, BuildResult, FlowPlan } from '../src/agent-contract.ts'

const plan: FlowPlan = {
  usedModules: ['m-inventory'],
  namespace: 'test',
  knownEntities: ['inventory-balance'],
  workflowSteps: ['query-balance'],
  missingInfo: [],
  createdModules: [],
  createdBlocks: [],
  createdServices: [],
}

const buildResult: BuildResult = {
  appName: 'inventory',
  outputPath: '/pre-proc/test/app.json',
  usedModules: [{ moduleId: 'm-inventory', name: '库存', blocks: [] }],
  extensions: [],
  generatedFiles: ['/pre-proc/test/app.json'],
  pendingConfirmations: [],
  previewUrl: '/apps/test/inventory/prototype.html',
  runtimeValidation: { valid: true, checks: [{ name: 'contract', ok: true, detail: 'ok' }] },
  hasRuntimeError: false,
}

function primitives(overrides: Partial<AgentPrimitives> = {}): AgentPrimitives {
  const ok = (label: string) => async () => ({ evidence: label })
  return {
    appCreation: ok('app'),
    semanticAnalysis: ok('semantic'),
    functionDecomposition: ok('decompose'),
    ontologyAnalysis: ok('ontology'),
    moduleCreation: ok('module'),
    blockCreation: ok('block'),
    ontologyTransfer: ok('transfer'),
    serviceApi: ok('service'),
    e2eVerification: ok('e2e'),
    publishing: async () => ({ output: { evidence: 'publish' }, result: buildResult }),
    pipelineAdvance: ok('gate-pass'),
    resolveGate: async () => 'confirm',
    ...overrides,
  }
}

function runWith(state: AgentState): AgentRun {
  return { state, plan, history: [] }
}

describe('agent-contract', () => {
  it('round-trips every pipeline stage through serde-compatible tags', () => {
    for (const kind of PIPELINE_ORDER) {
      expect(parseStageTag(serializeStage(kind))).toBe(kind)
    }
  })

  it('accepts serde aliases and active-Meta tags from the Rust wire shapes', () => {
    expect(parseStageTag('SceneCreation')).toBe('block-creation')
    expect(parseStageTag('factor_api')).toBe('service-api')
    expect(parseStageTag('FactorAPI')).toBe('service-api')
    expect(parseStageTag('AppCreation')).toBe('app-creation')
    expect(parseStageTag('E2EVerification')).toBe('e2e-verification')
    expect(parseStageTag('PipelineAdvance')).toBe('pipeline-advance')
    expect(parseStageTag('PipelineGateAwaiting')).toBe('pipeline-gate-awaiting')
    expect(parseStageTag('CreatedBlocks')).toBeNull()
  })

  it('exposes the 7 metadata gate stage ids', () => {
    expect(STAGE_IDS).toHaveLength(7)
    expect(STAGE_IDS).toContain('appagent-ready')
    expect(STAGE_IDS).toContain('quality')
  })
})

describe('agent-machine', () => {
  it('walks the full active-Meta pipeline (9 stages + 7 gates) to published', async () => {
    const run = await runPipeline('查询库存余额', primitives(), plan)
    expect(run.state.kind).toBe('published')
    expect((run.state as { result: BuildResult }).result.hasRuntimeError).toBe(false)
    expect(stageOf(run.state)).toBeNull()
  })

  it('tracks the exact stage sequence', async () => {
    const run = await runPipeline('查询库存余额', primitives(), plan)
    const kinds = run.history.map(t => t.from.kind)
    expect(kinds).toEqual([
      'app-creation',
      'semantic-analysis',
      'function-decomposition',
      'ontology-analysis',
      'module-creation',
      'block-creation',
      'ontology-transfer',
      'service-api',
      'e2e-verification',
      'publishing',
      ...STAGE_IDS.map(() => 'pipeline-advance'),
      'pipeline-advance', // terminal check (stage index past the sweep)
    ])
  })

  it('runs the gate sweep in StageId order', async () => {
    const seen: string[] = []
    const run = await runPipeline('查询库存余额', primitives({
      pipelineAdvance: async (stage) => { seen.push(stage); return { evidence: 'gate-pass' } },
    }), plan)
    expect(run.state.kind).toBe('published')
    expect(seen).toEqual([...STAGE_IDS])
  })

  it('carries artifact evidence on transitions', async () => {
    const run = await runPipeline('查询库存余额', primitives({
      ontologyAnalysis: async () => ({ evidence: 'entities: 3', artifacts: ['/pre-proc/test/_schema/entity.json'] }),
    }), plan)
    const ontology = run.history.find(t => t.from.kind === 'ontology-analysis')
    expect(ontology?.artifacts).toEqual(['/pre-proc/test/_schema/entity.json'])
  })

  it('retries E2E verification up to 3 attempts, then fails', async () => {
    const failing = primitives({ e2eVerification: async attempt => ({ evidence: `E2E failed (attempt ${attempt})` }) })
    const run = await runPipeline('查询库存余额', failing, plan)
    expect(run.state.kind).toBe('failed')
    expect((run.state as { error?: string }).error).toContain('3 attempts')
    expect(run.history.filter(t => t.from.kind === 'e2e-verification')).toHaveLength(3)
  })

  it('recovers from an E2E failure on the second attempt', async () => {
    let attempt = 0
    const flaky = primitives({
      e2eVerification: async () => {
        attempt += 1
        return { evidence: attempt === 1 ? 'E2E failed (attempt 1)' : 'e2e ok' }
      },
    })
    const run = await runPipeline('查询库存余额', flaky, plan)
    expect(run.state.kind).toBe('published')
    expect(run.history.filter(t => t.from.kind === 'e2e-verification')).toHaveLength(2)
  })

  it('retries publishing on validation failure up to the cap, then fails', async () => {
    const failing: AgentPrimitives = primitives({
      publishing: async (_plan, attempt) => ({
        output: { evidence: `attempt ${attempt} invalid` },
        result: { ...buildResult, runtimeValidation: { valid: false, checks: [{ name: 'contract', ok: false, detail: 'bad' }] } },
      }),
    })
    const run = await runPipeline('查询库存余额', failing, plan)
    expect(run.state.kind).toBe('failed')
    expect((run.state as { error?: string }).error).toContain('3 attempts')
    expect(run.history.filter(t => t.from.kind === 'publishing')).toHaveLength(4) // attempts 1..3 + terminal failure
  })

  it('pauses at a human gate and resumes on confirm', async () => {
    let calls = 0
    const run = await runPipeline('查询库存余额', primitives({
      pipelineAdvance: async (stage) => {
        if (stage === 'factor-dev') {
          return { evidence: 'HUMAN-GATE factor-dev: review DTOs before publishing' }
        }
        return { evidence: 'gate-pass' }
      },
      resolveGate: async (gateId) => { calls += 1; return gateId === 'factor-dev' ? 'confirm' : 'confirm' },
    }), plan)
    expect(run.state.kind).toBe('published')
    expect(calls).toBe(1)
    const awaiting = run.history.find(t => t.to.kind === 'pipeline-gate-awaiting')
    expect(awaiting).toBeDefined()
  })

  it('fails when a human gate is rejected', async () => {
    const run = await runPipeline('查询库存余额', primitives({
      pipelineAdvance: async () => ({ evidence: 'HUMAN-GATE quality: review before publishing' }),
      resolveGate: async () => 'reject',
    }), plan)
    expect(run.state.kind).toBe('failed')
    expect((run.state as { error?: string }).error).toContain('rejected')
  })

  it('fails fast when a metadata gate check fails', async () => {
    const run = await runPipeline('查询库存余额', primitives({
      pipelineAdvance: async stage => ({ evidence: stage === 'block-extract' ? 'GATE-FAIL block-extract: artifact missing' : 'gate-pass' }),
    }), plan)
    expect(run.state.kind).toBe('failed')
    expect((run.state as { error?: string }).error).toContain('block-extract')
  })

  it('cannot advance from terminal or legacy states', async () => {
    for (const terminal of [
      { kind: 'published', result: buildResult },
      { kind: 'failed', error: 'x' },
      { kind: 'awaiting-user-input' },
      { kind: 'planning' },
      { kind: 'composing' },
    ] as const) {
      await expect(advance(runWith(terminal as unknown as AgentState), primitives(), 'x')).rejects.toThrow(/cannot advance/)
    }
  })
})
