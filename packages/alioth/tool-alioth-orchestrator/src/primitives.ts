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
  interactionDeclaration,
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
import { generateBlock, generateService } from '@dsh-alioth/gen-alioth'
import { PIPELINE_SHARED_SURFACES, runControlledParallel } from './parallel.ts'

export interface CreateArgs {
  readonly namespace: string
  readonly code: string
  readonly name: string
  readonly modules: ReadonlyArray<{ readonly id: string; readonly name: string }>
  readonly blocks?: readonly string[]
  /**
   * 本 run 声明的 service 集合（id）。缺省 = 未声明：`factor-dev` 阶段门按「盘上
   * 有无 service.json」实例化，不会因此凭空要求产物（见 stage-gates 文档）。
   */
  readonly services?: readonly string[]
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
interface QualityReportView {
  readonly status: 'passed' | 'failed' | 'missing'
  /** 产物契约面（id/code/namespace/name/version/status）——管线自身的交付面。 */
  readonly schemaValidity: boolean
  /** 原型面——workflow 步骤的产物，由 per-App 人工门承压（见 prototypeGateItem）。 */
  readonly prototypeStandalone: boolean
}

async function readQualityReport(appDir: string): Promise<QualityReportView> {
  const text = await readFile(path.join(appDir, 'eval-report.json'), 'utf8').catch(() => null)
  if (text === null) return { status: 'missing', schemaValidity: false, prototypeStandalone: false }
  const report = jsonObjectOf(text)
  const dimensions = report === null ? null : report['dimensions']
  const dimension = (name: string): boolean =>
    typeof dimensions === 'object' && dimensions !== null
    && (dimensions as Record<string, unknown>)[name] === 1
  return {
    status: report !== null && report['passed'] === true ? 'passed' : 'failed',
    schemaValidity: dimension('schema_validity'),
    prototypeStandalone: dimension('prototype_standalone'),
  }
}

/** block 交互形态人工门的作用域前缀（`alioth_deferred list` 依此识别）。 */
const BLOCK_FORM_GATE_PREFIX = 'app-block-form-'

/**
 * block 交互形态的 per-App 人工门。上游 `stage_config.yaml` 把 `block_refinement`
 * 标为 `has_human_gate: true`（问「流程流还是工作台」）——该决定属人与模型，管线只
 * 产出骨架，替不了这个决定，故不得因此 fail 整条管线；义务落在本门上。
 *
 * 解锁条件 = block.json 里 `flows`/`workbenchPosts` **任一**已声明（非空）。用关键词
 * 「已声明」（`artifact-json-pointer-exists`）而非「等于某值」：BLOCK_SCHEMA §1.2 里两者
 * 皆 OPTIONAL、内容由模型决定，无法预知取值。`interactionMode` **不在** check-block-json
 * 的 CANONICAL_KEYS（R1 会判违规），故不作判据。
 */
function blockInteractionGateItem(input: {
  readonly namespace: string
  readonly app: string
  readonly block: string
  readonly path: string
}): DeferredItem {
  return {
    id: `block-interaction-form:${input.namespace}/${input.app}:${input.block}`,
    sessionId: `${BLOCK_FORM_GATE_PREFIX}${input.namespace}-${input.app}`,
    app: input.app,
    namespace: input.namespace,
    reason: `block ${input.block} 尚未声明交互形态（flows / workbenchPosts）——上游 block_refinement 为 human gate`,
    adjudication: '流程流（固定顺序）还是工作台（自由导航）是业务决定，只能由人/模型给；'
      + '未决定前该 block 的导航与前端绑定不可判，publish 必须被阻断',
    trigger: {
      kind: 'artifact-json-pointer-exists',
      path: input.path,
      pointers: ['/flows', '/workbenchPosts'],
    },
    successors: [
      `在 ${input.block}/block.json 声明 flows 或 workbenchPosts（canonical 键，禁止 interactionMode——R1 会判违规）`,
      '重跑 alioth_app_create 或直接 sweep：声明出现即自动解除本门',
    ],
    createdTs: new Date().toISOString(),
  }
}

/** 原型人工门的作用域前缀（`alioth_deferred list` 依此识别人工门）。 */
const PROTOTYPE_GATE_PREFIX = 'app-prototype-'

/**
 * 原型未产出的 per-App 人工门。解除条件 = **重跑后的** `eval-report.json` 里
 * `/dimensions/prototype_standalone == 1`——即「一次真实通过」解除（与
 * `extensions-degraded` 门同纪律），而不是锚死某个文件名。
 *
 * 为何不锚 `artifact-exists`：上游原型步骤（`alioth-app.yaml` / `alioth-compose.yaml`）
 * 跑 `prototype-tool.js build …/Prototypes/Apps/{app}/llm-tsx/app.tsx`，产物是
 * `Prototypes/Apps/{app}/a-v*.html`（其 `output_glob`），全链无人写 `prototype.html`；
 * 锚文件名会把门锁在一条永不出现的路径上。以评估维度为判据，也让「先产出原型、
 * 再重跑评估」这条正当路径自然解锁。
 */
function prototypeGateItem(input: {
  readonly appDir: string
  readonly namespace: string
  readonly app: string
}): DeferredItem {
  return {
    id: `prototype-standalone:${input.namespace}/${input.app}`,
    sessionId: `${PROTOTYPE_GATE_PREFIX}${input.namespace}-${input.app}`,
    app: input.app,
    namespace: input.namespace,
    reason: '原型未产出：eval-report 的 prototype_standalone 维度为 0（standalone 面无从判定，缺失记 0 分）',
    adjudication: '原型是 workflow 步骤的产物（Pre-Proc/{ns}/Prototypes/Apps/{app}/，输出 a-v*.html），'
      + '不属 PTC 管线职责；publish 前必须产出原型并重跑评估使该维度真实通过——本门不由管线自行解除',
    trigger: {
      kind: 'artifact-json-pointer',
      path: path.join(input.appDir, 'eval-report.json'),
      pointer: '/dimensions/prototype_standalone',
      equals: 1,
    },
    successors: [
      '在 workflow 步骤产出原型（llm-tsx/app.tsx → prototype-tool.js build）',
      '重跑 alioth_verify artifacts 让 eval-report 重评该维度（只有真实通过才解除本门）',
    ],
    createdTs: new Date().toISOString(),
  }
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
      // 骨架落盘（对齐上游 `create_block_scaffold`：`block: ""` / `coordinates: null`
      // 留待精化与本体映射阶段回填）。**已存在不覆写**（上游 Y4：workflow 技能步骤
      // 可能已更新过该 block.json，覆写会丢掉模型的工作）。落点与上游一致：
      // `Pre-Proc/{ns}/Sources/Apps/Blocks/{id}/block.json`——block-extract 阶段门禁读它。
      const namespaceRoot = path.join(preProcRootOf(preProcRoot), args.namespace)
      const declared = args.blocks ?? []
      if (declared.length === 0) {
        const existing = writtenFiles.filter(file => file.endsWith('block.json'))
        return {
          evidence: 'block creation: 本 run 未声明 block（无骨架可写；block 产物由 block 轨道产出）',
          artifacts: existing,
        }
      }
      const { modelVersion } = await ctx.aliothEnv.ready()
      const created: string[] = []
      const kept: string[] = []
      const artifacts: string[] = []
      const pendingForms: string[] = []
      for (const id of declared) {
        const file = path.join(namespaceRoot, 'Sources', 'Apps', 'Blocks', id, 'block.json')
        if (await readFile(file, 'utf8').then(() => true, () => false)) {
          kept.push(id)
          artifacts.push(file)
        } else {
          try {
            await mkdir(path.dirname(file), { recursive: true })
            const scaffold = generateBlock({ id, namespace: args.namespace, name: id, aliothVersion: modelVersion })
            await writeFile(file, `${JSON.stringify(scaffold, null, 2)}\n`, 'utf8')
            created.push(id)
            artifacts.push(file)
          } catch (error) {
            return {
              evidence: `GATE-FAIL block creation: ${id} 骨架写出失败（${describe(error)})`,
              artifacts,
            }
          }
        }
        // 交互形态是上游的 human gate（流程流 vs 工作台）：骨架期必然未决，故**此刻**登记
        // per-App 人工门——7 阶段门扫描发生在 publishing 之后，若留到那时登记，本次发布
        // 就已经放过去了（首次发布漏过 = fail-closed 失效）。声明落地后 sweep 自动解除。
        const declaration = interactionDeclaration(await readFile(file, 'utf8').then(
          text => jsonObjectOf(text) as unknown,
          () => null,
        ))
        if (declaration === null) {
          try {
            await createDeferredStore(ctx.aliothEnv.dataRoot()).register(
              blockInteractionGateItem({ namespace: args.namespace, app: args.code, block: id, path: file }),
            )
            pendingForms.push(id)
          } catch (error) {
            return {
              evidence: `GATE-FAIL block creation: ${id} 待决人工门登记失败（${describe(error)}）`,
              artifacts,
            }
          }
        }
      }
      return {
        evidence: `block creation: ${declared.length} declared, ${created.length} scaffold written`
          + ` (${created.join(', ') || 'none'}), ${kept.length} kept as-is (never overwritten)`
          + (pendingForms.length === 0 ? '' : `；${pendingForms.length} 个 block 交互形态待决已登记人工门（publish 前须声明）`),
        artifacts,
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
      // 骨架落盘（对齐上游 `<ns>/Sources/Apps/Services/{service}/service.json` 布局）。
      // 取值都有实例依据（AliothStudio Pre-Proc 的真实 service.json：`layer` 5/5 为 1、
      // `backendCrate` = `<ns小写>-service-<id>`），未据之处如实置为「未产出」：
      // `hasBackend:false`（后端 crate 由 service 轨道编写，本管线不写）、
      // `ontology.entities:[]`（实体映射由本体阶段回填）、`domain` 缺省取 id（精化阶段替换）。
      const namespaceRoot = path.join(preProcRootOf(preProcRoot), args.namespace)
      const declared = args.services ?? []
      const created: string[] = []
      const kept: string[] = []
      const artifacts: string[] = []
      for (const id of declared) {
        const file = path.join(namespaceRoot, 'Sources', 'Apps', 'Services', id, 'service.json')
        if (await readFile(file, 'utf8').then(() => true, () => false)) {
          kept.push(id)
          artifacts.push(file)
          continue
        }
        try {
          await mkdir(path.dirname(file), { recursive: true })
          const scaffold = generateService({
            id,
            namespace: args.namespace,
            domain: id,
            services: [],
            // layer 1 = 实例侧的既有取值（5/5 真实 service.json 皆 1）
            layer: 1,
            dtoDependencies: [],
            backendCrate: `${args.namespace.toLowerCase()}-service-${id}`,
            hasBackend: false,
            hasFrontend: false,
            ontology: { entities: [] },
          })
          await writeFile(file, `${JSON.stringify(scaffold, null, 2)}\n`, 'utf8')
          created.push(id)
          artifacts.push(file)
        } catch (error) {
          return {
            evidence: `GATE-FAIL service API: ${id} 骨架写出失败（${describe(error)}）`,
            artifacts,
          }
        }
      }
      const existing = writtenFiles.filter(f => f.includes('service'))
      return {
        evidence: declared.length === 0
          ? `service API: 本 run 未声明 service；${existing.length} service artifacts contract-validated`
          : `service API: ${declared.length} declared, ${created.length} scaffold written`
            + ` (${created.join(', ') || 'none'}), ${kept.length} kept as-is (never overwritten)`,
        artifacts: [...new Set([...artifacts, ...existing])],
      }
    },

    // 8. E2E verification — 验证证据的生产阶段：真实浏览器全链是人工验收项，
    //    这里跑确定性等价检查（app.json / module.json / standalone 原型 / 扩展声明覆盖）
    //    并如实落盘 §5.2 的 publish 前置证据（extension-verify.json、eval-report.json、
    //    产物版本快照、closure-audit 裁决）。任一检查未通过或证据写盘失败 → 证据以
    //    "E2E failed" 开头驱动修复循环，绝不按通过处理。
    async e2eVerification(attempt, _plan, finalAttempt = true) {
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

      // quality 评估报告：维度缺失按 0 分记（诚实评分，报告口径不动）。本阶段的**通过
      // 判据只取管线自己的产物契约面**（schema_validity）：原型是 workflow 步骤的产物
      // （上游产物为 `Prototypes/Apps/{app}/a-v*.html`），PTC 管线不产出原型——把它算作
      // 本阶段失败会让裸 create 必然三连败，并在无缺陷可修时驱动修复循环。原型面的义务
      // 改由 per-App 人工门承压（publish 前置 2 按 App 扫描，fail-closed 不破）。
      let qualityPassed = false
      let prototypeOk = false
      try {
        const report = await buildEvalReport({ app: args.code, namespace: args.namespace, appDir })
        qualityPassed = report.dimensions.schema_validity === 1
        prototypeOk = report.dimensions.prototype_standalone === 1
        evidenceArtifacts.push(await writeEvalReport(appDir, report))
      } catch (error) {
        failures.push(`eval-report write failed (${describe(error)})`)
      }
      checks.push({
        id: 'quality',
        passed: qualityPassed,
        description: 'eval-report 产物契约面（schema_validity）',
      })

      // 一份构建级证据（版本快照 + 结束审计）只属于**本次构建的终局尝试**：一次未修复
      // 构建的 3 次内部重试若各写一份，会留下 3 个版本目录与 3 条 rejected 裁决，而
      // closure audit 按连续 rejected 升级人工——一次调用就把该 App 打成人工升级。
      // 判定：功能面全过即为终局（不会再重试）；否则以机器给的 finalAttempt 为准。
      // 中间尝试仍写 e2e-report（本次尝试的判定），供重试循环与人工回溯。
      const functionallyPassed = checks.every(check => check.passed)
      const concluding = functionallyPassed || finalAttempt

      // 原型缺失 → 登记 per-App 人工门（幂等按 id）。阻断 publish 正是它的目的，
      // 故登记成功即本阶段该做的事完成；登记失败 fail-closed（记 failure + 检查不过）。
      let prototypeGateOk = prototypeOk
      if (concluding && !prototypeOk) {
        try {
          await createDeferredStore(ctx.aliothEnv.dataRoot())
            .register(prototypeGateItem({ appDir, namespace: args.namespace, app: args.code }))
          prototypeGateOk = true
        } catch (error) {
          failures.push(`prototype gate register failed (${describe(error)})`)
        }
      }
      checks.push({
        id: 'prototype-gate',
        passed: prototypeGateOk,
        description: prototypeOk
          ? '原型面已通过（无需登记人工门）'
          : '原型缺失已登记 per-App 人工门（publish 前置按 App 可见，直至真实通过）',
      })

      let snapshotDir = ''
      if (concluding) {
        try {
          const snapshot = await snapshotArtifacts(appDir)
          snapshotDir = snapshot.dir
          evidenceArtifacts.push(snapshot.dir)
        } catch (error) {
          failures.push(`snapshot failed (${describe(error)})`)
        }
      }
      checks.push({
        id: 'artifact-snapshot',
        passed: snapshotDir !== '',
        description: concluding ? '产物版本快照写入成功' : '中间尝试不落快照（终局尝试才写）',
      })

      // 独立结束审计：指纹锁死本次产物，裁决 append-only（连续 rejected 升级人工）。
      let closureSeq = 0
      if (concluding) {
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
      }
      checks.push({
        id: 'closure-audit',
        passed: closureSeq > 0,
        description: concluding ? `独立结束审计裁决已落盘（seq=${closureSeq}）` : '中间尝试不落裁决（终局尝试才写）',
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

      // 前置 3：quality 阶段 eval-report.json 存在且**产物契约面**通过。
      // 原型面不在本前置判定：它由 per-App 人工门承压（前置 2 可见），重复判只会让
      // 失败原因重叠、指向同一条未决门。报告的 overall `passed` 仍如实落盘。
      const qualityStatus = await readQualityReport(appDir)
      const qualityOk = qualityStatus.status !== 'missing' && qualityStatus.schemaValidity
      checks.push({
        name: 'publish-quality',
        ok: qualityOk,
        detail: qualityOk
          ? `eval-report.json schema_validity=1（overall passed=${qualityStatus.status === 'passed'}，`
            + `prototype_standalone=${qualityStatus.prototypeStandalone}）`
          : publishRepair(
              qualityStatus.status === 'missing' ? PUBLISH_RULES.qualityMissing : PUBLISH_RULES.qualityNotPassed,
              `${path.join(appDir, 'eval-report.json')} status=${qualityStatus.status}`
                + ` schema_validity=${qualityStatus.schemaValidity}`,
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
        // 影子镜像真实前置：quality 的判据是产物契约面（原型面由人工门承压）
        qualityPassed: qualityStatus.status !== 'missing' && qualityStatus.schemaValidity,
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
      const failing = checks.filter(check => !check.ok)
      const failedChecks = failing.map(check => check.name)
      // 首个失败前置的**取证**也进 evidence：只报检查名（如 publish-no-open-degraded-gates）
      // 等于让调用方自己去猜是哪个门/哪条规则，而 detail 里已经有门 id、原因与修复方向。
      const firstFailure = failing[0]?.detail ?? ''
      const firstFailureShown = firstFailure.length > 400 ? `${firstFailure.slice(0, 400)}…` : firstFailure
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
          + (published ? '' : `; failed preconditions: ${failedChecks.join(', ')}${firstFailureShown === '' ? '' : ` — ${firstFailureShown}`}`)
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
        // 声明的 block 集：本 run 没声明 block 时该 stage 无可判对象（见 stage-gates 文档）。
        blocks: args.blocks ?? [],
        // 声明的 service 集：未声明时该 stage 按「盘上有无 service.json」实例化。
        services: args.services ?? [],
      })
      // 待决项（模型/人的决定未落地）→ 登记 per-App 人工门：本阶段不判失败，但义务只能
      // 落在门上（publish 前置 2 按 App 扫描即 fail-closed）；登记失败则 fail-closed 到底。
      const gateErrors: string[] = []
      for (const pending of outcome.pending ?? []) {
        try {
          // 幂等（store 按 id 去重）：blockCreation 已登记的不再重复，此处兜住
          // 「由别处产出 block.json」的路径。
          await createDeferredStore(ctx.aliothEnv.dataRoot()).register(
            blockInteractionGateItem({
              namespace: args.namespace,
              app: args.code,
              block: pending.block,
              path: pending.path,
            }),
          )
        } catch (error) {
          gateErrors.push(`${pending.block}: ${describe(error)}`)
        }
      }
      if (gateErrors.length > 0) {
        return { evidence: `GATE-FAIL ${stage}: 待决人工门登记失败（${gateErrors.join('; ')}）`, artifacts: [] }
      }
      const pendingCount = outcome.pending?.length ?? 0
      const pendingNote = pendingCount === 0 ? '' : `；待决 ${pendingCount} 项已登记 per-App 人工门`
      return outcome.ok
        ? { evidence: `gate ${stage} passed: ${outcome.evidence}${pendingNote}`, artifacts: [...outcome.artifacts] }
        : { evidence: `GATE-FAIL ${stage}: ${outcome.evidence}${pendingNote}` }
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
