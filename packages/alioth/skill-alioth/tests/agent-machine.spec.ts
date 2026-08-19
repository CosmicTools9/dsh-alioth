import { describe, expect, it } from 'vitest'
import { parseStageTag, serializeStage, PIPELINE_ORDER } from '../src/agent-contract.ts'
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
}

function primitives(overrides: Partial<AgentPrimitives> = {}): AgentPrimitives {
  const ok = (label: string) => async () => ({ evidence: label })
  return {
    semanticAnalysis: ok('semantic'),
    functionDecomposition: ok('decompose'),
    ontologyAnalysis: ok('ontology'),
    moduleCreation: ok('module'),
    blockCreation: ok('block'),
    ontologyTransfer: ok('transfer'),
    serviceApi: ok('service'),
    publishing: async () => ({ output: { evidence: 'publish' }, result: buildResult }),
    ...overrides,
  }
}

function runWith(state: AgentState): AgentRun {
  return { state, plan, history: [] }
}

describe('agent-contract', () => {
  it('round-trips every pipeline stage through serde-compatible tags', () => {
    for (const kind of PIPELINE_ORDER) {
      const tag = serializeStage(kind)
      expect(parseStageTag(tag)).toBe(kind)
    }
  })

  it('accepts serde aliases from the Rust wire shapes', () => {
    expect(parseStageTag('SceneCreation')).toBe('block-creation')
    expect(parseStageTag('factor_api')).toBe('service-api')
    expect(parseStageTag('FactorAPI')).toBe('service-api')
    expect(parseStageTag('CreatedBlocks')).toBeNull()
  })
})

describe('agent-machine', () => {
  it('walks the full 7-stage pipeline to published', async () => {
    const run = await runPipeline('查询库存余额', primitives(), plan)
    expect(run.state.kind).toBe('published')
    expect(run.history).toHaveLength(8) // 7 stages + publishing → published
    expect(stageOf(run.state)).toBeNull()
  })

  it('tracks the exact stage sequence', async () => {
    const run = await runPipeline('查询库存余额', primitives(), plan)
    const kinds = run.history.map(t => t.from.kind)
    expect(kinds).toEqual([
      'semantic-analysis',
      'function-decomposition',
      'ontology-analysis',
      'module-creation',
      'block-creation',
      'ontology-transfer',
      'service-api',
      'publishing',
    ])
  })

  it('carries artifact evidence on transitions', async () => {
    const run = await runPipeline('查询库存余额', primitives({
      ontologyAnalysis: async () => ({ evidence: 'entities: 3', artifacts: ['/pre-proc/test/_schema/entity.json'] }),
    }), plan)
    const ontology = run.history.find(t => t.from.kind === 'ontology-analysis')
    expect(ontology?.artifacts).toEqual(['/pre-proc/test/_schema/entity.json'])
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
    const publishes = run.history.filter(t => t.from.kind === 'publishing')
    expect(publishes).toHaveLength(4) // attempts 1..3 + terminal failure
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
