/**
 * 人工裁决先例的查询面（上游 `analyze_ontology.rs::recall_verdicts` + `transfer_ontology.rs::decide_verdict`
 * 的合并体）：在**做映射决定之前**把本命名空间已有的人工/evo 先例取出来，并判定它是否可直接采用。
 *
 * 上游口径逐条照搬：
 * - 存储不可达 ⇒ warn + 空集，**MUST NOT 阻断**分析（先例是加速面，不是判据来源）；
 * - 目录读取失败 ⇒ collections 记空、`catalog: 'unavailable'` 如实透出（上游同样只带编译期常量面，
 *   因此「表不存在」在目录不可用时仍可能命中——这是上游既有行为，照搬不修饰）；
 * - 命中/未命中都带**可判原因**（`ignored`），不把「没用上」做成静默。
 * @module @dsh-alioth/tool-alioth-meta/precedents
 */

import {
  LIFECYCLE_ENTITIES,
  createMappingVerdictStore,
  decideMappingVerdict,
  type IgnoredVerdict,
} from '@dsh-alioth/verify-alioth'

/** 查询先例所需的 env 面（`ctx.aliothEnv` 满足）。 */
export interface PrecedentEnv {
  dataRoot(): string
  sql(text: string, values?: readonly unknown[]): Promise<{ readonly rows: readonly unknown[] }>
}

export interface PrecedentLookup {
  readonly namespace: string
  /** 需求域 id（本体模型 domain id）。 */
  readonly domain: string
}

/** 该形状直接进工具输出，故用 type + 可变字段（`JsonValue` 要求索引签名）。 */
export type PrecedentResult = {
  /** `used` = 目录读到了；`unavailable` = 读不到（存在性判据退化为编译期常量面）。 */
  catalog: 'used' | 'unavailable'
  /** 本域的先例条数（含被判不采用的）。 */
  precedents: number
  /** 可直接采用的先例（人工决定）；`null` = 本域没有可用先例，继续 discovery。 */
  verdict: { table: string; confidence: number; verdict_text: string } | null
  /** 未采用的先例及原因（`低置信` / `表不存在`）。 */
  ignored: IgnoredVerdict[]
}

/**
 * Look up the human precedents recorded for one requirement domain.
 * @param env - deployment env (data root + registry query face).
 * @param lookup - namespace + domain id.
 */
export async function resolvePrecedents(env: PrecedentEnv, lookup: PrecedentLookup): Promise<PrecedentResult> {
  const verdicts = await createMappingVerdictStore(env.dataRoot())
    .recall({ namespace: lookup.namespace, domain: lookup.domain })
    .catch(() => []) // 先例账本不可读 ⇒ 空集（上游：warn + 空，不阻断）
  let collections: string[] = []
  let catalog: 'used' | 'unavailable' = 'used'
  try {
    const result = await env.sql('SELECT table_name FROM isahl_meta.meta_collections')
    collections = result.rows
      .map(row => (row as { table_name?: unknown }).table_name)
      .filter((name): name is string => typeof name === 'string')
  } catch {
    catalog = 'unavailable'
  }
  const { decision, ignored } = decideMappingVerdict(lookup.domain, verdicts, {
    collections,
    lifecycleEntities: LIFECYCLE_ENTITIES,
  })
  return {
    catalog,
    precedents: verdicts.length,
    verdict: decision === null
      ? null
      : { table: decision.table, confidence: decision.confidence, verdict_text: decision.verdict.verdictText },
    ignored,
  }
}
