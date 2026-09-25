/**
 * `alioth_mapping_verdict` — 人工映射裁决（需求域 → isahl 表）的沉降与召回。
 *
 * 上游对照：`Meta/backend/app-agent/src/dialog_tools/record_verdict.rs`
 * （change `fix-appagent-knowledge-loop-wiring` design D4）。语义逐条照搬：
 * - `keep_gap=true` → **错误**（「保持缺口」不沉降：上游返回 err 而非 ok，防止 LLM 把拒绝当成功继续）；
 * - `table` 不在平台目录（`isahl_meta.meta_collections` ∪ 编译期生命周期实体）→ 拒绝（防 stale）；
 * - 目录不可判定（注册表不可达 / `meta_collections` 为空）→ **fail-closed 拒绝**——写径宁缺勿污染，
 *   且 MUST NOT 拿编译期常量列表当「目录可用」的证据（否则 DB 不可达时该判据恒真、fail-closed 分支永不生效）；
 * - 合法 → 沉降，来源固定人工档（`source=user`、`confidence=1.0`）。
 *
 * 召回（本仓补充面）：`action: 'recall'` 返回该命名空间下最新在前的裁决，可按 domain/table 收窄——
 * 上游由 memory 子系统在装配上下文时召回，本仓先把它做成显式读取面。
 * 存储见 `@dsh-alioth/verify-alioth` 的 `createMappingVerdictStore`（部署自有文件账本，不往注册表 schema 加表）。
 * @module @dsh-alioth/tool-alioth-verify/verdict-tool
 */

import { defineTool } from '@deepseek-ai/dsh-tools'
import type { Context } from '@deepseek-ai/cordis'
import type { MappingVerdict, MappingVerdictStore } from '@dsh-alioth/verify-alioth'

/** 上游 `state::PlatformCatalog.lifecycle_entities` 的编译期常量面（zc_id_bill/event/entity/status）。 */
const LIFECYCLE_ENTITIES: readonly string[] = ['zc_id_bill', 'zc_id_event', 'zc_id_entity', 'zc_id_status']

/** 注册表读取面（`ctx.aliothEnv.sql` 满足）。 */
export interface RegistryQuery {
  (text: string, values?: readonly unknown[]): Promise<{ readonly rows: readonly unknown[] }>
}

/** 平台目录判定结果：可用与否，以及目标表是否在目录内。 */
interface CatalogVerdict {
  readonly usable: boolean
  readonly hasTable: boolean
}

/**
 * 读平台目录判定 `table` 是否存在。
 * @param sql - registry query face.
 * @param table - target table name.
 * @returns `usable=false` when the catalog cannot be judged at all (fail-closed caller side).
 */
async function judgeCatalog(sql: RegistryQuery, table: string): Promise<CatalogVerdict> {
  const counted = await sql('SELECT count(*)::int AS n FROM isahl_meta.meta_collections')
  const n = (counted.rows[0] as { n?: number } | undefined)?.n ?? 0
  if (n === 0) {
    return { usable: false, hasTable: false }
  }
  const hit = await sql('SELECT 1 FROM isahl_meta.meta_collections WHERE table_name = $1 LIMIT 1', [table])
  return { usable: true, hasTable: hit.rows.length > 0 || LIFECYCLE_ENTITIES.includes(table) }
}

/** `output.schema` 的取值面是 JsonValue——显式摊平（不靠结构巧合）后才交给工具运行时。 */
function toJson(verdict: MappingVerdict): Record<string, string | number> {
  return {
    namespace: verdict.namespace,
    domain: verdict.domain,
    table: verdict.table,
    verdictText: verdict.verdictText,
    source: verdict.source,
    confidence: verdict.confidence,
    recordedAt: verdict.recordedAt,
  }
}

