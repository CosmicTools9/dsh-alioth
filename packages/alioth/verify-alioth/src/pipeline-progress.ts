/**
 * pipeline 进度的**诚实**投影（上游 `Meta/backend/app-agent/src/pipeline/progress.rs`，
 * change `fix-appagent-pipeline-handoff` B1/B2）。
 *
 * 单一判定源：产物合规性由 `evaluateStageGate`（7 阶段实质判据的唯一实现）判；「是否被尝试过」
 * 由**声明产物模式**在盘上判存在性（上游 `stage_config.yaml` 的 `output_patterns` 同源）。
 * 两者分开是必需的：`evaluateStageGate` 的 `artifacts` 只收**通过**的产物，缺失与形态损坏
 * 都返回 `[]`——拿它判 pending 会把「产物在但坏了」谎报成「尚未开始」。故：
 * - 盘上无任何声明产物 → `pending`（`not_attempted`）——缺失 ≠ 完成，MUST NOT 谎报；
 * - 有产物且 gate 通过 → `completed`（`pass`）；
 * - 有产物但 gate 不过 → 收进 `failures`，调用方须阻断（上游同名情形返回 `Err`：
 *   既不许写 manifest，也不许静默放行）。
 *
 * 与**驱动 run** 的语义差异（刻意，不是遗漏）：orchestrator 的 `pipelineAdvance` 把「产物缺失」
 * 直接判 GATE-FAIL——它驱动的是一次跑到完成的 run，中途缺失就是失败。本模块是**读侧投影**
 * （状态面板 / 交付 manifest），缺失只能如实说 pending。
 *
 * 投影口径声明（B2）：上游 `.pipeline/pipeline.yml` 声明 12 stage，AppAgent typed 投影 7
 * （`stage_config.yaml`）。本仓不读 `.pipeline/pipeline.yml`（那是部署编排的口径），故
 * `projection.non_projected_stages` 显式记 `null` + 原因，而不是假装求过差。
 * @module @dsh-alioth/verify-alioth/pipeline-progress
 */

import { globSync } from 'node:fs'
import { readFile, rename, writeFile } from 'node:fs/promises'
import { createHash } from 'node:crypto'
import path from 'node:path'
import { evaluateStageGate, type StageId } from './stage-gates.ts'

/** stage 的静态声明（上游 `stage_config.yaml` 的同一份投影）。 */
export interface PipelineStageSpec {
  readonly id: StageId
  readonly label: string
  readonly hasHumanGate: boolean
  readonly humanGatePrompt?: string
  /** 声明产物模式（相对命名空间根；`{N}` = 任意版本号，`{module}`/`{block}`/`{service}` 按声明集实例化）。 */
  readonly outputPatterns: readonly string[]
}

/** 7 阶段投影，顺序即上游 `StageId::all()` 的顺序。 */
export const PIPELINE_STAGES: readonly PipelineStageSpec[] = [
  {
    id: 'appagent-ready',
    label: 'AppAgent 产出就绪',
    hasHumanGate: false,
    outputPatterns: ['Apps/{app}/pipeline_manifest.json', 'Apps/{app}/app.json'],
  },
  {
    id: 'module-design',
    label: '模块原型设计',
    hasHumanGate: true,
    humanGatePrompt: 'Module 原型已创建，Block 结构是否确认？',
    outputPatterns: ['Prototypes/Modules/{module}/m-v{N}.html', 'Sources/Apps/Modules/{module}/module.json'],
  },
  {
    id: 'block-extract',
    label: 'Block 自动提取',
    hasHumanGate: false,
    outputPatterns: ['Prototypes/Blocks/{block}/b-v{N}.html', 'Sources/Apps/Blocks/{block}/block.json'],
  },
  {
    id: 'block-refinement',
    label: 'Block 打磨',
    hasHumanGate: true,
    humanGatePrompt: '该 Block 应采用流程流（固定顺序）还是工作台（自由导航）？',
    outputPatterns: ['Prototypes/Blocks/{block}/b-v{N}.html', 'Sources/Apps/Blocks/{block}/block.json'],
  },
  {
    id: 'ontology-mapping',
    label: '本体映射',
    hasHumanGate: true,
    humanGatePrompt: '原型中存在语义歧义字段，逐项确认映射方向',
    outputPatterns: ['local/ontology-output.json'],
  },
  {
    id: 'factor-dev',
    label: 'API 开发',
    hasHumanGate: true,
    humanGatePrompt: 'DTO 形状设计（字段分组方案）',
    outputPatterns: ['Sources/Apps/Services/{service}/service.json'],
  },
  {
    id: 'quality',
    label: '质量检查',
    hasHumanGate: false,
    outputPatterns: ['Apps/{app}/eval-report.json'],
  },
]

