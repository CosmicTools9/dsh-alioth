/**
 * dsh-alioth 的 AppAgent 构建回归基线跑者（消费侧）。
 *
 * 上游 `scripts/eval/appagent-build-eval.ts` 驱动 `appagent-client.ts` 打 Meta 后端 HTTP API——
 * 那是 Meta 仓的跑法；本仓没有 Meta 服务，证据面因此改为**本仓自己落盘的机械证据**：
 * `extension-verify.json` / `e2e-report.json` / `AppAgentTraces/closure-audit/{seq}.json` /
 * `eval-report.json` / 产物清单。评分与回归裁决复用 `@dsh-alioth/verify-alioth` 的
 * `parseEvalCases` / `scoreRun` / `decideRegression`（同一份 fail-closed 语义，不另立第二套）。
 *
 * 证据缺失 = 该维度 0 分且整轮 `degraded`（≠ 通过）：`degraded` 结果 MUST NOT 用于更新基线。
 *
 * 用法：
 *   node --import tsx scripts/appagent-build-eval.ts validate --cases <path>
 *   node --import tsx scripts/appagent-build-eval.ts run --cases <path> [--baseline <path>]
 *       [--write-baseline] [--gate] [--json] [--pre-proc <dir>]
 *
 * `--gate` 是唯一会以非零码退出的模式（回归或未达阈值）；缺省只报告，不改任何东西
 * （对齐上游"先非阻断登记、阻断化另立变更"的节奏）。
 * @module scripts/appagent-build-eval
 */

import { mkdir, readFile, readdir, writeFile } from 'node:fs/promises'
import { homedir } from 'node:os'
import path from 'node:path'
import { fileURLToPath } from 'node:url'
import {
  decideRegression,
  parseEvalCases,
  scoreRun,
  EVAL_DIMENSIONS,
  type EvalCase,
  type EvalCaseSet,
  type EvalEvidence,
} from '@dsh-alioth/verify-alioth'

interface Options {
  readonly command: 'validate' | 'run'
  readonly cases: string
  readonly baseline: string | undefined
  readonly writeBaseline: boolean
  readonly gate: boolean
  readonly json: boolean
  readonly preProcRoot: string
  readonly quiet: boolean
}

const USAGE = `用法：
  appagent-build-eval validate --cases <path>
  appagent-build-eval run --cases <path> [--baseline <path>] [--write-baseline] [--gate] [--json] [--pre-proc <dir>]`

class UsageError extends Error {}

function parseOptions(argv: readonly string[]): Options {
  const [command, ...rest] = argv
  if (command !== 'validate' && command !== 'run') {
    throw new UsageError(USAGE)
  }
  let cases: string | undefined
  let baseline: string | undefined
  let writeBaseline = false
  let gate = false
  let json = false
  let quiet = false
  let preProcRoot = process.env.ALIOTH_PRE_PROC_ROOT ?? path.join(homedir(), '.dsh-alioth', 'Pre-Proc')
  for (let index = 0; index < rest.length; index += 1) {
    const flag = rest[index]
    const value = rest[index + 1]
    switch (flag) {
      case '--cases':
        if (value === undefined) throw new UsageError(`--cases 缺参数\n${USAGE}`)
        cases = value
        index += 1
        break
      case '--baseline':
        if (value === undefined) throw new UsageError(`--baseline 缺参数\n${USAGE}`)
        baseline = value
        index += 1
        break
      case '--pre-proc':
        if (value === undefined) throw new UsageError(`--pre-proc 缺参数\n${USAGE}`)
        preProcRoot = value
        index += 1
        break
      case '--write-baseline':
        writeBaseline = true
        break
      case '--gate':
        gate = true
        break
      case '--json':
        json = true
        break
      case '--quiet':
        quiet = true
        break
      default:
        throw new UsageError(`未知参数 ${String(flag)}\n${USAGE}`)
    }
  }
  if (cases === undefined) {
    throw new UsageError(`--cases 必填（本仓不内置基准任务集：空集会产出全 0 的假信号）\n${USAGE}`)
  }
  if (command === 'validate' && (gate || writeBaseline || baseline !== undefined)) {
    throw new UsageError(`validate 只校验任务集，不接受 --baseline/--write-baseline/--gate\n${USAGE}`)
  }
  return { command, cases, baseline, writeBaseline, gate, json, preProcRoot, quiet }
}

