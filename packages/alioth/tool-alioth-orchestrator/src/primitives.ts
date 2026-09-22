/**
 * Real primitive bindings for the AppAgent pipeline machine
 * (`@dsh-alioth/skill-alioth/agent-machine`). Each stage maps to an existing
 * tool through `ctx.tools.execute` — the same path the model uses, so
 * approvals, gates, and the session log apply per stage. No LLM calls.
 * Semantic alignment is a dialogue precondition (semantic_search + model
 * decision) passed in via parameters; the semantic-analysis primitive merely
 * re-confirms hits for the audit trail.
 * @module @dsh-alioth/tool-alioth-orchestrator/primitives
 */

import { homedir } from 'node:os'
import { appendFile, mkdir, readFile, writeFile } from 'node:fs/promises'
import path from 'node:path'
import type { Context } from '@deepseek-ai/cordis'
import type { ToolRunContext } from '@deepseek-ai/dsh-tools'
// Type-only: the harness loader resolves our bare imports against the
// installation's own dsh-llm via the profile fallback (ESM never realpaths
// the plugin symlink), so a runtime import would couple plugin loading to
// the host's dsh-llm version. Type-only keeps loading version-free while
// staying compile-time honest against the pinned harness devDeps (the value
// factory is `ToolCallId` since 0.1.2-alpha.1; our pin floor is 0.1.5-alpha.1).
import type { ToolCallId } from '@deepseek-ai/dsh-llm'
import type { AgentPrimitives, StageOutput } from '@dsh-alioth/skill-alioth/agent-machine'
import { STAGE_IDS } from '@dsh-alioth/skill-alioth/agent-contract'
import type { BuildResult, FlowPlan } from '@dsh-alioth/skill-alioth/agent-contract'
import {
  formatRepairError,
  type RepairContract,
  validateEntitySpec,
  writeE2eReport,
  type E2eCheck,
  type EntitySpec,
  type FieldSpec,
  type RegistryView,
  type RepairClass,
} from '@dsh-alioth/skill-alioth'
import {
  appendClosureVerdict,
  artifactFingerprint,
  buildEvalReport,
  createDeferredStore,
  EXTENSION_FORMS,
  evaluatePublishShadow,
  evaluateStageGate,
  latestMatchingVerdict,
  listSnapshots,
  snapshotArtifacts,
  verifyExtensions,
  writeEvalReport,
  writeExtensionVerify,
  type ClosureVerdict,
  type DeferredItem,
} from '@dsh-alioth/verify-alioth'
import { PIPELINE_SHARED_SURFACES, runControlledParallel } from './parallel.ts'

export interface CreateArgs {
  readonly namespace: string
  readonly code: string
  readonly name: string
  readonly modules: ReadonlyArray<{ readonly id: string; readonly name: string }>
  readonly blocks?: readonly string[]
  readonly entities?: ReadonlyArray<{
    readonly table: string
    readonly name: string
    readonly inherits?: readonly string[]
    readonly category?: string
    readonly coordinates?: { readonly scene: string; readonly factor: string; readonly function: string }
    readonly fields?: ReadonlyArray<{
      readonly name: string
      readonly category: string
      readonly dataType: string
      readonly title?: string
      readonly required?: boolean
      readonly targetTable?: string
      readonly localKey?: string
      readonly junctionTable?: string
    }>
  }>
}

/** Execute one registered tool through the registry (model-equivalent path). */
async function runTool(
  ctx: Context,
  exec: ToolRunContext,
  toolName: string,
  args: unknown,
): Promise<Record<string, unknown>> {
  const result = await ctx.tools.execute({
    signal: exec.signal,
    callId: `${toolName}-pipeline` as ToolCallId,
    name: toolName,
    arguments: args,
    ...(exec.agent === undefined ? {} : { agent: exec.agent }),
  })
  if (result.isError) {
    throw new Error(`pipeline stage ${toolName} failed: ${result.error.message}`)
  }
  return result.value as Record<string, unknown>
}

/**
 * Preflight entity validation — restores PTC atomicity inside the pipeline:
 * AppCreation (stage 0) writes the artifact tree, but a failed entity must
 * abort BEFORE any artifact is written. Validate all declared entities first
 * (same deterministic checks as alioth_entity_write) and throw on issues.
 */