/** 单 stage 的清扫结论（诚实两态 + gate 字面）。 */
export interface StageScan {
  readonly id: StageId
  readonly label: string
  readonly status: 'completed' | 'pending'
  readonly gate: 'pass' | 'not_attempted'
  readonly hasHumanGate: boolean
  readonly humanGatePrompt?: string
  /** 仅 completed 携带（本次清扫时刻——投影不持有历史，别把它当「真实完成时间」）。 */
  readonly completedAt?: string
  /** 判定取证（上游 `evidence` 同义）。 */
  readonly evidence: string
}

/** 被判了但不过的 stage（上游同名情形 → `Err`）。 */
export interface StageFailure {
  readonly id: StageId
  readonly evidence: string
}

export interface StageProgressScan {
  readonly stages: readonly StageScan[]
  /** 第一个未完成的投影 stage（全部完成 → null）。 */
  readonly currentStage: StageId | null
  readonly allCompleted: boolean
  /** 非空即「不许写 manifest、不许静默放行」。 */
  readonly failures: readonly StageFailure[]
}

export interface ScanStageProgressInput {
  readonly appDir: string
  /** 命名空间根 = `Pre-Proc/{namespace}`（声明产物模式相对它解析）。 */
  readonly preProcRoot: string
  readonly namespace: string
  readonly app: string
  readonly modules?: readonly string[]
  readonly blocks?: readonly string[]
  readonly services?: readonly string[]
  /**
   * 调用点是否**正在写出** `pipeline_manifest.json`：true 时 `appagent-ready` 视为 completed
   * ——该 stage 的 gate 产物就是本 manifest，写出动作本身即完成依据（上游唯一豁免）。
   */
  readonly atManifestWrite?: boolean
  /** 时钟注入（测试确定性）。 */
  readonly now?: () => Date
}

/** 模式实例化：`{app}`/`{ns}` 固定；集合模板按声明集展开，未声明则收成 `*`。 */
function instantiate(
  pattern: string,
  input: { readonly app: string; readonly namespace: string; readonly modules?: readonly string[]; readonly blocks?: readonly string[]; readonly services?: readonly string[] },
): string[] {
  const sets: Record<string, readonly string[] | undefined> = {
    module: input.modules,
    block: input.blocks,
    service: input.services,
  }
  let expanded = [pattern.replaceAll('{app_code}', input.app).replaceAll('{app}', input.app).replaceAll('{ns}', input.namespace).replaceAll('{N}', '*')]
  for (const [key, values] of Object.entries(sets)) {
    const declared = values !== undefined && values.length > 0 ? values : ['*']
    expanded = expanded.flatMap(current => declared.map(value => current.replaceAll(`{${key}}`, value)))
  }
  return expanded
}

/**
 * 盘上是否存在该 stage 的任何声明产物（「是否被尝试过」的判据）。
 * @param preProcRoot - namespace root.
 * @param spec - stage spec.
 * @param input - declaration sets.
 */
function attempted(preProcRoot: string, spec: PipelineStageSpec, input: ScanStageProgressInput): boolean {
  for (const pattern of spec.outputPatterns) {
    for (const concrete of instantiate(pattern, input)) {
      if (globSync(concrete, { cwd: preProcRoot }).length > 0) {
        return true
      }
    }
  }
  return false
}

/**
 * 逐 stage 清扫（诚实语义，单一判定源）。
 * @param input - app scope + declared collections.
 */
export async function scanStageProgress(input: ScanStageProgressInput): Promise<StageProgressScan> {
  const now = input.now ?? (() => new Date())
  const stages: StageScan[] = []
  const failures: StageFailure[] = []

  for (const spec of PIPELINE_STAGES) {
    const exempt = spec.id === 'appagent-ready' && input.atManifestWrite === true
    const started = exempt || attempted(input.preProcRoot, spec, input)
    const outcome = started
      ? await evaluateStageGate(spec.id, {
          appDir: input.appDir,
          preProcRoot: input.preProcRoot,
          namespace: input.namespace,
          app: input.app,
          ...(input.modules === undefined ? {} : { modules: input.modules }),
          ...(input.blocks === undefined ? {} : { blocks: input.blocks }),
          ...(input.services === undefined ? {} : { services: input.services }),
        })
      : null
    const completed = exempt || outcome?.ok === true
    if (started && !completed) {
      failures.push({ id: spec.id, evidence: outcome?.evidence ?? '未取得判定取证' })
    }
    stages.push({
      id: spec.id,
      label: spec.label,
      status: completed ? 'completed' : 'pending',
      gate: completed ? 'pass' : 'not_attempted',
      hasHumanGate: spec.hasHumanGate,
      ...(spec.humanGatePrompt === undefined ? {} : { humanGatePrompt: spec.humanGatePrompt }),
      ...(completed ? { completedAt: now().toISOString() } : {}),
      evidence: completed
        ? (exempt ? `${outcome?.evidence ?? ''}（at_manifest_write 豁免：本 manifest 即该 stage 产物）` : (outcome?.evidence ?? ''))
        : (outcome === null
            ? `声明产物缺失（${spec.outputPatterns.join(' | ')}）⇒ 尚未尝试`
            : outcome.evidence),
    })
  }

  const firstPending = stages.find(stage => stage.status === 'pending')
  return {
    stages,
    currentStage: firstPending?.id ?? null,
    allCompleted: firstPending === undefined,
    failures,
  }
}

