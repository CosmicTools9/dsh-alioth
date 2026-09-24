/**
 * 7 阶段实质判据 —— 对齐上游 `pipeline/stage_config.yaml`（7 个 stage 的声明产物）
 * 与 `pipeline/stage.rs` 的门禁语义（`RequiredArtifactGate`：声明产物缺失即 Fail，
 * **MUST NOT 静默放行**）。
 *
 * 判据一览（`preProcRoot` = `Pre-Proc/{namespace}`，`appDir` = `Pre-Proc/{namespace}/Apps/{app}`）：
 * - `appagent-ready`  → `{appDir}/pipeline_manifest.json` 或 `{appDir}/app.json` 可解析；
 * - `module-design`   → `Sources/Apps/Modules/{m}/module.json`（`modules` 非空时逐个须齐）；
 * - `block-extract`   → **逐个已声明 block**：`Sources/Apps/Blocks/{block}/block.json`（上游
 *   `stage_config.yaml` 按 `{block}` 模板化；未声明且盘上无产物 ⇒ 无可判对象，如实记）；
 * - `block-refinement`→ block.json 且含交互形态声明（`interactionMode` / `flows` / `workbenchPosts`）；
 * - `ontology-mapping`→ `{preProcRoot}/local/ontology-output.json` 可解析；
 * - `factor-dev`      → **逐个已声明 service**：`Sources/Apps/Services/{service}/service.json`（同理）；
 * - `quality`         → `{appDir}/eval-report.json` 可解析且 **`dimensions.schema_validity === 1`**
 *   （产物契约面；原型面由 per-App 人工门承压——与 orchestrator 同口径）。
 *
 * 失败一律 fail-closed：缺失 / 不可解析 / 未声明 ⇒ `ok=false`，evidence 写明实际查过的路径。
 * @module @dsh-alioth/verify-alioth/stage-gates
 */

import { readFile, readdir } from 'node:fs/promises'
import path from 'node:path'

export type StageId =
  | 'appagent-ready'
  | 'module-design'
  | 'block-extract'
  | 'block-refinement'
  | 'ontology-mapping'
  | 'factor-dev'
  | 'quality'

export interface StageGateOutcome {
  readonly stage: StageId
  readonly ok: boolean
  readonly evidence: string
  readonly artifacts: readonly string[]
  /**
   * 本阶段**不判失败、但义务未落地**的事项：调用方须为其登记 per-App 人工门
   * （解锁条件由门自带）。缺省 = 无待决项。存在 pending 时 `ok` 仍可为 true——
   * 待决不是通过，义务在门上（publish 前置按 App 扫描即阻断）。
   */
  readonly pending?: readonly StageGatePending[]
}

/** 待决项：模型/人的决定尚未落地的可判定形态。 */
export interface StageGatePending {
  readonly kind: 'block-interaction-form'
  readonly block: string
  /** 判据文件（真实路径，非 relToRoot）。 */
  readonly path: string
}

/**
 * 各声明的候选根（上游布局 `Sources/Apps/{Modules,Blocks,Services}` 为主，
 * 短形态 `{Modules,Blocks,Services}` 一并接受——契约的简写与上游布局取并集，避免漏判真实产物）。
 */
const MODULE_ROOTS: readonly string[] = ['Sources/Apps/Modules', 'Modules']
const BLOCK_ROOTS: readonly string[] = ['Sources/Apps/Blocks', 'Blocks']
const SERVICE_ROOTS: readonly string[] = ['Sources/Apps/Services', 'Services']

type JsonProbe = { readonly ok: true; readonly value: unknown } | { readonly ok: false; readonly reason: string }

async function probeJson(file: string): Promise<JsonProbe> {
  const text = await readFile(file, 'utf8').catch(() => null)
  if (text === null) return { ok: false, reason: '缺失/不可读' }
  try {
    return { ok: true, value: JSON.parse(text) as unknown }
  } catch (error) {
    return { ok: false, reason: `不可解析（${error instanceof Error ? error.message : String(error)}）` }
  }
}

/** 在候选根下逐子目录找同一文件名（一层深度；目录不存在 = 无命中）。 */
async function scanChildFile(roots: readonly string[], fileName: string): Promise<readonly string[]> {
  const found: string[] = []
  for (const root of roots) {
    const children = await readdir(root, { withFileTypes: true }).catch(() => null)
    if (children === null) continue
    for (const child of children.sort((a, b) => (a.name < b.name ? -1 : 1))) {
      if (!child.isDirectory()) continue
      const candidate = path.join(root, child.name, fileName)
      if ((await readFile(candidate, 'utf8').catch(() => null)) !== null) found.push(candidate)
    }
  }
  return found
}

