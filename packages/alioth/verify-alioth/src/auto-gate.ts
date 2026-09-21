/**
 * publish 门影子五谓词 —— 对齐上游 `auto_gate.rs`（T18 候选 A 影子模式）：
 * **零行为变更**：不自动放行、不自动拒绝，只求值五谓词并留痕，与人工实际决策做结构化对照。
 *
 * fail-closed：任一谓词输入不可判定 ⇒ 该谓词 false（与「谓词不全满足 → 维持人工确认」同向）；
 * `extensionStatus === 'degraded'` ⇒ 扩展谓词 false（degraded ≠ passed）。
 * @module @dsh-alioth/verify-alioth/auto-gate
 */

/** 影子评估结果：`willAutoApprove` = 五谓词全真；`predicates` 承载分项布尔（字段名即契约）。 */
export interface PublishShadow {
  readonly willAutoApprove: boolean
  readonly predicates: Record<string, boolean>
}

/** publish auto-approve 五谓词（字段名即契约，下游对照脚本按名消费）。 */
export const PUBLISH_SHADOW_PREDICATES: readonly string[] = [
  'artifacts_complete',
  'quality_passed',
  'extension_verify_passed',
  'closure_audit_approved',
  'no_open_deferred',
]

export function evaluatePublishShadow(input: {
  readonly artifactsComplete: boolean
  readonly qualityPassed: boolean
  readonly extensionStatus: 'passed' | 'degraded'
  readonly closureApproved: boolean
  readonly noOpenDeferred: boolean
}): PublishShadow {
  const predicates: Record<string, boolean> = {
    artifacts_complete: input.artifactsComplete === true,
    quality_passed: input.qualityPassed === true,
    extension_verify_passed: input.extensionStatus === 'passed',
    closure_audit_approved: input.closureApproved === true,
    no_open_deferred: input.noOpenDeferred === true,
  }
  return {
    willAutoApprove: Object.values(predicates).every(Boolean),
    predicates,
  }
}
