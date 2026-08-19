/**
 * Unified data contracts with AliothStudio Meta's AppAgent.
 *
 * The source of truth is the frozen AppAgent pipeline contract (vendored
 * `app-agent/src/state.rs` from the model distribution): the 7-stage
 * pipeline state machine and its data structures (FlowPlan, BuildResult,
 * StepResult, ...). Serialization aligns with the Rust `serde` shapes
 * including aliases (`created_scenes`/`created_factors`, `SceneCreation`).
 * dsh-alioth's agent machine and the Meta AppAgent speak the same language —
 * sessions and artifacts are interchangeable.
 * @module @dsh-alioth/skill-alioth/agent-contract
 */

/** The AppAgent pipeline state (7-stage line plus terminal states). */
export type AgentState =
  | { readonly kind: 'semantic-analysis' }
  | { readonly kind: 'function-decomposition' }
  | { readonly kind: 'ontology-analysis'; readonly ontologyRound: number }
  | { readonly kind: 'module-creation' }
  | { readonly kind: 'block-creation' }
  | { readonly kind: 'ontology-transfer' }
  | { readonly kind: 'service-api' }
  | { readonly kind: 'publishing'; readonly publishAttempt: number; readonly lastError?: string }
  | { readonly kind: 'published'; readonly result: BuildResult }
  // Backward-compatible legacy states (older sessions deserialize into them).
  | { readonly kind: 'initializing' }
  | { readonly kind: 'planning' }
  | { readonly kind: 'composing' }
  | { readonly kind: 'verifying' }
  | { readonly kind: 'awaiting-user-input'; readonly reason?: string }
  | { readonly kind: 'failed'; readonly error?: string }

/** The 7-stage pipeline order; `published` is the only successful terminal. */
export const PIPELINE_ORDER = [
  'semantic-analysis',
  'function-decomposition',
  'ontology-analysis',
  'module-creation',
  'block-creation',
  'ontology-transfer',
  'service-api',
  'publishing',
] as const

export type PipelineStage = typeof PIPELINE_ORDER[number]

/** FlowPlan — the planning-phase artifact shared with the Meta AppAgent. */
export interface FlowPlan {
  readonly usedModules: readonly string[]
  readonly namespace: string
  readonly knownEntities: readonly string[]
  readonly workflowSteps: readonly string[]
  readonly missingInfo: readonly MissingInfo[]
  /** 7-stage outputs. */
  readonly createdModules: readonly string[]
  /** serde alias `created_scenes`. */
  readonly createdBlocks: readonly string[]
  /** serde alias `created_factors`. */
  readonly createdServices: readonly string[]
}

/** BuildResult — the published artifact descriptor (Meta wire shape). */
export interface BuildResult {
  readonly appName: string
  readonly outputPath: string
  readonly usedModules: readonly ModuleUsage[]
  readonly extensions: readonly ExtensionResult[]
  readonly generatedFiles: readonly string[]
  readonly pendingConfirmations: readonly string[]
  readonly endpointUrl?: string
  /** Prototype preview URL (e.g. /apps/{namespace}/{code}/prototype.html). */
  readonly previewUrl?: string
  readonly runtimeValidation?: RunValidationResult
}

export interface MissingInfo {
  readonly category: string
  readonly description: string
}

export interface ModuleUsage {
  readonly moduleId: string
  readonly name: string
  readonly blocks: readonly string[]
}

export interface ExtensionResult {
  readonly kind: string
  readonly content: string
}

export interface RunValidationResult {
  readonly valid: boolean
  readonly checks: readonly { readonly name: string; readonly ok: boolean; readonly detail: string }[]
}

/** Next-stage transition of the pipeline. */
export interface PipelineTransition {
  readonly from: AgentState
  readonly to: AgentState
  /** Transition metadata (artifact references produced at the stage). */
  readonly artifacts?: readonly string[]
}

/** Serialize a stage to its wire representation (serde-compatible tag). */
export function serializeStage(kind: AgentState['kind']): string {
  return kind
}

/** Parse a wire tag (serde alias aware) into a stage kind; `null` when unknown. */
export function parseStageTag(tag: string): AgentState['kind'] | null {
  switch (tag) {
    case 'SemanticAnalysis': case 'semantic-analysis': return 'semantic-analysis'
    case 'FunctionDecomposition': case 'function-decomposition': return 'function-decomposition'
    case 'OntologyAnalysis': case 'ontology-analysis': return 'ontology-analysis'
    case 'ModuleCreation': case 'module-creation': return 'module-creation'
    case 'BlockCreation': case 'SceneCreation': case 'block-creation': return 'block-creation'
    case 'OntologyTransfer': case 'ontology-transfer': return 'ontology-transfer'
    case 'ServiceAPI': case 'factor_api': case 'FactorAPI': case 'service-api': return 'service-api'
    case 'Publishing': case 'publishing': return 'publishing'
    case 'Published': case 'published': return 'published'
    case 'Initializing': case 'initializing': return 'initializing'
    case 'Planning': case 'planning': return 'planning'
    case 'Composing': case 'composing': return 'composing'
    case 'Verifying': case 'verifying': return 'verifying'
    case 'AwaitingUserInput': case 'awaiting-user-input': return 'awaiting-user-input'
    case 'Failed': case 'failed': return 'failed'
    default: return null
  }
}
