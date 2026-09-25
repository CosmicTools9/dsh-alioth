/**
 * 人工裁决的**命中判定**（上游 `Meta/backend/app-agent/src/dialog_tools/transfer_ontology.rs`
 * `decide_verdict`，change `wire-mapping-verdict-recall-into-transfer` D2）。
 *
 * 纯函数、零 IO：判定「本 ns 里已有的裁决能否直接用于这个需求域」。逐条照搬上游判据：
 * - 域 id **精确**匹配（不做模糊/相似度——裁决是人的决定，模糊匹配等于替人改判）；
 * - 置信度 ≥ {@link ACCEPT_SCORE}（低置信进 `ignored`，带原因，不静默丢弃）；
 * - 目录表存在性：`catalog` 可用（任一侧非空）却查不到该表 ⇒ 不命中（防 stale）；目录为空 ⇒ **豁免**
 *   （上游同判：目录不可用时不能拿「查不到」当否决证据）。
 *
 * `ignored` 是**观测面**：调用方把「为什么没用上某条裁决」透给模型/人，而不是只给一个空结果。
 * @module @dsh-alioth/verify-alioth/mapping-decisions
 */

import type { MappingVerdict } from './mapping-verdicts.ts'

/** 上游 `state::PlatformCatalog.lifecycle_entities` 的编译期常量面（zc_id_bill/event/entity/status）。 */
export const LIFECYCLE_ENTITIES: readonly string[] = ['zc_id_bill', 'zc_id_event', 'zc_id_entity', 'zc_id_status']

/** 置信度门槛（上游 `ACCEPT_SCORE`：0.8，本仓人工裁决恒为 1.0）。 */
export const ACCEPT_SCORE = 0.8

/**
 * 一条未被采纳的裁决 + 可判原因（观测面）。
 * 声明为 type 且字段可变：该形状直接进工具输出（`JsonValue` 要求索引签名，interface/readonly 数组都不满足）。
 */
export type IgnoredVerdict = {
  domain: string
  table: string
  /** `低置信` | `表不存在`（上游同词）。 */
  reason: string
  confidence: number
}

/** 目录读取面：`null` = 目录不可用（豁免存在性判定）。 */
export interface PlatformCatalogView {
  /** `isahl_meta.meta_collections` 的表名集。 */
  readonly collections: readonly string[]
  /** 编译期生命周期实体（`zc_id_bill`/`event`/`entity`/`status`）。 */
  readonly lifecycleEntities: readonly string[]
}

export interface MappingDecision {
  readonly table: string
  readonly confidence: number
  /** 命中的那条裁决（供取证）。 */
  readonly verdict: MappingVerdict
}

export interface MappingDecisionResult {
  /** 命中则给出表与置信度；`null` = 无可用先例（调用方继续走 discovery）。 */
  readonly decision: MappingDecision | null
  /** 未被采纳的先例及原因（空集 = 该域本来就没有先例）。 */
  ignored: IgnoredVerdict[]
}

/**
 * Judge whether a recorded verdict decides this domain.
 * @param domain - requirement domain id to decide.
 * @param verdicts - this namespace's verdicts, newest first (as `recall` returns them).
 * @param catalog - platform catalog view, or `null` when the catalog cannot be read.
 * @returns the decision (if any) plus the ignored precedents with reasons.
 */
export function decideMappingVerdict(
  domain: string,
  verdicts: readonly MappingVerdict[],
  catalog: PlatformCatalogView | null,
): MappingDecisionResult {
  const ignored: IgnoredVerdict[] = []
  const hit = verdicts.find(verdict => verdict.domain === domain)
  if (hit === undefined) {
    return { decision: null, ignored }
  }
  if (hit.confidence < ACCEPT_SCORE) {
    ignored.push({ domain, table: hit.table, reason: '低置信', confidence: hit.confidence })
    return { decision: null, ignored }
  }
  const catalogReady = catalog !== null
    && (catalog.collections.length > 0 || catalog.lifecycleEntities.length > 0)
  const tableExists = catalog !== null
    && (catalog.collections.includes(hit.table) || catalog.lifecycleEntities.includes(hit.table))
  if (catalogReady && !tableExists) {
    ignored.push({ domain, table: hit.table, reason: '表不存在', confidence: hit.confidence })
    return { decision: null, ignored }
  }
  return { decision: { table: hit.table, confidence: hit.confidence, verdict: hit }, ignored }
}
