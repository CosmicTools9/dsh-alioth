/**
 * The complete AppAgent pipeline as a deterministic TS state machine.
 * Mirrors the ACTIVE Meta AppAgent flow (unified contracts in
 * `agent-contract.ts`): app creation → semantic analysis → function
 * decomposition → ontology analysis → module/block creation → ontology
 * transfer → service API → E2E verification (retry ≤3) → publishing →
 * pipeline advance (7 metadata gate sweep, pause at human gates) → published.
 * Each stage executes through injected primitives (the plugin tools /
 * libraries); LLM is used ONLY at the semantic-analysis stage (the sanctioned
 * seam). No LLM anywhere else — the machine is deterministic and testable.
 *
 * Alignment note (2026-08-25, remove-appagent-hollow-analysis-stages): the
 * active Meta line deprecated the three analysis states (SemanticAnalysis /
 * FunctionDecomposition / OntologyAnalysis — keyword shells and passthrough)
 * and moved real analysis into Planning (LLM ontology output). dsh-alioth
 * keeps the stages because they carry real deterministic work here — audit
 * confirmation, registry grounding, entity registration — and the pipeline
 * order stays wire-compatible with the Meta serde enum.
 * @module @dsh-alioth/skill-alioth/agent-machine
 */

import type { AgentState, BuildResult, FlowPlan, PipelineTransition } from './agent-contract.ts'
import { PIPELINE_ORDER, STAGE_IDS } from './agent-contract.ts'

/** Stage execution result feeding the transition. */
export interface StageOutput {
  /** Artifacts produced at this stage (paths, ids). */
  readonly artifacts?: readonly string[]
  /** Human-readable evidence line for the stage. */
  readonly evidence: string
}

/** Primitive hooks each stage maps to. Injected by the host (tool plugin). */
export interface AgentPrimitives {
  /** App creation: namespace + intent → app container skeleton. */
  readonly appCreation: (input: string) => Promise<StageOutput>
  /** Semantic analysis: natural language → intent/concept hits. */
  readonly semanticAnalysis: (input: string) => Promise<StageOutput>
  /** Function decomposition: intent → functional units (FlowPlan draft). */
  readonly functionDecomposition: (input: string) => Promise<StageOutput>
  /** Ontology analysis: entities/coordinates resolution (validated). */
  readonly ontologyAnalysis: (round: number, plan: FlowPlan) => Promise<StageOutput>
  /** Module creation/assembly. */
  readonly moduleCreation: (plan: FlowPlan) => Promise<StageOutput>
  /** Block creation. */
  readonly blockCreation: (plan: FlowPlan) => Promise<StageOutput>
  /** Ontology transfer: analysis → Factor layer (service ontology). */
  readonly ontologyTransfer: (plan: FlowPlan) => Promise<StageOutput>
  /** Service API generation. */
  readonly serviceApi: (plan: FlowPlan) => Promise<StageOutput>
  /** E2E verification (real browser full chain); false → repair loop. */
  readonly e2eVerification: (attempt: number, plan: FlowPlan) => Promise<StageOutput>
  /** Publishing: validation + build gate. */
  readonly publishing: (plan: FlowPlan, attempt: number) => Promise<{ output: StageOutput; result: BuildResult }>
  /** Pipeline advance: run the auto-gate for one metadata stage. */
  readonly pipelineAdvance: (stage: string, plan: FlowPlan) => Promise<StageOutput>
  /** Human gate answer injection (PTC: caller-provided; default confirm). */
  readonly resolveGate: (gateId: string, prompt: string) => Promise<'confirm' | 'reject'>
}

export interface AgentRun {
  readonly state: AgentState
  /** Transition log for audit/replay. */
  readonly history: readonly PipelineTransition[]
  readonly plan: FlowPlan
  /** BuildResult cached at successful publishing (consumed at terminal). */
  readonly result?: BuildResult
}

function initialState(): AgentState {
  return { kind: 'app-creation' }
}

/** The stage kind for a state; terminal states have no pipeline position. */
export function stageOf(state: AgentState): string | null {
  // pipeline-gate-awaiting stays advanceable: resolveGate is injected, so the
  // sweep continues without external blocking.
  if (state.kind === 'published' || state.kind === 'failed' || state.kind === 'awaiting-user-input') {
    return null
  }
  return state.kind
}

