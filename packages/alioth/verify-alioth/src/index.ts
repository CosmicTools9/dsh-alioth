/**
 * `@dsh-alioth/verify-alioth` —— AppAgent 验证 / 闭环的**纯库**（只用 node 内建 + `yaml` 真解析器，
 * 无 harness 依赖）：评估报告、扩展声明运行时验证、独立结束审计、产物版本快照与回退、
 * 两段式补丁、构建回归基线、能力广告、阻塞登记、用量成本、7 阶段判据、publish 影子五谓词。
 *
 * 全局纪律（MUST NOT 软化）：缺失即 0 分 + violation（缺失 ≠ 满分）；degraded ≠ passed；
 * 回退全有或全无；两段式补丁未确认不写盘；不可得必须显式 `unknown(reason)`。
 * @module @dsh-alioth/verify-alioth
 */

export {
  buildEvalReport,
  writeEvalReport,
  EVAL_REPORT_SCHEMA_VERSION,
  EVAL_REPORT_DIMENSIONS,
  type EvalReport,
  type EvalViolation,
} from './eval-report.ts'
export {
  verifyExtensions,
  writeExtensionVerify,
  EXTENSION_VERIFY_SCHEMA_VERSION,
  type DeclarationCoverage,
  type ExtensionVerification,
} from './extension-verify.ts'
export {
  appendClosureVerdict,
  artifactFingerprint,
  latestMatchingVerdict,
  readClosureVerdicts,
  CLOSURE_SCHEMA_VERSION,
  ESCALATE_THRESHOLD,
  type ClosureFinding,
  type ClosureVerdict,
} from './closure-audit.ts'
export {
  listSnapshots,
  restoreSnapshot,
  snapshotArtifacts,
  KEEP_VERSIONS,
  SNAPSHOT_MANIFEST,
  type SnapshotEntry,
  type SnapshotResult,
} from './artifact-version.ts'
export { applyPatchProposal, proposePatch, type PatchProposal } from './patch-assets.ts'
export {
  decideRegression,
  parseEvalCases,
  scoreRun,
  EVAL_BASELINE_SCHEMA,
  EVAL_DIMENSIONS,
  type EvalCase,
  type EvalCaseSet,
  type EvalEvidence,
} from './eval-baseline.ts'
export {
  collectCapabilities,
  type AutonomyFact,
  type BudgetFact,
  type CapabilityReport,
  type CapabilityValue,
  type GatesFact,
  type LlmFact,
  type SkillFact,
} from './capabilities.ts'
export {
  createDeferredStore,
  DEFERRED_SCHEMA_VERSION,
  type DeferredItem,
  type DeferredStore,
  type DeferredTrigger,
} from './deferred.ts'
export { aggregateUsage, estimateCost, type PriceTable, type UsageEvent, type UsageSummary } from './usage.ts'
export { evaluateStageGate, type StageGateOutcome, type StageId } from './stage-gates.ts'
export { evaluatePublishShadow, PUBLISH_SHADOW_PREDICATES, type PublishShadow } from './auto-gate.ts'