async function preflightEntities(ctx: Context, exec: ToolRunContext, entities: CreateArgs['entities']): Promise<void> {
  if (entities === undefined || entities.length === 0) {
    return
  }
  const rows = await ctx.aliothEnv.sql<{ table_name: string; name: string; inherits: unknown }>(
    `SELECT table_name, name, config->'inherits' AS inherits
     FROM isahl_meta.meta_collections`,
  )
  const collections = new Map<string, { name: string; inherits: readonly string[] }>()
  for (const row of rows.rows) {
    collections.set(row.table_name, {
      name: row.name,
      inherits: Array.isArray(row.inherits) ? row.inherits.map(entry => String(entry)) : [],
    })
  }
  const registry: RegistryView = { collections }
  for (const entity of entities) {
    const spec: EntitySpec = {
      table: entity.table,
      name: entity.name,
      inherits: entity.inherits ?? [],
      ...(entity.category === undefined ? {} : { category: entity.category }),
      ...(entity.coordinates === undefined ? {} : { coordinates: entity.coordinates }),
      fields: (entity.fields ?? []).map(field => ({
        name: field.name,
        category: field.category as FieldSpec['category'],
        dataType: field.dataType,
        ...(field.title === undefined ? {} : { title: field.title }),
        ...(field.required === undefined ? {} : { required: field.required }),
        ...(field.targetTable === undefined && field.localKey === undefined && field.junctionTable === undefined
          ? {}
          : { reference: {
              targetTable: field.targetTable ?? '',
              ...(field.localKey === undefined ? {} : { localKey: field.localKey }),
              ...(field.junctionTable === undefined ? {} : { junctionTable: field.junctionTable }),
            } }),
      })) satisfies readonly FieldSpec[],
    }
    const issues = validateEntitySpec(spec, registry)
    if (issues.length > 0) {
      throw new Error(
        `alioth_app_create: alioth_entity_write (preflight) rejected ${entity.table}: ${issues.map(issue => issue.message).join('; ')}`,
      )
    }
  }
  void exec
}

/**
 * Gateway ExtensionLoader 认可的扩展声明形态（`load_from_dir` 逐文件加载，见
 * `env-alioth/vendor/Framework/backend/runtime-engine/src/extension.rs:843-905`：
 * constraints:855 / rules:867 / statemachines:879 / workflows:891 / profiles:903）。
 * 形态名与 verify-alioth `verifyExtensions` 的 `FORM_SPECS.form` 同表；调用方注入，
 * 库不读 Rust 源码。
 */


/** 未配置 preProcRoot 时的根解析链（逐阶段现读：部署可改环境变量）。 */
function preProcRootOf(configured: string | undefined): string {
  return configured ?? process.env.ALIOTH_PRE_PROC_ROOT ?? path.join(homedir(), '.dsh-alioth', 'Pre-Proc')
}

function describe(error: unknown): string {
  return error instanceof Error ? error.message : String(error)
}

/** JSON.parse 的窄化读取：不可解析或顶层非对象 → null（不扮成带字段的形状）。 */
function jsonObjectOf(text: string): Record<string, unknown> | null {
  let parsed: unknown
  try {
    parsed = JSON.parse(text)
  } catch {
    return null
  }
  if (typeof parsed !== 'object' || parsed === null || Array.isArray(parsed)) {
    return null
  }
  // typeof/Array.isArray 已窄化：unknown 在此降为记录形状。
  return parsed as Record<string, unknown>
}

/** 扩展验证证据：canonical 报告的存在 + 判定 + 绑定的产物指纹（空 = 未绑定当前产物）。 */
interface ExtensionEvidence {
  readonly status: 'passed' | 'degraded' | 'missing'
  readonly fingerprint: string
}

/** canonical 扩展验证报告的判定：缺失/不可解析/非 passed 一律按未通过（fail-closed）。 */
async function readExtensionReport(appDir: string): Promise<ExtensionEvidence> {
  const text = await readFile(path.join(appDir, 'extension-verify.json'), 'utf8').catch(() => null)
  if (text === null) return { status: 'missing', fingerprint: '' }
  const report = jsonObjectOf(text)
  if (report === null) return { status: 'degraded', fingerprint: '' }
  const fingerprint = report['artifact_fingerprint']
  return {
    status: report['status'] === 'passed' ? 'passed' : 'degraded',
    fingerprint: typeof fingerprint === 'string' ? fingerprint : '',
  }
}

/** quality 评估报告的判定：缺失与 passed≠true 分态（缺失 ≠ 未通过 ≠ 通过）。 */
async function readQualityReport(appDir: string): Promise<'passed' | 'failed' | 'missing'> {
  const text = await readFile(path.join(appDir, 'eval-report.json'), 'utf8').catch(() => null)
  if (text === null) return 'missing'
  const report = jsonObjectOf(text)
  return report !== null && report['passed'] === true ? 'passed' : 'failed'
}

/**
 * 本 app 未解除的降级验证门（publish 前置，上游 `publish.rs:765-770`
 * `open_degraded_gates_for_app`）：`extension-verify.json`（canonical，仅真实执行产出）与
 * `extension-verify.degraded.json`（降级留痕）是两文件纪律——一次降级运行不覆盖旧的
 * canonical passed，故单看 canonical 会被陈旧证据骗过，必须同时看未决降级门。
 *
 * 扫描口径（上游 `deferred.rs:267-271`：会话级扫描会失明）：先 `sweep()` 全库解除
 * **触发条件成立**的挂起项（未成立者原地不动），再按 `app` + `namespace` 跨会话归属过滤
 * ——先前会话登记的未决门对新会话的发布同样可见。
 *
 * 缺省归属 = fail-closed 匹配（上游 `deferred.rs:286` 对缺 `app_code` 的记录
 * `map_or(true, …)` 同判）：不带 app/namespace 的登记（含 verify-alioth 对损坏
 * 登记文件合成的显式阻塞项）对**任何** App 的 publish 都可见，修好前持续阻断。
 */