/** 相对 `preProcRoot` 的相对路径（在根之外 → 原样绝对路径）。 */
function relToRoot(base: string, file: string): string {
  const rel = path.relative(base, file)
  return rel.startsWith('..') ? file : rel.split(path.sep).join('/')
}

/** block.json 的交互形态声明（流程流 `flows` / 工作台 `workbenchPosts` / 显式 `interactionMode`）。 */
/** 读 block.json 的交互形态声明（canonical 键）；`null` = 未声明。单一判据来源。 */
export function interactionDeclaration(value: unknown): string | null {
  if (typeof value !== 'object' || value === null || Array.isArray(value)) return null
  const record = value as Record<string, unknown>
  // 只认 canonical 键：`interactionMode` **不在** check-block-json 的 CANONICAL_KEYS
  // （R1 unknown-key 会直接判违规），认它等于让模型按我们的门写一个必违真门禁的字段。
  // BLOCK_SCHEMA §1.2：`flows` / `workbenchPosts` 皆 OPTIONAL，二者其一即声明。
  const flows = record['flows']
  if (Array.isArray(flows) && flows.length > 0) return `flows=${flows.length} 条`
  const workbenchPosts = record['workbenchPosts']
  if (Array.isArray(workbenchPosts) && workbenchPosts.length > 0) return `workbenchPosts=${workbenchPosts.length} 条`
  return null
}