/** Read a JSON artifact; `null` when absent or unparsable (caller scores it 0, never 1). */
async function readJsonArtifact(file: string): Promise<Record<string, unknown> | null> {
  let raw: string
  try {
    raw = await readFile(file, 'utf8')
  } catch {
    return null
  }
  try {
    const parsed: unknown = JSON.parse(raw)
    return typeof parsed === 'object' && parsed !== null && !Array.isArray(parsed)
      ? parsed as Record<string, unknown>
      : null
  } catch {
    return null
  }
}

/** Newest closure verdict by sequence number, or `null` when none was written. */
async function latestClosureVerdict(appDir: string): Promise<Record<string, unknown> | null> {
  const dir = path.join(appDir, 'AppAgentTraces', 'closure-audit')
  let entries: string[]
  try {
    entries = await readdir(dir)
  } catch {
    return null
  }
  const numbered = entries
    .filter(entry => entry.endsWith('.json'))
    .map(entry => ({ entry, seq: Number.parseInt(path.basename(entry, '.json'), 10) }))
    .filter(item => Number.isInteger(item.seq))
    .sort((left, right) => right.seq - left.seq)
  for (const item of numbered) {
    const verdict = await readJsonArtifact(path.join(dir, item.entry))
    if (verdict !== null) return verdict
  }
  return null
}

/** Count of `*.yaml` under `extensions/` (0 when the directory is absent). */
async function extensionFileCount(appDir: string): Promise<number> {
  const entries = await readdir(path.join(appDir, 'extensions')).catch(() => null)
  return entries === null ? 0 : entries.filter(entry => entry.endsWith('.yaml')).length
}

/** Declared artifacts present under the app tree, as a 0..1 fraction (missing → 0, never full). */
async function artifactCoverage(appDir: string): Promise<number> {
  // Denominator: app.json plus the extensions actually declared — at least one
  // extension slot, so an app with no extensions/ directory still has a ratio.
  const declared = Math.max(1, await extensionFileCount(appDir))
  const appJson = await readJsonArtifact(path.join(appDir, 'app.json')) !== null ? 1 : 0
  return (appJson + Math.min(declared, await extensionFileCount(appDir))) / (1 + declared)
}

/**
 * Mechanical evidence per case. Every dimension is independently derived from the
 * artifacts our own tools write; a dimension whose artifact is absent is `undefined`
 * so `scoreRun` records 0 + degraded rather than a default.
 */
export async function collectEvidence(input: {
  readonly caseSpec: EvalCase
  readonly preProcRoot: string
}): Promise<EvalEvidence> {
  const appDir = path.join(input.preProcRoot, input.caseSpec.namespace, 'Apps', input.caseSpec.app)
  const evidence: {
    extension_verify?: number
    e2e?: number
    closure_audit?: number
    eval_report_rules?: number
    artifacts?: number
  } = {}

  const extensionVerify = await readJsonArtifact(path.join(appDir, 'extension-verify.json'))
  if (extensionVerify !== null) {
    evidence.extension_verify = extensionVerify.status === 'passed' ? 1 : 0
  }

  const e2e = await readJsonArtifact(path.join(appDir, 'e2e-report.json'))
  if (e2e !== null) {
    evidence.e2e = e2e.passed === true ? 1 : 0
  }

  const closure = await latestClosureVerdict(appDir)
  if (closure !== null) {
    evidence.closure_audit = closure.verdict === 'approved' ? 1 : 0
  }

  const evalReport = await readJsonArtifact(path.join(appDir, 'eval-report.json'))
  if (evalReport !== null) {
    const dimensions = evalReport.dimensions
    const values = typeof dimensions === 'object' && dimensions !== null && !Array.isArray(dimensions)
      ? Object.values(dimensions as Record<string, unknown>).filter((value): value is number => typeof value === 'number')
      : []
    evidence.eval_report_rules = values.length === 0
      ? 0
      : values.reduce((sum, value) => sum + value, 0) / values.length
  }

  const artifacts = await artifactCoverage(appDir)
  if (artifacts > 0) {
    evidence.artifacts = artifacts
  }

  return evidence
}

/** Evidence for every case in the set, keyed by case id. */
export async function collectCaseEvidence(input: {
  readonly caseSet: EvalCaseSet
  readonly preProcRoot: string
}): Promise<Record<string, EvalEvidence>> {
  const collected: Record<string, EvalEvidence> = {}
  for (const caseSpec of input.caseSet.cases) {
    collected[caseSpec.id] = await collectEvidence({ caseSpec, preProcRoot: input.preProcRoot })
  }
  return collected
}

/** Baseline document shape — case id → weighted score, plus the schema tag and timestamp. */
interface Baseline {
  readonly schema: string
  readonly ts: string
  readonly scores: Record<string, number>
}