export function registerMappingVerdictTool(
  ctx: Context,
  options: { readonly store: MappingVerdictStore; readonly sql: RegistryQuery },
): void {
  ctx.tools.register(defineTool({
    name: 'alioth_mapping_verdict',
    description:
      'Record or recall a HUMAN mapping verdict (requirement domain → isahl table) in the knowledge '
      + 'ledger — the harness mirror of the upstream `record_mapping_verdict` sink. Record (default): '
      + '`keep_gap: true` means "keep the gap open" and is REFUSED (nothing is sedimented, the call '
      + 'fails); a `table` outside the platform catalog is refused (a stale verdict would poison later '
      + 'recall); when the registry cannot be read the call fails closed (never writes an unverifiable '
      + 'verdict). Recall: returns this namespace\'s verdicts, newest first, optionally narrowed by '
      + '`domain` / `table`.',
    parameters: {
      namespace: {
        type: 'string',
        required: true,
        description: 'The caller\'s own workspace namespace — resolve with alioth_workspace_current first.',
      },
      action: {
        type: 'string',
        description: '"record" (default) or "recall".',
      },
      domain: {
        type: 'string',
        description: 'Requirement domain id (ontology model domain, e.g. material_plan). Required for record; optional filter for recall.',
      },
      table: {
        type: 'string',
        description: 'Target isahl table name. Required for record (must be in the platform catalog); optional filter for recall.',
      },
      verdict_text: {
        type: 'string',
        description: 'The operator\'s verdict text (record only).',
      },
      keep_gap: {
        type: 'boolean',
        description: 'record only: true = keep the gap open — refused by design, nothing is stored.',
      },
    },
    output: {
      schema: {
        type: 'object',
        additionalProperties: false,
        properties: {
          action: { type: 'string', required: true },
          namespace: { type: 'string' },
          count: { type: 'number' },
          verdict: { type: 'json' },
          verdicts: { type: 'json' },
        },
      },
      render: (_args, value) => [{
        type: 'text',
        text: value.action === 'recall'
          ? `${String(value.count ?? 0)} mapping verdict(s) (${String(value.namespace)})`
          : `mapping verdict recorded: ${String((value.verdict as { domain?: unknown } | undefined)?.domain ?? '')} → ${String((value.verdict as { table?: unknown } | undefined)?.table ?? '')}`,
      }],
    },
    async execute(rawArgs) {
      const args = rawArgs as Record<string, unknown>
      const namespace = String(args.namespace)
      const action = typeof args.action === 'string' && args.action.length > 0 ? args.action : 'record'
      const domain = typeof args.domain === 'string' ? args.domain : undefined
      const table = typeof args.table === 'string' ? args.table : undefined

      if (action === 'recall') {
        const verdicts = await options.store.recall({ namespace, ...(domain === undefined ? {} : { domain }), ...(table === undefined ? {} : { table }) })
        return { action, namespace, count: verdicts.length, verdicts: verdicts.map(toJson) }
      }
      if (action !== 'record') {
        throw new Error(`alioth_mapping_verdict: unknown action ${JSON.stringify(action)} (expected record | recall)`)
      }
      if (domain === undefined || table === undefined) {
        throw new Error('alioth_mapping_verdict: record needs both `domain` and `table`')
      }
      if (args.keep_gap === true) {
        throw new Error(
          'alioth_mapping_verdict: keep_gap=true means "keep the gap open" — nothing is sedimented '
          + '(the upstream sink returns an error for exactly this call so a refusal is never mistaken for success).',
        )
      }

      let catalogue: CatalogVerdict
      try {
        catalogue = await judgeCatalog(options.sql, table)
      } catch (error) {
        // 写径宁缺勿污染：目录读不到就不写（上游 `catalog_usable` 同理）。
        throw new Error(
          'alioth_mapping_verdict: platform catalog unavailable (registry unreachable) — refusing to '
          + `sediment an unverifiable verdict: ${error instanceof Error ? error.message : String(error)}`,
        )
      }
      if (!catalogue.usable) {
        throw new Error(
          'alioth_mapping_verdict: platform catalog unusable (`isahl_meta.meta_collections` is empty) — '
          + 'refusing to sediment a verdict; the compile-time lifecycle list is NOT evidence of a usable catalog.',
        )
      }
      if (!catalogue.hasTable) {
        throw new Error(
          `alioth_mapping_verdict: table ${JSON.stringify(table)} is not in the platform catalog — refused `
          + '(anti-stale: a verdict pointing at a retired table would poison later recall).',
        )
      }

      const verdict = await options.store.record({
        namespace,
        domain,
        table,
        verdictText: typeof args.verdict_text === 'string' ? args.verdict_text : '',
      })
      return { action, namespace, verdict: toJson(verdict) }
    },
  }))
}