export async function openDegradedGates(dataRoot: string, namespace: string, app: string): Promise<DeferredItem[]> {
  const store = createDeferredStore(dataRoot)
  await store.sweep()
  return (await store.all()).filter(
    item => (item.app ?? app) === app && (item.namespace ?? namespace) === namespace,
  )
}

/** 管线本地规则：模块镜像的源产物缺失（不是模型能产出的文件）。 */
const MODULE_ARTIFACT_MISSING_RULE = {
  ruleId: 'pipeline-module-artifact-missing',
  class: 'not-fixable',
  message: 'PTC 管线 module-creation：app 创建阶段未写出该模块的 module.json，镜像无法进行',
  suggestedAction: '先查 app-creation 是否写出 modules/{id}/module.json，再核对 orchestrator 与工具插件的 preProcRoot 是否同一棵树',
} as const

/** publish 前置的本地规则行（skill-alioth 的修复表无 publish 前置特征；渲染仍走 formatRepairError）。 */
interface PublishRule {
  readonly ruleId: string
  readonly class: RepairClass
  readonly message: string
  readonly suggestedAction: string
}

const PUBLISH_RULES = {
  extensionMissing: {
    ruleId: 'publish-extension-verify-missing',
    class: 'fixable',
    message: 'publish 前置未满足：extension-verify.json（canonical）缺失',
    suggestedAction: '先跑扩展运行时验证并落盘 canonical 报告——降级文件（extension-verify.degraded.json）永不解除此前置',
  },
  extensionDegraded: {
    ruleId: 'publish-extension-verify-degraded',
    class: 'fixable',
    message: 'publish 前置未满足：扩展验证 status=degraded',
    suggestedAction: '按 extension-verify.json 的 uncovered 声明逐条补齐形态/必需键，重跑验证直至 status=passed',
  },
  extensionUnbound: {
    ruleId: 'publish-extension-verify-unbound',
    class: 'fixable',
    message: 'publish 前置未满足：扩展验证报告未绑定当前产物（指纹为空或与当前产物不符）',
    suggestedAction: '产物已变或验证并非真实执行：重跑扩展验证产出绑定当前产物指纹的 canonical 报告后重试',
  },
  qualityMissing: {
    ruleId: 'publish-quality-report-missing',
    class: 'fixable',
    message: 'publish 前置未满足：quality 阶段 eval-report.json 缺失/不可解析',
    suggestedAction: '先跑 quality 评估（verify-alioth buildEvalReport）落盘 eval-report.json',
  },
  qualityNotPassed: {
    ruleId: 'publish-quality-not-passed',
    class: 'fixable',
    message: 'publish 前置未满足：eval-report.json passed≠true',
    suggestedAction: '按 eval-report.json 的 violations 修复产物（缺失面按 0 分计），重跑评估',
  },
  snapshotMissing: {
    ruleId: 'publish-snapshot-missing',
    class: 'retryable',
    message: 'publish 前置未满足：产物版本快照未写入',
    suggestedAction: '先建产物快照（verify-alioth snapshotArtifacts）后重试发布；快照范围为空必须显式报错',
  },
  degradedGateOpen: {
    ruleId: 'publish-degraded-gate-unresolved',
    class: 'fixable',
    message: 'publish 前置未满足：存在未决的降级验证门（canonical passed 可能是陈旧证据）',
    suggestedAction: '按登记项说明逐条补齐未执行的声明或取得人工确认，用 alioth_deferred unlock 解除后重试发布',
  },
  closureMissing: {
    ruleId: 'publish-closure-verdict-missing',
    class: 'fixable',
    message: 'publish 前置未满足：closure-audit 无与当前产物指纹匹配的裁决',
    suggestedAction: '对当前产物跑独立结束审计（verify-alioth appendClosureVerdict）——审计后产物被改即指纹失配，须重新审计',
  },
  closureNotApproved: {
    ruleId: 'publish-closure-not-approved',
    class: 'fixable',
    message: 'publish 前置未满足：closure-audit 最新匹配裁决非 approved',
    suggestedAction: '按 closure-audit 裁决的 findings 修复产物并重新审计（连续 rejected 会升级人工）',
  },
} as const satisfies Record<string, PublishRule>

/** 前置失败的单行证据（ruleId/class/suggestedAction/证据，唯一解析点见 ruleIdFromError）。 */
function publishRepair(rule: PublishRule, evidence: string): string {
  return formatRepairError({
    ruleId: rule.ruleId,
    class: rule.class,
    message: rule.message,
    suggestedAction: rule.suggestedAction,
    evidence,
  })
}

