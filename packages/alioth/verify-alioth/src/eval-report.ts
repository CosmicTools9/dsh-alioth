/**
 * `verify_artifacts` 产物（eval-report）—— 对齐上游 AppAgent `dialog_tools/verify_artifacts.rs`：
 * 仅跑**规则维度**，落盘 `{appDir}/eval-report.json`（quality stage 的声明产物）。
 *
 * 契约（诚实评分纪律，MUST NOT 软化）：
 * - 产物缺失 / 不可解析 → 该维度记 0 分**并**记一条 violation（缺失 ≠ 满分）；
 * - 报告以 `evaluated_dimensions` 标注**实际评估面**，MUST NOT 冒充完整 rubric；
 * - 解析一律走真解析器（`JSON.parse`），MUST NOT 用正则模拟解析。
 * @module @dsh-alioth/verify-alioth/eval-report
 */

import { access, mkdir, readFile, rename, writeFile } from 'node:fs/promises'
import path from 'node:path'

export const EVAL_REPORT_SCHEMA_VERSION = '1.0'

/**
 * 通过阈值（上游 `evaluate.rs:22` `THRESHOLD = 0.8`）。
 * 本库只评估 2 个规则维度（权重按 `evaluated_dimensions` 归一化后各 0.5），
 * 故 0.8 阈值等价于「两个维度都须满分」——缺失面不得靠别的维度抬升为通过。
 */
const PASS_THRESHOLD = 0.8

/** 单个规则违规（`data.violations` 元素，供 LLM 判读）。 */
export interface EvalViolation {
  readonly rule: string
  readonly severity: 'error' | 'warning'
  readonly message: string
  readonly target: string
}

/** `eval-report.json` 文档形态。 */
export interface EvalReport {
  readonly schema_version: string
  readonly app: string
  readonly namespace: string
  readonly passed: boolean
  readonly dimensions: Record<string, number>
  readonly evaluated_dimensions: readonly string[]
  readonly violations: readonly EvalViolation[]
  readonly note: string
  readonly ts: string
}

/** 本库评估的规则维度（顺序即 `evaluated_dimensions` 顺序）。 */
export const EVAL_REPORT_DIMENSIONS: readonly string[] = ['schema_validity', 'prototype_standalone']

/** app.json 必填字段（上游 `assess_schema_validity` 的 required 集）。 */
const APP_REQUIRED_FIELDS: readonly string[] = ['id', 'code', 'namespace', 'name', 'version', 'status']

const APP_STATUS_ENUM: readonly string[] = ['developing', 'active', 'deprecated', 'archived']
const DEPLOYMENT_MODE_ENUM: readonly string[] = ['standalone', 'embedded']

type JsonProbe =
  | { readonly kind: 'ok'; readonly value: unknown }
  | { readonly kind: 'missing' }
  | { readonly kind: 'unparsable'; readonly reason: string }

/** 读 + 解析 JSON：缺失与不可解析**分态返回**（调用方必须分态记 violation，不得合并为「无内容」）。 */
async function probeJson(file: string): Promise<JsonProbe> {
  let text: string
  try {
    text = await readFile(file, 'utf8')
  } catch {
    return { kind: 'missing' }
  }
  try {
    return { kind: 'ok', value: JSON.parse(text) as unknown }
  } catch (error) {
    return { kind: 'unparsable', reason: error instanceof Error ? error.message : String(error) }
  }
}

/** 定位 `prototype.html`：app 目录内优先，其次上游 Composing 布局 `Pre-Proc/{ns}/Prototypes/Apps/{app}/`。 */
async function resolvePrototypeHtml(appDir: string, app: string): Promise<string | null> {
  const preProcRoot = path.dirname(path.dirname(path.resolve(appDir)))
  const candidates = [
    path.join(path.resolve(appDir), 'prototype.html'),
    path.join(preProcRoot, 'Prototypes', 'Apps', app, 'prototype.html'),
  ]
  for (const candidate of candidates) {
    const present = await access(candidate).then(
      () => true,
      () => false,
    )
    if (present) return candidate
  }
  return null
}

/**
 * 字符级资源引用扫描器（上游 `extension_verify::identifiers` 同族纪律：
 * 字符级扫描，非正则、非解析器）。提取 `src="…"` / `href="…"` 属性值（单双引号皆可）。
 */