/**
 * Advance the pipeline one stage. Deterministic: the transition depends only
 * on the current state and the injected primitive's output. Retry loops are
 * bounded: E2E verification ≤3 attempts, publishing ≤maxPublishAttempts.
 * PipelineAdvance walks STAGE_IDS; a human gate pauses the run
 * (pipeline-gate-awaiting) until resolveGate answers. Semantic analysis is
 * the only stage that may consult the LLM via the injected primitive — the
 * machine itself never calls a model.
 */
export async function advance(
  run: AgentRun,
  primitives: AgentPrimitives,
  input: string,
  maxPublishAttempts = 3,
): Promise<{ run: AgentRun; transition: PipelineTransition }> {
  const { state, plan, history } = run
  switch (state.kind) {
    case 'app-creation': {
      const output = await primitives.appCreation(input)
      return {
        run: { state: { kind: 'semantic-analysis' }, plan, history },
        transition: { from: state, to: { kind: 'semantic-analysis' }, ...(output.artifacts === undefined ? {} : { artifacts: output.artifacts }), },
      }
    }
    case 'semantic-analysis': {
      const output = await primitives.semanticAnalysis(input)
      return {
        run: { state: { kind: 'function-decomposition' }, plan, history },
        transition: { from: state, to: { kind: 'function-decomposition' }, ...(output.artifacts === undefined ? {} : { artifacts: output.artifacts }), },
      }
    }
    case 'function-decomposition': {
      const output = await primitives.functionDecomposition(input)
      return {
        run: { state: { kind: 'ontology-analysis', ontologyRound: 0 }, plan, history },
        transition: { from: state, to: { kind: 'ontology-analysis', ontologyRound: 0 }, ...(output.artifacts === undefined ? {} : { artifacts: output.artifacts }), },
      }
    }
    case 'ontology-analysis': {
      const output = await primitives.ontologyAnalysis(state.ontologyRound, plan)
      return {
        run: { state: { kind: 'module-creation' }, plan, history },
        transition: { from: state, to: { kind: 'module-creation' }, ...(output.artifacts === undefined ? {} : { artifacts: output.artifacts }), },
      }
    }
    case 'module-creation': {
      const output = await primitives.moduleCreation(plan)
      return {
        run: { state: { kind: 'block-creation' }, plan, history },
        transition: { from: state, to: { kind: 'block-creation' }, ...(output.artifacts === undefined ? {} : { artifacts: output.artifacts }), },
      }
    }
    case 'block-creation': {
      const output = await primitives.blockCreation(plan)
      return {
        run: { state: { kind: 'ontology-transfer' }, plan, history },
        transition: { from: state, to: { kind: 'ontology-transfer' }, ...(output.artifacts === undefined ? {} : { artifacts: output.artifacts }), },
      }
    }
    case 'ontology-transfer': {
      const output = await primitives.ontologyTransfer(plan)
      return {
        run: { state: { kind: 'service-api' }, plan, history },
        transition: { from: state, to: { kind: 'service-api' }, ...(output.artifacts === undefined ? {} : { artifacts: output.artifacts }), },
      }
    }
    case 'service-api': {
      const output = await primitives.serviceApi(plan)
      return {
        run: { state: { kind: 'e2e-verification', attempt: 1 }, plan, history },
        transition: { from: state, to: { kind: 'e2e-verification', attempt: 1 }, ...(output.artifacts === undefined ? {} : { artifacts: output.artifacts }), },
      }
    }
    case 'e2e-verification': {
      const attempt = state.attempt
      const output = await primitives.e2eVerification(attempt, plan)
      if (output.evidence.startsWith('E2E failed')) {
        if (attempt < 3) {
          const retry: AgentState = { kind: 'e2e-verification', attempt: attempt + 1 }
          return { run: { state: retry, plan, history }, transition: { from: state, to: retry, ...(output.artifacts === undefined ? {} : { artifacts: output.artifacts }), } }
        }
        const terminal: AgentState = { kind: 'failed', error: `E2E verification exceeded 3 attempts: ${output.evidence}` }
        return { run: { state: terminal, plan, history }, transition: { from: state, to: terminal } }
      }
      return {
        run: { state: { kind: 'publishing', publishAttempt: 1 }, plan, history },
        transition: { from: state, to: { kind: 'publishing', publishAttempt: 1 }, ...(output.artifacts === undefined ? {} : { artifacts: output.artifacts }), },
      }
    }
    case 'publishing': {
      const attempt = state.publishAttempt
      if (attempt > maxPublishAttempts) {
        const terminal: AgentState = { kind: 'failed', error: `publishing exceeded ${maxPublishAttempts} attempts` }
        return { run: { state: terminal, plan, history }, transition: { from: state, to: terminal } }
      }
      const { output, result } = await primitives.publishing(plan, attempt)
      if (!(result.runtimeValidation?.valid ?? false)) {
        const retry: AgentState = { kind: 'publishing', publishAttempt: attempt + 1, lastError: output.evidence }
        return { run: { state: retry, plan, history }, transition: { from: state, to: retry, ...(output.artifacts === undefined ? {} : { artifacts: output.artifacts }), } }
      }
      const next: AgentState = { kind: 'pipeline-advance', stageIndex: 0 }
      return {
        run: { state: next, plan, history, ...(result === undefined ? {} : { result }) },
        transition: { from: state, to: next, ...(output.artifacts === undefined ? {} : { artifacts: output.artifacts }), },
      }
    }
    case 'pipeline-advance': {
      const stage = STAGE_IDS[state.stageIndex]
      if (stage === undefined) {
        if (run.result === undefined) {
          throw new Error('agent-machine: published result unavailable — publishing never succeeded')
        }
        const terminal: AgentState = { kind: 'published', result: run.result }
        return { run: { state: terminal, plan, history }, transition: { from: state, to: terminal } }
      }
      const output = await primitives.pipelineAdvance(stage, plan)
      if (output.evidence.startsWith('HUMAN-GATE')) {
        const awaiting: AgentState = { kind: 'pipeline-gate-awaiting', stageIndex: state.stageIndex, gateId: stage, prompt: output.evidence }
        return { run: { state: awaiting, plan, history, ...(run.result === undefined ? {} : { result: run.result }) }, transition: { from: state, to: awaiting, ...(output.artifacts === undefined ? {} : { artifacts: output.artifacts }), } }
      }
      if (output.evidence.startsWith('GATE-FAIL')) {
        const terminal: AgentState = { kind: 'failed', error: `gate ${stage} failed: ${output.evidence}` }
        return { run: { state: terminal, plan, history }, transition: { from: state, to: terminal } }
      }
      const next: AgentState = { kind: 'pipeline-advance', stageIndex: state.stageIndex + 1, lastConfirmedGate: state.stageIndex }
      return { run: { state: next, plan, history, ...(run.result === undefined ? {} : { result: run.result }) }, transition: { from: state, to: next, ...(output.artifacts === undefined ? {} : { artifacts: output.artifacts }), } }
    }
    case 'pipeline-gate-awaiting': {
      const answer = await primitives.resolveGate(state.gateId, state.prompt)
      if (answer === 'reject') {
        const terminal: AgentState = { kind: 'failed', error: `human gate ${state.gateId} rejected` }
        return { run: { state: terminal, plan, history }, transition: { from: state, to: terminal } }
      }
      const next: AgentState = { kind: 'pipeline-advance', stageIndex: state.stageIndex + 1, lastConfirmedGate: state.stageIndex }
      return { run: { state: next, plan, history, ...(run.result === undefined ? {} : { result: run.result }) }, transition: { from: state, to: next } }
    }
    default:
      throw new Error(`agent-machine: cannot advance from terminal/legacy state ${state.kind}`)
  }
}

/** Run the full pipeline to a terminal or pausing state. */
export async function runPipeline(
  input: string,
  primitives: AgentPrimitives,
  initialPlan: FlowPlan,
): Promise<AgentRun> {
  let run: AgentRun = { state: initialState(), plan: initialPlan, history: [] }
  const history: PipelineTransition[] = []
  // Bound: E2E ≤3 + publishing ≤3 + 7 gates + 9 stages + slack.
  const maxAdvances = PIPELINE_ORDER.length + 24
  for (let i = 0; i < maxAdvances && stageOf(run.state) !== null; i++) {
    const advanced = await advance(run, primitives, input)
    history.push(advanced.transition)
    run = { ...advanced.run, history }
  }
  return run
}