/** 求值单个阶段的实质判据（只读磁盘，不改任何状态）。 */
export async function evaluateStageGate(
  stage: StageId,
  input: {
    readonly appDir: string
    readonly preProcRoot: string
    readonly namespace: string
    readonly app: string
    readonly modules?: readonly string[]
    /**
     * 本 run 声明的 block 集合。上游 `stage_config.yaml` 的 block 产物是
     * `Prototypes/Blocks/{block}/b-v{N}.html` + `Sources/Apps/Blocks/{block}/block.json`
     * ——**按 `{block}` 模板化**，故判据 = 逐个已声明 block（与 `module-design` 对
     * `modules` 同规），而不是「无条件至少一个」。未声明且盘上无产物 ⇒ 本 stage 无
     * 可判对象（如实记入 evidence，不静默放行）。
     */
    readonly blocks?: readonly string[]
    /** 本 run 声明的 service 集合（`Sources/Apps/Services/{service}/service.json` 同理模板化）。 */
    readonly services?: readonly string[]
  },
): Promise<StageGateOutcome> {
  const appDir = path.resolve(input.appDir)
  const preProcRoot = path.resolve(input.preProcRoot)
  const outcome = (ok: boolean, evidence: string, artifacts: readonly string[]): StageGateOutcome => ({
    stage,
    ok,
    evidence,
    artifacts,
  })

  switch (stage) {
    case 'appagent-ready': {
      const candidates = [path.join(appDir, 'pipeline_manifest.json'), path.join(appDir, 'app.json')]
      const failures: string[] = []
      for (const candidate of candidates) {
        const probe = await probeJson(candidate)
        if (probe.ok) {
          return outcome(true, `${relToRoot(preProcRoot, candidate)} 可解析（appagent-ready 产物就绪）`, [
            relToRoot(preProcRoot, candidate),
          ])
        }
        failures.push(`${relToRoot(preProcRoot, candidate)}：${probe.reason}`)
      }
      return outcome(false, `pipeline_manifest.json 与 app.json 均不可用（${failures.join('；')}）`, [])
    }

    case 'module-design': {
      const moduleRootsAbs = MODULE_ROOTS.map(root => path.join(preProcRoot, root))
      const modules = input.modules ?? []
      if (modules.length === 0) {
        const found = await scanChildFile(moduleRootsAbs, 'module.json')
        if (found.length === 0) {
          return outcome(false, `未提供 modules 且 ${MODULE_ROOTS.join('|')} 下无 module.json：module-design 判据不可得`, [])
        }
        return outcome(
          true,
          `未提供 modules；发现 ${found.length} 个 module.json（${found.map(file => relToRoot(preProcRoot, file)).join(', ')}）`,
          found.map(file => relToRoot(preProcRoot, file)),
        )
      }
      const artifacts: string[] = []
      const failures: string[] = []
      for (const module of modules) {
        let hit: string | null = null
        for (const root of moduleRootsAbs) {
          const candidate = path.join(root, module, 'module.json')
          const probe = await probeJson(candidate)
          if (probe.ok) {
            hit = candidate
            artifacts.push(relToRoot(preProcRoot, candidate))
            break
          }
        }
        if (hit === null) failures.push(`${module}：module.json 缺失/不可解析`)
      }
      return failures.length === 0
        ? outcome(true, `${modules.length} 个 module 的 module.json 齐备且可解析`, artifacts)
        : outcome(false, `module-design 未就绪：${failures.join('；')}`, artifacts)
    }

    case 'block-extract': {
      const blockRootsAbs = BLOCK_ROOTS.map(root => path.join(preProcRoot, root))
      const declared = input.blocks ?? []
      if (declared.length === 0) {
        const onDisk = await scanChildFile(blockRootsAbs, 'block.json')
        return onDisk.length === 0
          ? outcome(true, `本 run 未声明 block，${BLOCK_ROOTS.join('|')} 下亦无 block.json ⇒ 本 stage 无可判对象（未声明 ≠ 通过：block 由 BlockCreation/block 轨道产出，届时此处逐块判）`, [])
          : outcome(true, `本 run 未声明 block；盘上发现 ${onDisk.length} 个 block.json（按存在如实记）`, onDisk.map(file => relToRoot(preProcRoot, file)))
      }
      const artifacts: string[] = []
      const failures: string[] = []
      for (const block of declared) {
        let hit: string | null = null
        for (const root of blockRootsAbs) {
          const candidate = path.join(root, block, 'block.json')
          if ((await probeJson(candidate)).ok) {
            hit = candidate
            artifacts.push(relToRoot(preProcRoot, candidate))
            break
          }
        }
        if (hit === null) failures.push(`${block}：block.json 缺失/不可解析`)
      }
      return failures.length === 0
        ? outcome(true, `${declared.length} 个已声明 block 的 block.json 齐备且可解析`, artifacts)
        : outcome(false, `block-extract 未就绪：${failures.join('；')}`, artifacts)
    }

    case 'block-refinement': {
      const blockRootsAbs = BLOCK_ROOTS.map(root => path.join(preProcRoot, root))
      const declaredBlocks = input.blocks ?? []
      const found = declaredBlocks.length === 0
        ? await scanChildFile(blockRootsAbs, 'block.json')
        : declaredBlocks.map(block => path.join(blockRootsAbs[0] as string, block, 'block.json'))
      if (declaredBlocks.length === 0 && found.length === 0) {
        return outcome(true, `本 run 未声明 block，${BLOCK_ROOTS.join('|')} 下亦无 block.json ⇒ 本 stage 无可判对象`, [])
      }
      const declaredArtifacts: string[] = []
      const declaredNotes: string[] = []
      const undeclared: string[] = []
      const undeclaredFiles: string[] = []
      const missingDeclared: string[] = []
      for (const file of found) {
        const probe = await probeJson(file)
        if (!probe.ok) {
          undeclared.push(`${relToRoot(preProcRoot, file)}：${probe.reason}`)
          // 已声明却没有产物 = 失败（与 block-extract 同规）；与「文件在、形态待决」不同。
          if (declaredBlocks.length > 0) missingDeclared.push(`${relToRoot(preProcRoot, file)}：${probe.reason}`)
          continue
        }
        const declaration = interactionDeclaration(probe.value)
        if (declaration === null) {
          undeclared.push(`${relToRoot(preProcRoot, file)}：未声明交互形态`)
          undeclaredFiles.push(file)
          continue
        }
        declaredArtifacts.push(relToRoot(preProcRoot, file))
        declaredNotes.push(`${relToRoot(preProcRoot, file)}（${declaration}）`)
      }
      if (missingDeclared.length > 0) {
        return outcome(false, `block-refinement 未就绪：${missingDeclared.join('；')}`, declaredArtifacts)
      }
      if (declaredArtifacts.length > 0) {
        return outcome(true, `${declaredArtifacts.length} 个 block 已声明交互形态：${declaredNotes.join('；')}`, declaredArtifacts)
      }
      // 未声明 = **人/模型的待决**，不是管线失败：上游 stage_config 把 block_refinement
      // 标为 `has_human_gate: true`（问「流程流还是工作台」），而管线只产出骨架、不替人
      // 做决定。按与原型面同一策略：义务转成 per-App 人工门（解锁 = 任一 canonical 声明
      // 出现），由调用方登记；本门如实记 pending，而不 fail 整个管线。
      if (undeclared.length === 0) {
        return outcome(false, `block-refinement 未就绪：${BLOCK_ROOTS.join('|')} 下无 block.json`, [])
      }
      return {
        ...outcome(
          true,
          `待人工/模型决策：${undeclared.length} 个 block 尚未声明交互形态（${undeclared.join('；')}）`
            + '——上游该 stage 为 human gate，义务由 per-App 人工门承压',
          [],
        ),
        pending: undeclaredFiles.map(file => ({
          kind: 'block-interaction-form' as const,
          block: path.basename(path.dirname(file)),
          path: file,
        })),
      }
    }

    case 'ontology-mapping': {
      const target = path.join(preProcRoot, 'local', 'ontology-output.json')
      const probe = await probeJson(target)
      return probe.ok
        ? outcome(true, `${relToRoot(preProcRoot, target)} 可解析（本体映射产物就绪）`, [relToRoot(preProcRoot, target)])
        : outcome(false, `${relToRoot(preProcRoot, target)} ${probe.reason}`, [])
    }

    case 'factor-dev': {
      const serviceRootsAbs = SERVICE_ROOTS.map(root => path.join(preProcRoot, root))
      const declaredServices = input.services ?? []
      if (declaredServices.length === 0) {
        const onDisk = await scanChildFile(serviceRootsAbs, 'service.json')
        return onDisk.length === 0
          ? outcome(true, `本 run 未声明 service，${SERVICE_ROOTS.join('|')} 下亦无 service.json ⇒ 本 stage 无可判对象（service 由 factor/service 轨道产出）`, [])
          : outcome(true, `本 run 未声明 service；盘上发现 ${onDisk.length} 个 service.json（按存在如实记）`, onDisk.map(file => relToRoot(preProcRoot, file)))
      }
      const found = declaredServices.map(service => path.join(serviceRootsAbs[0] as string, service, 'service.json'))
      const usable: string[] = []
      const unusable: string[] = []
      for (const file of found) {
        const probe = await probeJson(file)
        if (probe.ok) usable.push(file)
        else unusable.push(`${relToRoot(preProcRoot, file)}：${probe.reason}`)
      }
      return usable.length > 0
        ? outcome(true, `发现 ${usable.length} 个可解析 service.json`, usable.map(file => relToRoot(preProcRoot, file)))
        : outcome(false, `factor-dev 未就绪：${unusable.length > 0 ? unusable.join('；') : `已声明 ${declaredServices.length} 个 service，均无 service.json`}`, [])
    }

    case 'quality': {
      const target = path.join(appDir, 'eval-report.json')
      const probe = await probeJson(target)
      if (!probe.ok) return outcome(false, `${relToRoot(preProcRoot, target)} ${probe.reason}`, [])
      const passed =
        typeof probe.value === 'object' && probe.value !== null && !Array.isArray(probe.value)
          ? (probe.value as Record<string, unknown>)['passed']
          : undefined
      const dimensions = typeof probe.value === 'object' && probe.value !== null && !Array.isArray(probe.value)
        ? (probe.value as Record<string, unknown>)['dimensions']
        : undefined
      const schemaValidity = typeof dimensions === 'object' && dimensions !== null
        && (dimensions as Record<string, unknown>)['schema_validity'] === 1
      const prototypeStandalone = typeof dimensions === 'object' && dimensions !== null
        && (dimensions as Record<string, unknown>)['prototype_standalone'] === 1
      // 判据 = **产物契约面**（schema_validity），与 orchestrator 的 E2E/发布前置同口径：
      // 原型面（prototype_standalone）是 workflow 步骤的产物，由 per-App 人工门带解锁条件
      // 承压（publishing 前置 2 按 App 扫描）——两处口径若不一，一次 create 会自相矛盾。
      return schemaValidity
        ? outcome(
            true,
            `${relToRoot(preProcRoot, target)} schema_validity=1（overall passed=${String(passed)}，`
              + `prototype_standalone=${prototypeStandalone ? '1' : '0 → 由 per-App 人工门承压'}）`,
            [relToRoot(preProcRoot, target)],
          )
        : outcome(false, `${relToRoot(preProcRoot, target)} schema_validity 未达 1（dimensions=${JSON.stringify(dimensions)}，缺失按 0 分计）`, [])
    }

    default:
      return outcome(false, `未知 stage：${String(stage)}`, [])
  }
}