const HTML_SPACE_CHARS: Record<string, true> = { ' ': true, '\t': true, '\n': true, '\r': true }

function scanResourceReferences(html: string): readonly string[] {
  const refs: string[] = []
  let i = 0
  while (i < html.length) {
    const ch = html[i]
    if (ch === undefined) break
    const attr = ch === 's' ? 'src' : ch === 'h' ? 'href' : undefined
    if (attr === undefined || !html.startsWith(attr, i)) {
      i += 1
      continue
    }
    let j = i + attr.length
    while (HTML_SPACE_CHARS[html[j] ?? ''] === true) j += 1
    if (html[j] !== '=') {
      i += attr.length
      continue
    }
    j += 1
    while (HTML_SPACE_CHARS[html[j] ?? ''] === true) j += 1
    const quote = html[j]
    if (quote !== '"' && quote !== "'") {
      i = j
      continue
    }
    const end = html.indexOf(quote, j + 1)
    if (end < 0) break
    refs.push(html.slice(j + 1, end))
    i = end + 1
  }
  return refs
}

/** app.json 的规则违规（空数组 = 合规）。 */
function schemaIssues(app: Record<string, unknown>): readonly string[] {
  const issues: string[] = []
  for (const field of APP_REQUIRED_FIELDS) {
    if (app[field] === undefined || app[field] === null) issues.push(`缺少必填字段: ${field}`)
  }
  const code = app['code']
  if (typeof code === 'string' && !/^[a-z][a-z0-9-]*$/.test(code)) {
    issues.push('code 非法 pattern（需 ^[a-z][a-z0-9-]*$）')
  }
  const namespace = app['namespace']
  if (typeof namespace === 'string' && !/^[A-Z][a-zA-Z0-9-]*$/.test(namespace)) {
    issues.push('namespace 非法 pattern（需 ^[A-Z][a-zA-Z0-9-]*$）')
  }
  for (const field of ['version', 'min_alioth_version'] as const) {
    const value = app[field]
    if (typeof value === 'string' && !/^\d+\.\d+\.\d+$/.test(value)) {
      issues.push(`${field} 非法 pattern（需 ^\\d+\\.\\d+\\.\\d+$）`)
    }
  }
  const status = app['status']
  if (typeof status !== 'string' || !APP_STATUS_ENUM.includes(status)) {
    issues.push(`status 非法枚举（schema enum: ${APP_STATUS_ENUM.join('/')}）`)
  }
  const deploymentMode = app['deploymentMode']
  if (
    deploymentMode !== undefined &&
    deploymentMode !== null &&
    (typeof deploymentMode !== 'string' || !DEPLOYMENT_MODE_ENUM.includes(deploymentMode))
  ) {
    issues.push(`deploymentMode 非法（schema enum: ${DEPLOYMENT_MODE_ENUM.join('/')}）`)
  }
  return issues
}

/**
 * 构建评估报告。**诚实评分**：任一产物缺失/不可解析 → 该维度 0 分并记 violation。
 */
