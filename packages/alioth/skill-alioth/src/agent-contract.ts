/**
 * Unified data contracts with AliothStudio Meta's AppAgent.
 *
 * The source of truth is the ACTIVE `Meta/backend/app-agent` pipeline
 * (AliothStudio `state.rs` / `pipeline/stage.rs`), NOT the frozen vendor
 * copy — the active line has evolved past it: AppCreation (stage 0),
 * E2EVerification (ego-browser E2E with retry), and the PipelineAdvance /
 * PipelineGateAwaiting metadata gate sweep after publishing. Serialization
 * aligns with the Rust `serde` shapes including aliases
 * (`created_scenes`/`created_factors`, `SceneCreation`).
 * @module @dsh-alioth/skill-alioth/agent-contract
 */

/** The AppAgent pipeline state (active Meta line: 9 stages + gates + terminals). */
export type AgentState =
  // 0. App creation: namespace + raw intent → app container (code/name/goal).
  | { readonly kind: 'app-creation' }
  // 1. Semantic analysis: raw intent → business intent and key concepts.
  | { readonly kind: 'semantic-analysis' }
  // 2. Function decomposition: intent → functional units (module/scene/factor).
  | { readonly kind: 'function-decomposition' }
  // 3. Ontology analysis: map to the Alioth ontology (entities/relations/coordinates).
  | { readonly kind: 'ontology-analysis'; readonly ontologyRound: number }
  // 4. Module creation/assembly.
  | { readonly kind: 'module-creation' }
  // 5. Block creation (serde alias `SceneCreation`).
  | { readonly kind: 'block-creation' }
  // 6. Ontology transfer: analysis → Factor layer.
  | { readonly kind: 'ontology-transfer' }
  // 7. Service API generation (serde aliases `factor_api`/`FactorAPI`).
  | { readonly kind: 'service-api' }
  // 7.5 E2E verification: real browser full chain (frontend→API→DB); failure
  //     loops back for repair (≤3 attempts).
  | { readonly kind: 'e2e-verification'; readonly attempt: number }
  // Publish: compile validation + release to Gateway.
  | { readonly kind: 'publishing'; readonly publishAttempt: number; readonly lastError?: string }
  // Post-publish: sweep the 7 metadata-scope stages, auto-gate each, pause at
  // human gates.
  | { readonly kind: 'pipeline-advance'; readonly stageIndex: number; readonly lastConfirmedGate?: number }
  // Waiting for the user's answer on a pipeline human gate.
  | { readonly kind: 'pipeline-gate-awaiting'; readonly stageIndex: number; readonly gateId: string; readonly prompt: string }
  | { readonly kind: 'published'; readonly result: BuildResult }
  // Backward-compatible legacy states (older sessions deserialize into them).
  | { readonly kind: 'initializing' }
  | { readonly kind: 'planning' }
  | { readonly kind: 'composing' }
  | { readonly kind: 'verifying' }
  | { readonly kind: 'awaiting-user-input'; readonly reason?: string }
  | { readonly kind: 'failed'; readonly error?: string }

/** The 9-stage pipeline order; `published` is the only successful terminal. */
export const PIPELINE_ORDER = [
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
] as const

export type PipelineStage = typeof PIPELINE_ORDER[number]

/** The 7 metadata-scope gate stages swept by PipelineAdvance (StageId::all). */
export const STAGE_IDS = [
  'appagent-ready',
  'module-design',
  'block-extract',
  'block-refinement',
  'ontology-mapping',
  'factor-dev',
  'quality',
] as const

export type StageId = typeof STAGE_IDS[number]

/** FlowPlan — the planning-phase artifact shared with the Meta AppAgent. */
export interface FlowPlan {
  readonly usedModules: readonly string[]
  readonly namespace: string
  readonly knownEntities: readonly string[]
  readonly workflowSteps: readonly string[]
  readonly missingInfo: readonly MissingInfo[]
  /** 7-stage outputs (aliases `created_scenes`/`created_factors`). */
  readonly createdModules: readonly string[]
  readonly createdBlocks: readonly string[]
  readonly createdServices: readonly string[]
  /** Ontology analysis result (OntologyModel JSON). */
  readonly ontologyModelJson?: string
  /** Function decomposition result: functional unit list. */
  readonly functionalUnits?: readonly FunctionalUnit[]
}

/** Functional unit from FunctionDecomposition (deterministic id from name). */
export interface FunctionalUnit {
  readonly id?: string
  readonly sourceRequirementIds: readonly string[]
  readonly name: string
  readonly description: string
  readonly entities: readonly string[]
  readonly suggestedModule?: string
  /** serde alias `suggested_scenes`. */
  readonly suggestedBlocks: readonly string[]
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
  readonly hasRuntimeError: boolean
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
    case 'AppCreation': case 'app-creation': return 'app-creation'
    case 'SemanticAnalysis': case 'semantic-analysis': return 'semantic-analysis'
    case 'FunctionDecomposition': case 'function-decomposition': return 'function-decomposition'
    case 'OntologyAnalysis': case 'ontology-analysis': return 'ontology-analysis'
    case 'ModuleCreation': case 'module-creation': return 'module-creation'
    case 'BlockCreation': case 'SceneCreation': case 'block-creation': return 'block-creation'
    case 'OntologyTransfer': case 'ontology-transfer': return 'ontology-transfer'
    case 'ServiceAPI': case 'factor_api': case 'FactorAPI': case 'service-api': return 'service-api'
    case 'E2EVerification': case 'e2e-verification': return 'e2e-verification'
    case 'Publishing': case 'publishing': return 'publishing'
    case 'PipelineAdvance': case 'pipeline-advance': return 'pipeline-advance'
    case 'PipelineGateAwaiting': case 'pipeline-gate-awaiting': return 'pipeline-gate-awaiting'
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
