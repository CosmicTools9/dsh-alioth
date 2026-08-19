/**
 * The complete AppAgent pipeline as a deterministic TS state machine.
 * Mirrors the Meta AppAgent 7-stage flow (unified contracts in
 * `agent-contract.ts`): semantic analysis → function decomposition →
 * ontology analysis → module creation → block creation → ontology transfer →
 * service API → publishing. Each stage executes through injected primitives
 * (the plugin tools / libraries); LLM is used ONLY at the semantic-analysis
 * stage (the sanctioned seam). No LLM anywhere else — the machine is
 * deterministic and testable.
 * @module @dsh-alioth/skill-alioth/agent-machine
 */

import type { AgentState, BuildResult, FlowPlan, PipelineTransition } from './agent-contract.ts'
import { PIPELINE_ORDER } from './agent-contract.ts'

/** Stage execution result feeding the transition. */
export interface StageOutput {
  /** Artifacts produced at this stage (paths, ids). */
  readonly artifacts?: readonly string[]
  /** Human-readable evidence line for the stage. */
  readonly evidence: string
}

/** Primitive hooks each stage maps to. Injected by the host (tool plugin). */
export interface AgentPrimitives {
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
  /** Publishing: validation + build gate. */
  readonly publishing: (plan: FlowPlan, attempt: number) => Promise<{ output: StageOutput; result: BuildResult }>
}

export interface AgentRun {
  readonly state: AgentState
  /** Transition log for audit/replay. */
  readonly history: readonly PipelineTransition[]
  readonly plan: FlowPlan
}

function initialState(): AgentState {
  return { kind: 'semantic-analysis' }
}

/** The stage kind for a state; terminal states have no pipeline position. */
export function stageOf(state: AgentState): string | null {
  if (state.kind === 'published' || state.kind === 'failed' || state.kind === 'awaiting-user-input') {
    return null
  }
  return state.kind
}

/**
 * Advance the pipeline one stage. Deterministic: the transition depends only
 * on the current state and the injected primitive's output. `publishing`
 * loops with a bounded attempt count on validation failure (Publishing
 * carries `publish_attempt`); semantic analysis is the only stage that may
 * consult the LLM via the injected primitive — the machine itself never calls
 * a model.
 */
export async function advance(
  run: AgentRun,
  primitives: AgentPrimitives,
  input: string,
  maxPublishAttempts = 3,
): Promise<{ run: AgentRun; transition: PipelineTransition }> {
  const { state, plan, history } = run
  switch (state.kind) {
    case 'semantic-analysis': {
      const output = await primitives.semanticAnalysis(input)
      return {
        run: { state: { kind: 'function-decomposition' }, plan, history },
        transition: { from: state, to: { kind: 'function-decomposition' }, artifacts: output.artifacts },
      }
    }
    case 'function-decomposition': {
      const output = await primitives.functionDecomposition(input)
      return {
        run: { state: { kind: 'ontology-analysis', ontologyRound: 0 }, plan, history },
        transition: { from: state, to: { kind: 'ontology-analysis', ontologyRound: 0 }, artifacts: output.artifacts },
      }
    }
    case 'ontology-analysis': {
      const output = await primitives.ontologyAnalysis(state.ontologyRound, plan)
      return {
        run: { state: { kind: 'module-creation' }, plan, history },
        transition: { from: state, to: { kind: 'module-creation' }, artifacts: output.artifacts },
      }
    }
    case 'module-creation': {
      const output = await primitives.moduleCreation(plan)
      return {
        run: { state: { kind: 'block-creation' }, plan, history },
        transition: { from: state, to: { kind: 'block-creation' }, artifacts: output.artifacts },
      }
    }
    case 'block-creation': {
      const output = await primitives.blockCreation(plan)
      return {
        run: { state: { kind: 'ontology-transfer' }, plan, history },
        transition: { from: state, to: { kind: 'ontology-transfer' }, artifacts: output.artifacts },
      }
    }
    case 'ontology-transfer': {
      const output = await primitives.ontologyTransfer(plan)
      return {
        run: { state: { kind: 'service-api' }, plan, history },
        transition: { from: state, to: { kind: 'service-api' }, artifacts: output.artifacts },
      }
    }
    case 'service-api': {
      const output = await primitives.serviceApi(plan)
      return {
        run: { state: { kind: 'publishing', publishAttempt: 1 }, plan, history },
        transition: { from: state, to: { kind: 'publishing', publishAttempt: 1 }, artifacts: output.artifacts },
      }
    }
    case 'publishing': {
      const attempt = state.publishAttempt
      if (attempt > maxPublishAttempts) {
        const terminal: AgentState = { kind: 'failed', error: `publishing exceeded ${maxPublishAttempts} attempts` }
        return { run: { state: terminal, plan, history }, transition: { from: state, to: terminal } }
      }
      const { output, result } = await primitives.publishing(plan, attempt)
      if (!result.runtimeValidation?.valid ?? false) {
        const retry: AgentState = { kind: 'publishing', publishAttempt: attempt + 1, lastError: output.evidence }
        return { run: { state: retry, plan, history }, transition: { from: state, to: retry, artifacts: output.artifacts } }
      }
      const terminal: AgentState = { kind: 'published', result }
      return { run: { state: terminal, plan, history }, transition: { from: state, to: terminal, artifacts: output.artifacts } }
    }
    default:
      throw new Error(`agent-machine: cannot advance from terminal/legacy state ${state.kind}`)
  }
}

/** Run the full pipeline to a terminal state (deterministic sequence of `advance`). */
export async function runPipeline(
  input: string,
  primitives: AgentPrimitives,
  initialPlan: FlowPlan,
): Promise<AgentRun> {
  let run: AgentRun = { state: initialState(), plan: initialPlan, history: [] }
  const history: PipelineTransition[] = []
  // Bound: publishing may retry up to maxPublishAttempts; the happy path is
  // exactly PIPELINE_ORDER.length advances.
  const maxAdvances = PIPELINE_ORDER.length + 8
  for (let i = 0; i < maxAdvances && stageOf(run.state) !== null; i++) {
    const advanced = await advance(run, primitives, input)
    history.push(advanced.transition)
    run = { ...advanced.run, history }
  }
  return run
}