async function readBaseline(file: string): Promise<Baseline | null> {
  const parsed = await readJsonArtifact(file)
  if (parsed === null) return null
  const scores = parsed.scores
  if (typeof scores !== 'object' || scores === null || Array.isArray(scores)) return null
  const clean: Record<string, number> = {}
  for (const [key, value] of Object.entries(scores as Record<string, unknown>)) {
    if (typeof value === 'number' && Number.isFinite(value)) clean[key] = value
  }
  return { schema: typeof parsed.schema === 'string' ? parsed.schema : 'dsh-alioth-build-eval/v1', ts: typeof parsed.ts === 'string' ? parsed.ts : '', scores: clean }
}

async function main(): Promise<number> {
  let options: Options
  try {
    options = parseOptions(process.argv.slice(2))
  } catch (error) {
    process.stderr.write(`${error instanceof Error ? error.message : String(error)}\n`)
    return 2
  }

  const document = await readFile(options.cases, 'utf8')
  const caseSet = parseEvalCases(document)

  if (options.command === 'validate') {
    const dimensionKeys = new Set(caseSet.cases.flatMap(entry => Object.keys(entry.dimensions)))
    const unknown = [...dimensionKeys].filter(key => !EVAL_DIMENSIONS.includes(key))
    if (unknown.length > 0) {
      process.stderr.write(`任务集引用了未知维度：${unknown.join(', ')}\n`)
      return 2
    }
    process.stdout.write(`任务集合法：schema=${caseSet.schema} cases=${caseSet.cases.length} threshold=${caseSet.threshold} tolerance=${caseSet.tolerance}\n`)
    return 0
  }

  const evidence = await collectCaseEvidence({ caseSet, preProcRoot: options.preProcRoot })
  const scored = scoreRun({ caseSet, evidence })

  const report = {
    schema: caseSet.schema,
    threshold: caseSet.threshold,
    tolerance: caseSet.tolerance,
    score: scored.score,
    degraded: scored.degraded,
    perCase: scored.perCase,
    evidence,
  }

  if (options.json) {
    process.stdout.write(`${JSON.stringify(report, null, 2)}\n`)
  } else if (!options.quiet) {
    process.stdout.write(`构建回归评测：score=${scored.score.toFixed(4)} threshold=${caseSet.threshold} degraded=${String(scored.degraded)}\n`)
    for (const caseSpec of caseSet.cases) {
      const perCase = scored.perCase[caseSpec.id] ?? 0
      const dims = evidence[caseSpec.id] ?? {}
      const dimText = EVAL_DIMENSIONS
        .map(dimension => `${dimension}=${typeof dims[dimension as keyof EvalEvidence] === 'number' ? String(dims[dimension as keyof EvalEvidence]) : 'missing'}`)
        .join(' ')
      process.stdout.write(`  ${caseSpec.id} (${caseSpec.namespace}/${caseSpec.app}) score=${perCase.toFixed(4)} ${dimText}\n`)
    }
  }

  if (options.writeBaseline) {
    if (scored.degraded) {
      process.stderr.write('拒绝写基线：本轮 degraded（证据缺失）——degraded ≠ 通过，不得作为基线。\n')
      return 2
    }
    const target = options.baseline ?? path.join(options.preProcRoot, '..', 'appagent-build-eval-baseline.json')
    await mkdir(path.dirname(target), { recursive: true })
    const baseline: Baseline = { schema: 'dsh-alioth-build-eval/v1', ts: new Date().toISOString(), scores: scored.perCase }
    await writeFile(target, `${JSON.stringify(baseline, null, 2)}\n`, 'utf8')
    process.stdout.write(`基线已写入 ${target}\n`)
  }

  if (!options.gate) {
    return 0
  }

  const baselineFile = options.baseline
  if (baselineFile === undefined) {
    process.stderr.write('--gate 需要 --baseline（没有基线就没有回归可比）。\n')
    return 2
  }
  const baseline = await readBaseline(baselineFile)
  if (baseline === null) {
    process.stderr.write(`基线不可读或结构非法：${baselineFile}\n`)
    return 2
  }
  const verdict = decideRegression({ baseline: baseline.scores, current: scored.perCase, caseSet })
  process.stdout.write(`回归裁决：${verdict.verdict} —— ${verdict.detail}\n`)
  return verdict.verdict === 'pass' ? 0 : 1
}

// Entry guard: the module is imported by its spec (evidence collection is testable in
// isolation), so the CLI only runs when this file is the process entry point.
const invokedDirectly = process.argv[1] !== undefined
  && path.resolve(process.argv[1]) === fileURLToPath(import.meta.url)
if (invokedDirectly) {
  process.exitCode = await main()
}