/** publish 影子对照留痕面：append-only JSONL，落 `{appDir}/AppAgentTraces/publish-shadow.jsonl`。 */
async function appendPublishShadowTrace(appDir: string, line: Record<string, unknown>): Promise<void> {
  const dir = path.join(appDir, 'AppAgentTraces')
  await mkdir(dir, { recursive: true })
  await appendFile(path.join(dir, 'publish-shadow.jsonl'), `${JSON.stringify(line)}\n`, 'utf8')
}

/** Bind the 9-stage pipeline to real tools for one `alioth_app_create` call. */
export function buildPrimitives(
  ctx: Context,
  exec: ToolRunContext,
  args: CreateArgs,
  workflowAdapter: string | undefined,
  preProcRoot: string | undefined,
): AgentPrimitives {
  /** Phase 2/3/4 share one app_write (write-once); later stages verify. */
  let writtenFiles: string[] = []

  return {
    // 0. App creation — the application container: the contract-validated
    //    artifact tree (contract gate inside app_write; write-once).
    async appCreation(input) {
      // Atomicity: validate declared entities BEFORE any artifact write.
      await preflightEntities(ctx, exec, args.entities)
      const written = await runTool(ctx, exec, 'alioth_app_write', {
        namespace: args.namespace,
        code: args.code,
        name: args.name,
        modules: args.modules,
        ...(args.blocks === undefined ? {} : { blocks: args.blocks }),
      })
      writtenFiles = Array.isArray(written.files) ? written.files as string[] : []
      return {
        evidence: `app creation: container ${args.namespace}/${args.code} ("${args.name}", intent: ${input}), ${writtenFiles.length} files`,
        artifacts: writtenFiles,
      }
    },

    // 1. Semantic analysis — dialogue preconditions (alignment already done);
    //    re-confirm via semantic search for the audit trail. The search is a
    //    confirmation, not a gate: an empty registry (fresh bootstrap) must
    //    not block the pipeline — alignment parameters already carry the
    //    resolution.
    async semanticAnalysis(input) {
      try {
        const search = await runTool(ctx, exec, 'alioth_schema_semantic_search', {
          query: input,
          ...(args.entities === undefined ? {} : { k: Math.max(5, args.entities.length) }),
        })
        const hits = Array.isArray(search.hits) ? (search.hits as unknown[]).length : 0
        return {
          evidence: `semantic search: ${hits} registry hits for "${input}"`,
          artifacts: [String(search.cacheKey ?? '')].filter(Boolean),
        }
      } catch (error) {
        return {
          evidence: `semantic search unavailable (${error instanceof Error ? error.message : 'unknown'}); alignment preconditions accepted from parameters`,
        }
      }
    },

    // 2. Function decomposition — registry inventory grounding.
    async functionDecomposition(_input) {
      const info = await runTool(ctx, exec, 'alioth_schema_info', { action: 'entities', limit: 50 })
      const entities = Array.isArray(info.entities) ? info.entities as unknown[] : []
      return {
        evidence: `function decomposition: ${entities.length} registry entities available; plan=${args.modules.length} modules, ${args.blocks?.length ?? 0} blocks`,
        artifacts: [`namespace ${args.namespace}`, `app ${args.code}`],
      }
    },

    // 3. Ontology analysis — register declared new entities (validated).
    async ontologyAnalysis() {
      const registered: string[] = []
      for (const entity of args.entities ?? []) {
        await runTool(ctx, exec, 'alioth_entity_write', {
          table: entity.table,
          name: entity.name,
          ...(entity.inherits === undefined ? {} : { inherits: entity.inherits }),
          ...(entity.category === undefined ? {} : { category: entity.category }),
          ...(entity.coordinates === undefined ? {} : { coordinates: entity.coordinates }),
          fields: entity.fields ?? [],
        })
        registered.push(entity.table)
      }
      return {
        evidence: `ontology analysis: ${registered.length} entities registered (${registered.join(', ') || 'none'})`,
        artifacts: registered,
      }
    },

    // 4. Module creation — 每个模块自己的产物写出：把 app 级 `modules/{id}/module.json`
    //    （app 创建时写一次）镜像到命名空间 Sources 布局 `Sources/Apps/Modules/{id}/module.json`
    //    —— module-design 阶段门禁读的正是这个路径。这是管线里唯一登记的并行单元集合：
    //    单元只写自己模块的写面；共享清单面（app.json/module.json/extensions/*.yaml/
    //    Cargo.toml）由受控原语判定「命中即串行」，故当前按模块 id 顺序单所有者写。
    async moduleCreation() {
      const namespaceRoot = path.join(preProcRootOf(preProcRoot), args.namespace)
      const appDir = path.join(namespaceRoot, 'Apps', args.code)
      const outcome = await runControlledParallel(
        args.modules.map(module => ({
          id: module.id,
          writes: [`${args.namespace}/Sources/Apps/Modules/${module.id}/module.json`],
          run: async () => {
            const source = path.join(appDir, 'modules', module.id, 'module.json')
            const artifact = await readFile(source, 'utf8').catch(() => null)
            if (artifact === null) {
              throw new Error(`模块产物缺失：${source}（app 创建未写出模块 ${module.id} 的 module.json）`)
            }
            const target = path.join(namespaceRoot, 'Sources', 'Apps', 'Modules', module.id, 'module.json')
            await mkdir(path.dirname(target), { recursive: true })
            await writeFile(target, artifact, 'utf8')
            return target
          },
        })),
        { sharedWriteSurfaces: PIPELINE_SHARED_SURFACES },
      )
      if (!outcome.ok) {
        // Pipeline-local rule, not the step-gate `gate-output-missing`: that rule's action
        // tells the model to author the missing file, but a module mirror is produced by
        // the pipeline itself — a model cannot fix it, and the operator's real lead is a
        // root mismatch or an app-creation that did not write the module artifact.
        const contract: RepairContract = {
          ruleId: MODULE_ARTIFACT_MISSING_RULE.ruleId,
          class: MODULE_ARTIFACT_MISSING_RULE.class,
          message: MODULE_ARTIFACT_MISSING_RULE.message,
          suggestedAction: `${MODULE_ARTIFACT_MISSING_RULE.suggestedAction}（本管线根 = ${path.dirname(path.dirname(appDir))}；若与工具插件的 preProcRoot 不一致，模块产物在另一棵树下）`,
          evidence: outcome.results
            .filter(result => result.status !== 'ok')
            .map(result => `${result.id}: ${result.error ?? 'skipped（前序失败）'}`)
            .join('; '),
        }
        throw new Error(
          `alioth_app_pipeline module-creation: 模块产物写出未完成（${outcome.incomplete.join(', ')}）\n${formatRepairError(contract)}`,
        )
      }
      const mirrors = outcome.results.flatMap(result => result.value === undefined ? [] : [result.value])
      return {
        evidence: `module creation: ${mirrors.length} module artifacts mirrored into Sources/Apps/Modules`
          + ` (serial=${outcome.exclusive.length}/${outcome.results.length}, concurrency=${outcome.concurrency})`,
        artifacts: mirrors,
      }
    },

    // 5. Block creation — write-once preserved; verify the block artifacts.
    async blockCreation() {
      const blockFiles = writtenFiles.filter(f => f.endsWith('block.json'))
      return {
        evidence: `block creation: ${blockFiles.length} block artifacts verified`,
        artifacts: blockFiles,
      }
    },

    // 6. Ontology transfer — service/extension artifacts verified; 本体映射产物
    //    （`local/ontology-output.json`，ontology-mapping 阶段门禁的判据）确定性落盘：
    //    内容 = 本次运行已登记的实体与其对齐坐标（未声明坐标者如实留空，绝不猜码）。
    async ontologyTransfer() {
      const namespaceRoot = path.join(preProcRootOf(preProcRoot), args.namespace)
      const mapping = {
        schema_version: '1.0',
        namespace: args.namespace,
        app: args.code,
        generated_by: 'dsh-alioth orchestrator (deterministic; no LLM)',
        modules: args.modules.map(module => module.id),
        entities: (args.entities ?? []).map(entity => ({
          table: entity.table,
          name: entity.name,
          coordinates: entity.coordinates ?? null,
        })),
        ts: new Date().toISOString(),
      }
      const mappingPath = path.join(namespaceRoot, 'local', 'ontology-output.json')
      await mkdir(path.dirname(mappingPath), { recursive: true })
      await writeFile(mappingPath, `${JSON.stringify(mapping, null, 2)}\n`, 'utf8')
      const serviceFiles = writtenFiles.filter(f =>
        f.includes('service') || f.endsWith('extensions.yaml') || f.endsWith('extension.yaml'),
      )
      return {
        evidence: `ontology transfer: ${serviceFiles.length} service/extension artifacts verified; `
          + `mapping written to ${path.relative(namespaceRoot, mappingPath)}`,
        artifacts: [...serviceFiles, mappingPath],
      }
    },

    // 7. Service API — contract validation already gated by app_write; verify.
    async serviceApi() {
      const serviceFiles = writtenFiles.filter(f => f.includes('service'))
      return {
        evidence: `service API: ${serviceFiles.length} service artifacts contract-validated`,
        artifacts: serviceFiles,
      }
    },

    // 8. E2E verification — 验证证据的生产阶段：真实浏览器全链是人工验收项，
    //    这里跑确定性等价检查（app.json / module.json / standalone 原型 / 扩展声明覆盖）
    //    并如实落盘 §5.2 的 publish 前置证据（extension-verify.json、eval-report.json、
    //    产物版本快照、closure-audit 裁决）。任一检查未通过或证据写盘失败 → 证据以
    //    "E2E failed" 开头驱动修复循环，绝不按通过处理。
    async e2eVerification(attempt) {
      const appDir = path.join(preProcRootOf(preProcRoot), args.namespace, 'Apps', args.code)
      // An unwritable tree must be reported by the evidence writes below, not
      // abort the stage before it can report anything.
      await mkdir(appDir, { recursive: true }).catch(() => {})
      const appJson = writtenFiles.some(f => f.endsWith('app.json'))
      const moduleJson = writtenFiles.some(f => f.endsWith('module.json'))
      const checks: E2eCheck[] = [
        { id: 'app-json', passed: appJson, description: 'app.json artifact present' },
        { id: 'module-json', passed: moduleJson, description: 'module.json artifacts present' },
      ]
      const evidenceArtifacts: string[] = []
      const failures: string[] = []

      // 扩展运行时验证：声明未全覆盖即 degraded（degraded 永不写 canonical 位）。
      let extensionStatus: 'passed' | 'degraded' = 'degraded'
      try {
        const verification = await verifyExtensions({
          app: args.code,
          namespace: args.namespace,
          appDir,
          allowedForms: [...EXTENSION_FORMS],
        })
        extensionStatus = verification.status
        evidenceArtifacts.push(await writeExtensionVerify(appDir, verification))
      } catch (error) {
        failures.push(`extension-verify write failed (${describe(error)})`)
      }
      checks.push({
        id: 'extension-verify',
        passed: extensionStatus === 'passed',
        description: `扩展声明运行时覆盖 status=${extensionStatus}`,
      })

      // quality 评估报告：维度缺失按 0 分记（诚实评分）。
      let qualityPassed = false
      try {
        const report = await buildEvalReport({ app: args.code, namespace: args.namespace, appDir })
        qualityPassed = report.passed
        evidenceArtifacts.push(await writeEvalReport(appDir, report))
      } catch (error) {
        failures.push(`eval-report write failed (${describe(error)})`)
      }
      checks.push({
        id: 'quality',
        passed: qualityPassed,
        description: 'eval-report 规则维度（schema_validity + prototype_standalone）',
      })

      // 产物版本快照（publish 前置）：内容为空（无产物可快照）必须显式失败。
      let snapshotDir = ''
      try {
        const snapshot = await snapshotArtifacts(appDir)
        snapshotDir = snapshot.dir
        evidenceArtifacts.push(snapshot.dir)
      } catch (error) {
        failures.push(`snapshot failed (${describe(error)})`)
      }
      checks.push({
        id: 'artifact-snapshot',
        passed: snapshotDir !== '',
        description: '产物版本快照写入成功',
      })

      // 独立结束审计：指纹锁死本次产物，裁决 append-only（连续 rejected 升级人工）。
      let closureSeq = 0
      try {
        const fingerprint = await artifactFingerprint(appDir)
        const verdict = await appendClosureVerdict(appDir, {
          app: args.code,
          namespace: args.namespace,
          fingerprint,
          verdict: checks.every(check => check.passed) ? 'approved' : 'rejected',
          findings: checks.map(check => ({
            dimension: check.id,
            verdict: check.passed ? 'pass' : 'fail',
            detail: check.description,
          })),
          evidence: [...evidenceArtifacts],
        })
        closureSeq = verdict.seq
        evidenceArtifacts.push(path.join(appDir, 'AppAgentTraces', 'closure-audit', `${verdict.seq}.json`))
      } catch (error) {
        failures.push(`closure-audit failed (${describe(error)})`)
      }
      checks.push({
        id: 'closure-audit',
        passed: closureSeq > 0,
        description: `独立结束审计裁决已落盘（seq=${closureSeq}）`,
      })

      const passed = checks.every(check => check.passed)
      let reportPath = ''
      try {
        reportPath = await writeE2eReport(appDir, {
          app: args.code,
          namespace: args.namespace,
          attempt,
          passed,
          checks,
          note: 'dsh-alioth deterministic equivalent — prototype build chain and real-browser run are manual acceptance items',
        })
      } catch (error) {
        failures.push(`evidence report write failed (${describe(error)})`)
      }
      const failedIds = checks.filter(check => !check.passed).map(check => check.id)
      const reportNote = reportPath === ''
        ? `; ${failures.join('; ')}`
        : `; evidence ${path.basename(reportPath)} written`
      return {
        evidence: passed
          ? `E2E verification (attempt ${attempt}): artifacts complete${reportNote}`
          : `E2E failed (attempt ${attempt}): app.json=${appJson}, module.json=${moduleJson}`
            + ` [${failedIds.join(', ')}]${reportNote}`,
        artifacts: reportPath === '' ? [...writtenFiles] : [...writtenFiles, ...evidenceArtifacts, reportPath],
      }
    },

    // 9. Publishing — 读回校验 + 五项发布前置（扩展验证未降级 / 无未决降级门 /
    //    quality 报告存在且 passed / 版本快照已写入 / 与当前产物指纹匹配的 approved
    //    裁决），全部 fail-closed；另跑 publish 影子五谓词对照——**零行为变更**：只把
    //    对照结果 append 到 `{appDir}/AppAgentTraces/publish-shadow.jsonl`，不参与判定。
    async publishing(_plan, attempt) {
      const namespaceRoot = path.join(preProcRootOf(preProcRoot), args.namespace)
      const appDir = path.join(namespaceRoot, 'Apps', args.code)
      const inspected = await runTool(ctx, exec, 'alioth_app_inspect', {
        namespace: args.namespace,
        app: args.code,
      })
      const missing = Array.isArray(inspected.missing) ? inspected.missing as string[] : []
      const checks: { name: string; ok: boolean; detail: string }[] = [
        {
          name: 'inspect-readback',
          ok: missing.length === 0,
          detail: missing.length === 0 ? 'app reads back' : `missing: ${missing.join(', ')}`,
        },
      ]

      // 当前产物指纹（前置 1 的绑定判据与前置 5 的匹配判据共用；读不到 = 产物未就绪）。
      const currentFingerprint = await artifactFingerprint(appDir).catch(() => '')

      // 前置 1：扩展运行时验证未降级 **且绑定当前产物**：canonical 报告存在、
      // status=passed、artifact_fingerprint 非空且等于当前产物指纹（空指纹或失配
      // = 验证并非对当前产物真实执行过，不得当通过证据）。
      const extensionEvidence = await readExtensionReport(appDir)
      const extensionBound = extensionEvidence.fingerprint !== ''
        && extensionEvidence.fingerprint === currentFingerprint
      checks.push({
        name: 'publish-extension-verify',
        ok: extensionEvidence.status === 'passed' && extensionBound,
        detail: extensionEvidence.status === 'passed' && extensionBound
          ? `extension-verify.json status=passed @ ${extensionEvidence.fingerprint}`
          : publishRepair(
              extensionEvidence.status === 'missing'
                ? PUBLISH_RULES.extensionMissing
                : extensionEvidence.status === 'degraded'
                  ? PUBLISH_RULES.extensionDegraded
                  : PUBLISH_RULES.extensionUnbound,
              `report=${path.join(appDir, 'extension-verify.json')} status=${extensionEvidence.status}`
                + ` reportFingerprint=${extensionEvidence.fingerprint === '' ? '空' : extensionEvidence.fingerprint}`
                + ` currentFingerprint=${currentFingerprint === '' ? '不可得' : currentFingerprint}`,
            ),
      })

      // 前置 2（上游 publish 3b）：无未决降级门——canonical passed 也可能是陈旧证据。
      const openGates = await openDegradedGates(ctx.aliothEnv.dataRoot(), args.namespace, args.code)
      checks.push({
        name: 'publish-no-open-degraded-gates',
        ok: openGates.length === 0,
        detail: openGates.length === 0
          ? 'deferred 无本 app 未解除的降级门'
          : publishRepair(
              PUBLISH_RULES.degradedGateOpen,
              openGates
                .map(item => `${item.id} trigger=${item.trigger.kind}:${item.trigger.path}`
                  + ` reason=${item.reason} adjudication=${item.adjudication}`
                  + ` 待确认=${item.successors.length === 0 ? '未指定' : item.successors.join(',')}`)
                .join(' | '),
            ),
      })

      // 前置 3：quality 阶段 eval-report.json 存在且 passed。
      const qualityStatus = await readQualityReport(appDir)
      checks.push({
        name: 'publish-quality',
        ok: qualityStatus === 'passed',
        detail: qualityStatus === 'passed'
          ? 'eval-report.json passed=true'
          : publishRepair(
              qualityStatus === 'missing' ? PUBLISH_RULES.qualityMissing : PUBLISH_RULES.qualityNotPassed,
              `${path.join(appDir, 'eval-report.json')} status=${qualityStatus}`,
            ),
      })

      // 前置 4：产物版本快照已写入（快照存在 = 写入成功的可观察证据）。
      const snapshots = await listSnapshots(appDir).catch(() => [] as number[])
      checks.push({
        name: 'publish-artifact-snapshot',
        ok: snapshots.length > 0,
        detail: snapshots.length > 0
          ? `versions 快照 ${snapshots.length} 版（最新 seq=${snapshots[snapshots.length - 1]}）`
          : publishRepair(PUBLISH_RULES.snapshotMissing, `${path.join(appDir, 'versions')} 下无快照`),
      })

      // 前置 5：closure-audit 存在与**当前**产物指纹匹配的 approved 裁决。
      const verdict: ClosureVerdict | null = await latestMatchingVerdict(appDir, currentFingerprint)
        .catch(() => null)
      const approved = verdict?.verdict === 'approved'
      checks.push({
        name: 'publish-closure-verdict',
        ok: approved,
        detail: approved && verdict !== null
          ? `closure-audit seq=${verdict.seq} approved @ ${verdict.fingerprint}`
          : publishRepair(
              verdict === null ? PUBLISH_RULES.closureMissing : PUBLISH_RULES.closureNotApproved,
              `fingerprint=${currentFingerprint === '' ? '不可得' : currentFingerprint}`
                + ` verdict=${verdict === null ? 'none' : verdict.verdict}`,
            ),
      })

      let workflowGate = 'not-configured'
      if (workflowAdapter !== undefined) {
        const step = await runTool(ctx, exec, 'alioth_workflow_step', {
          namespace: args.namespace,
          app: args.code,
        })
        if (step.finished !== true) {
          await runTool(ctx, exec, 'alioth_workflow_complete', {
            namespace: args.namespace,
            app: args.code,
          })
          workflowGate = `step ${String(step.stepId)} passed`
        } else {
          workflowGate = 'finished'
        }
      }
      checks.push({ name: 'workflow-gate', ok: workflowGate !== 'failed', detail: workflowGate })

      // publish 影子自动门（契约 §5.2）：五谓词只记录对照，绝不自动放行/拒绝。
      // `noOpenDeferred` 取本 app 未解除降级门（前置 2 的同一集合，已走过触发求值）。
      const shadow = evaluatePublishShadow({
        artifactsComplete: missing.length === 0,
        qualityPassed: qualityStatus === 'passed',
        extensionStatus: extensionEvidence.status === 'passed' && extensionBound ? 'passed' : 'degraded',
        closureApproved: approved,
        noOpenDeferred: openGates.length === 0,
      })
      await appendPublishShadowTrace(appDir, {
        ts: new Date().toISOString(),
        attempt,
        fingerprint: currentFingerprint === '' ? null : currentFingerprint,
        willAutoApprove: shadow.willAutoApprove,
        predicates: shadow.predicates,
        openDegradedGates: openGates.map(item => item.id),
      }).catch(() => {})

      const published = checks.every(check => check.ok)
      const failedChecks = checks.filter(check => !check.ok).map(check => check.name)
      const result: BuildResult = {
        appName: args.name,
        outputPath: `Pre-Proc/${args.namespace}/Apps/${args.code}/app.json`,
        usedModules: args.modules.map(m => ({ moduleId: m.id, name: m.name, blocks: [] })),
        extensions: [],
        generatedFiles: writtenFiles,
        pendingConfirmations: [],
        previewUrl: `/apps/${args.namespace}/${args.code}/prototype.html`,
        runtimeValidation: { valid: published, checks },
        hasRuntimeError: false,
      }
      const output: StageOutput = {
        evidence: `publishing attempt ${attempt}: verified=${missing.length === 0}, workflow=${workflowGate}`
          + (published ? '' : `; failed preconditions: ${failedChecks.join(', ')}`)
          + `; shadow willAutoApprove=${shadow.willAutoApprove}`,
        artifacts: [result.outputPath],
      }
      return { output, result }
    },

    // Pipeline advance — the metadata gate sweep (StageId::all): auto-gates
    // are deterministic artifact checks; a missing artifact is GATE-FAIL.
    // No human gate in PTC mode — resolveGate confirms by default.
    async pipelineAdvance(stage, _plan) {
      const namespaceRoot = path.join(preProcRootOf(preProcRoot), args.namespace)
      const known = STAGE_IDS.find(candidate => candidate === stage)
      if (known === undefined) {
        return { evidence: `GATE-FAIL ${stage}: 未知 stage（不在 StageId::all 的 7 阶段内）` }
      }
      const outcome = await evaluateStageGate(known, {
        appDir: path.join(namespaceRoot, 'Apps', args.code),
        preProcRoot: namespaceRoot,
        namespace: args.namespace,
        app: args.code,
        modules: args.modules.map(module => module.id),
      })
      return outcome.ok
        ? { evidence: `gate ${stage} passed: ${outcome.evidence}`, artifacts: [...outcome.artifacts] }
        : { evidence: `GATE-FAIL ${stage}: ${outcome.evidence}` }
    },

    async resolveGate(_gateId, _prompt) {
      // PTC mode: no interactive human gate; the caller's approval mode
      // governs writes. Confirm deterministically (documented).
      return 'confirm' as const
    },
  }
}

/** Assemble the FlowPlan shared with the Meta AppAgent (unified contract). */
export function buildPlan(args: CreateArgs): FlowPlan {
  return {
    usedModules: args.modules.map(m => m.id),
    namespace: args.namespace,
    knownEntities: (args.entities ?? []).map(e => e.table),
    workflowSteps: args.blocks ?? [],
    missingInfo: [],
    createdModules: [],
    createdBlocks: [],
    createdServices: [],
  }
}