export async function buildEvalReport(input: {
  readonly app: string
  readonly namespace: string
  readonly appDir: string
}): Promise<EvalReport> {
  const appDir = path.resolve(input.appDir)
  const violations: EvalViolation[] = []
  const dimensions: Record<string, number> = {}

  // ── 维度 1：schema_validity（app.json 可解析 + 必需字段/模式/枚举）──
  const appJsonPath = path.join(appDir, 'app.json')
  const probe = await probeJson(appJsonPath)
  let schemaScore = 0
  if (probe.kind === 'missing') {
    violations.push({
      rule: 'schema_validity',
      severity: 'error',
      message: 'app.json 缺失：无法判定 schema（缺失记 0 分，MUST NOT 按满分处理）',
      target: appJsonPath,
    })
  } else if (probe.kind === 'unparsable') {
    violations.push({
      rule: 'schema_validity',
      severity: 'error',
      message: `app.json 不可解析：${probe.reason}`,
      target: appJsonPath,
    })
  } else if (typeof probe.value !== 'object' || probe.value === null || Array.isArray(probe.value)) {
    violations.push({
      rule: 'schema_validity',
      severity: 'error',
      message: `app.json 顶层非对象（${Array.isArray(probe.value) ? 'array' : typeof probe.value}）`,
      target: appJsonPath,
    })
  } else {
    const issues = schemaIssues(probe.value as Record<string, unknown>)
    if (issues.length === 0) {
      schemaScore = 1
    } else {
      for (const issue of issues) {
        violations.push({ rule: 'schema_validity', severity: 'error', message: issue, target: appJsonPath })
      }
    }
  }
  dimensions['schema_validity'] = schemaScore

  // ── 维度 2：prototype_standalone（prototype.html 存在且无外部 CDN / 越界资源）──
  const prototypePath = await resolvePrototypeHtml(appDir, input.app)
  let prototypeScore = 0
  if (prototypePath === null) {
    violations.push({
      rule: 'prototype_standalone',
      severity: 'error',
      message:
        'prototype.html 缺失（查过 app 目录与 Prototypes/Apps/{app}/）：standalone 面无从判定（缺失记 0 分）',
      target: path.join(appDir, 'prototype.html'),
    })
  } else {
    const html = await readFile(prototypePath, 'utf8').catch(() => null)
    if (html === null) {
      violations.push({
        rule: 'prototype_standalone',
        severity: 'error',
        message: 'prototype.html 不可读',
        target: prototypePath,
      })
    } else if (html.trim() === '') {
      violations.push({
        rule: 'prototype_standalone',
        severity: 'error',
        message: 'prototype.html 为空文件',
        target: prototypePath,
      })
    } else {
      const external: string[] = []
      const escaping: string[] = []
      for (const ref of scanResourceReferences(html)) {
        const trimmed = ref.trim()
        const lowered = trimmed.toLowerCase()
        if (lowered.startsWith('http://') || lowered.startsWith('https://') || lowered.startsWith('//')) {
          external.push(trimmed)
          continue
        }
        if (trimmed.startsWith('/') || trimmed.split(/[\\/]+/).includes('..')) escaping.push(trimmed)
      }
      for (const ref of external) {
        violations.push({
          rule: 'prototype_standalone',
          severity: 'error',
          message: `发现外部资源引用（原型须 standalone，MUST NOT 依赖 CDN）：${ref}`,
          target: prototypePath,
        })
      }
      for (const ref of escaping) {
        violations.push({
          rule: 'prototype_standalone',
          severity: 'error',
          message: `发现越界相对/绝对资源引用（逃出 app 目录）：${ref}`,
          target: prototypePath,
        })
      }
      if (external.length === 0 && escaping.length === 0) prototypeScore = 1
    }
  }
  dimensions['prototype_standalone'] = prototypeScore

  const evaluated = EVAL_REPORT_DIMENSIONS.filter(dimension => dimension in dimensions)
  const weighted =
    evaluated.length === 0
      ? 0
      : evaluated.reduce((acc, dimension) => acc + (dimensions[dimension] ?? 0), 0) / evaluated.length
  const passed = evaluated.length === EVAL_REPORT_DIMENSIONS.length && weighted >= PASS_THRESHOLD

  return {
    schema_version: EVAL_REPORT_SCHEMA_VERSION,
    app: input.app,
    namespace: input.namespace,
    passed,
    dimensions,
    evaluated_dimensions: evaluated,
    violations,
    note: passed
      ? `规则维度通过（evaluated_dimensions=${evaluated.join(',')}，overall=${weighted.toFixed(3)} ≥ ${PASS_THRESHOLD}）`
      : `规则维度未通过（evaluated_dimensions=${evaluated.join(',')}，overall=${weighted.toFixed(3)} < ${PASS_THRESHOLD}）：violations=${violations.length}，缺失面按 0 分计`,
    ts: new Date().toISOString(),
  }
}

/** 落盘 `{appDir}/eval-report.json`（tmp + rename 原子替换），返回落盘路径。 */
export async function writeEvalReport(appDir: string, report: EvalReport): Promise<string> {
  const dir = path.resolve(appDir)
  await mkdir(dir, { recursive: true })
  const target = path.join(dir, 'eval-report.json')
  const tmp = path.join(dir, `.eval-report.json.tmp-${process.pid}`)
  await writeFile(tmp, `${JSON.stringify(report, null, 2)}\n`, 'utf8')
  await rename(tmp, target)
  return target
}