/**
 * `pipeline_progress` 段的形状（与上游逐键同形；叶子皆 JSON 标量，可直接交给工具运行时）。
 * 声明为 type 而非 interface：`JsonValue` 要求索引签名，interface 不会获得隐式索引签名。
 */
export type PipelineProgressJson = {
  readonly current_stage: StageId | null
  readonly stages: Record<string, {
    readonly status: 'completed' | 'pending'
    readonly via?: 'appagent'
    readonly completed_at?: string
  }>
  readonly stage_source: string
  readonly projection: {
    readonly source: string
    readonly registry: string
    readonly scope: 'metadata'
    readonly projected_stages: StageId[]
    readonly non_projected_stages: null
    readonly non_projected_reason: string
  }
}

/** manifest 的 `pipeline_progress` 段（map 形状，与上游逐键同形）。 */
export function pipelineProgressJson(
  scan: StageProgressScan,
  options: { readonly stageSource: string },
): PipelineProgressJson {
  const stages: PipelineProgressJson['stages'] = {}
  for (const stage of scan.stages) {
    stages[stage.id] = stage.status === 'completed'
      ? { status: 'completed', via: 'appagent', ...(stage.completedAt === undefined ? {} : { completed_at: stage.completedAt }) }
      : { status: 'pending' }
  }
  return {
    current_stage: scan.currentStage,
    stages,
    stage_source: options.stageSource,
    projection: {
      source: options.stageSource,
      registry: 'verify-alioth/src/pipeline-progress.ts',
      scope: 'metadata',
      projected_stages: PIPELINE_STAGES.map(stage => stage.id),
      non_projected_stages: null,
      non_projected_reason: '本仓不读 .pipeline/pipeline.yml（上游 12 stage 口径归部署编排）；投影 = AppAgent typed 7 阶段',
    },
  }
}

/** `artifact_manifest` 的一条（上游同形：相对路径 + `sha256:` + media_type + stage_id）。 */
export interface ArtifactEntry {
  readonly path: string
  readonly content_hash: string
  readonly media_type: string
  readonly stage_id: string
}

/** 登记一条产物（不可读即**不登记**——manifest 只声明真实存在的东西）。 */
export async function artifactEntry(root: string, file: string, stageId: string): Promise<ArtifactEntry | null> {
  const bytes = await readFile(file).catch(() => null)
  if (bytes === null) {
    return null
  }
  const digest = createHash('sha256').update(bytes).digest('hex')
  const rel = path.relative(root, file)
  const ext = path.extname(file).toLowerCase()
  return {
    path: rel.startsWith('..') ? file : rel.split(path.sep).join('/'),
    content_hash: `sha256:${digest}`,
    media_type: ext === '.json' ? 'application/json' : ext === '.yaml' || ext === '.yml' ? 'application/yaml' : 'application/octet-stream',
    stage_id: stageId,
  }
}

/** 交付 manifest（`{appDir}/pipeline_manifest.json`）：进度投影 + 产物清单 + 未决人工门。 */
export interface PipelineManifest {
  readonly pipeline_progress: PipelineProgressJson
  readonly artifact_manifest: { readonly entries: readonly ArtifactEntry[] }
  /** 本仓增量：未决 per-App 人工门（上游把人工门作为 stage 静态声明呈现，未决项由其 deferred 面承载）。 */
  readonly open_human_gates: readonly string[]
}

/**
 * 原子写出 `pipeline_manifest.json`（tmp + rename，同上游）。
 * @param appDir - `Pre-Proc/{ns}/Apps/{app}`.
 * @param manifest - the manifest body.
 * @returns the written path.
 */
export async function writePipelineManifest(appDir: string, manifest: PipelineManifest): Promise<string> {
  const target = path.join(appDir, 'pipeline_manifest.json')
  const temp = path.join(appDir, '.pipeline_manifest.json.tmp')
  await writeFile(temp, `${JSON.stringify(manifest, null, 2)}\n`)
  await rename(temp, target)
  return target
}
