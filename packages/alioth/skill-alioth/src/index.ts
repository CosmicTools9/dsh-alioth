/**
 * Alioth skill-adapter orchestration primitives: adapter parsing, the
 * track/step state machine, gate checking, and file-backed run state. Pure
 * (no harness deps) so tests and future tool/skill wiring share one model.
 * @module @dsh-alioth/skill-alioth
 */

export {
  parseAdapterDocument,
  loadAdapter,
  parseRuntimeAllowedPrograms,
  type Adapter,
  type Step,
  type StepGate,
  type StepSchema,
  type Track,
} from './adapter.ts'
export {
  initialRunState,
  currentStep,
  completeCurrentStep,
  type RunPosition,
  type RunState,
  type RunTransition,
} from './state.ts'
export {
  checkGate,
  checkStepGates,
  classifyGateError,
  isLlmFixable,
  type GateContext,
  type GateErrorKind,
  type GateResult,
  type GateStatus,
  type ProgramResult,
  type ProgramRunner,
} from './gates.ts'
export { loadRun, saveRun, type RunMeta } from './workspace.ts'
export { ADAPTER_TOOL_TO_DSH, missingToolSurface, type MissingTool } from './mapping.ts'
export { createProgramRunner, bunAvailable, type ProgramRunnerOptions } from './bun.ts'
export {
  validateCoordinates,
  validateEntitySpec,
  validateMappedColumn,
  LOCAL_KEYS_BY_TABLE as physicalLocalKeys,
  ROOT_COLUMNS as physicalRootColumns,
  type PhysicalColumnIndex,
  type CoordinatesSpec,
  type EntitySpec,
  type FieldSpec,
  type ReferenceSpec,
  type RegistryView,
  type ValidationIssue,
} from './entity-validate.ts'
export {
  buildE2eReport,
  writeE2eReport,
  E2E_REPORT_SCHEMA_VERSION,
  type E2eCheck,
  type E2eReport,
} from './e2e-report.ts'
