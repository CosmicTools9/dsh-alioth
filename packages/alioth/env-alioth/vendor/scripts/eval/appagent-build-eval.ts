#!/usr/bin/env bun
/**
 * appagent-build-eval.ts — AppAgent 端到端构建回归评测 CLI
 * （openspec change: add-appagent-runtime-verify-eval-and-rollback, capability
 *  `appagent-build-eval-baseline`）
 *
 * 评测 `Meta/backend/app-agent/eval/cases.yaml` 基准任务集：**机械证据为主**
 * （扩展声明验证 / e2e 报告 / 独立闭包裁决 / 规则评估面 / 产物可解析性），LLM rubric
 * 仅作次级加权项（默认权重 0）——**禁止以 LLM 自评作为唯一判据**。
 *
 * 约束：
 * - 禁止 Python（NO_PYTHON_FOR_PROJECT_TOOLS）；YAML 解析用 `yq`（NO_REGEX 工具表正解），
 *   JSON 用原生 `JSON.parse`；全脚本零正则模拟解析
 * - 证据缺失记 0 分（沿 `appagent-evaluation` 诚实性纪律），MUST NOT 按满分或按通过处理
 * - 降级（未执行面 / 环境不可达）≠ 通过：run MUST NOT 覆盖基线
 * - 回归不自动应用任何修订（CONTRACT.md §3）：只投递 `plans/` 修复意图，落地由人工门禁
 *
 * Usage:
 *   bun scripts/eval/appagent-build-eval.ts validate [--root <dir>]
 *   bun scripts/eval/appagent-build-eval.ts run [--case <id>]... [--build] [--gate]
 *                                                 [--baseline <path>] [--write-baseline]
 *                                                 [--rubric <file>] [--root <dir>]
 *   bun scripts/eval/appagent-build-eval.ts compare <baseline.json> [--root <dir>]
 *
 * Exit codes:
 *   0  通过（overall >= threshold 且无回归，或未 --gate）
 *   1  --gate 未达标 / 检出回归（单 case 降幅超容差）
 *   2  任务集非法（拒绝执行）
 *   4  degraded（证据缺失 / 环境不可达 / rubric 未执行；≠ 通过）
 */
import { spawnSync } from 'child_process';
import { createHash } from 'crypto';
import {
  existsSync,
  mkdirSync,
  readFileSync,
  readdirSync,
  writeFileSync,
  statSync,
} from 'fs';
import { join, resolve } from 'path';

import { UnreachableError, login, runTurn } from './appagent-client';

const SCHEMA = 'appagent-build-eval/v1';
/**
 * 维度白名单（非法键 → 拒绝执行，防「自定义维度」绕过证据面）。
 * 静态字符串键查表用 `Record`（ts-set-map）；成员判定 `d in KNOWN_DIMENSIONS`。
 */
const KNOWN_DIMENSIONS: Record<string, true> = {
  extension_verify: true,
  e2e: true,
  closure_audit: true,
  eval_report_rules: true,
  artifacts: true,
};
export type Dimension = keyof typeof KNOWN_DIMENSIONS;

// ── CLI 参数 ────────────────────────────────────────────────────────────────

const argv = process.argv.slice(2);
const cmd = argv[0];

function flag(name: string): string | undefined {
  const i = argv.indexOf(`--${name}`);
  return i >= 0 ? (argv[i + 1] ?? '') : undefined;
}
function multiFlag(name: string): string[] {
  const out: string[] = [];
  for (let i = 0; i < argv.length; i++) {
    if (argv[i] === `--${name}` && argv[i + 1] !== undefined) out.push(argv[i + 1]!);
  }
  return out;
}
const ROOT = resolve(flag('root') ?? process.env.PROJECT_ROOT ?? resolve(import.meta.dirname, '..', '..'));
const CASES_PATH = join(ROOT, 'Meta', 'backend', 'app-agent', 'eval', 'cases.yaml');
const DEFAULT_BASELINE = join(ROOT, 'Meta', 'backend', 'app-agent', 'eval', 'baseline.json');
const GATE = argv.includes('--gate');
const BUILD = argv.includes('--build');
const WRITE_BASELINE = argv.includes('--write-baseline');
const CASE_FILTER = multiFlag('case');
const RUBRIC_PATH = flag('rubric');
const BASELINE_PATH = resolve(flag('baseline') ?? DEFAULT_BASELINE);

// ── 类型 ────────────────────────────────────────────────────────────────────

type Case = {
  id: string;
  namespace: string;
  app: string;
  goal: string;
  weight: number;
  rubric_weight: number;
  dimensions: Partial<Record<Dimension, number>>;
};
type TaskSet = { schema: string; threshold: number; tolerance: number; cases: Case[] };

type DimResult = {
  dimension: Dimension;
  status: 'pass' | 'fail' | 'degraded' | 'missing';
  score: 0 | 1;
  detail: string;
  evidence: string | null;
};
type CaseResult = {
  id: string;
  namespace: string;
  app: string;
  weight: number;
  score: number;
  dimensions: DimResult[];
  rubric: { weight: number; score: number | null; source: string };
  degraded: boolean;
  build: { mode: 'evidence' | 'dialog'; status: string; sessionId?: string; elapsedMs?: number } | null;
};
type RunReport = {
  schema: typeof SCHEMA;
  generated_at: string;
  root: string;
  threshold: number;
  tolerance: number;
  overall_score: number;
  degraded: boolean;
  cases: CaseResult[];
};

// ── YAML（yq → JSON）────────────────────────────────────────────────────────

function loadYamlFile(path: string): unknown {
  const result = spawnSync('yq', ['eval', '-o=json', path], { encoding: 'utf8' });
  if (result.status !== 0) {
    throw new Error(`yq 解析失败 ${path}: ${(result.stderr || '').trim()}`);
  }
  return JSON.parse(result.stdout);
}

// ── 任务集校验（非法即拒绝执行，exit 2）─────────────────────────────────────

function loadTaskSet(): TaskSet {
  if (!existsSync(CASES_PATH)) {
    throw new RejectError(`基准任务集缺失：${CASES_PATH}（先创建 cases.yaml）`);
  }
  const raw = loadYamlFile(CASES_PATH) as TaskSet;
  if (!raw || typeof raw !== 'object') throw new RejectError('cases.yaml 结构非法');
  if (raw.schema !== SCHEMA) {
    throw new RejectError(`cases.yaml schema 非法：${String(raw.schema)} != ${SCHEMA}`);
  }
  if (!Array.isArray(raw.cases) || raw.cases.length === 0) {
    throw new RejectError('cases.yaml 缺 cases 数组或为空');
  }
  if (raw.cases.length < 5 || raw.cases.length > 15) {
    throw new RejectError(`case 数须在 5–15（当前 ${raw.cases.length}）`);
  }
  if (!(raw.threshold > 0 && raw.threshold <= 1)) {
    throw new RejectError(`threshold 非法：${raw.threshold}`);
  }
  if (!(raw.tolerance >= 0)) throw new RejectError(`tolerance 非法：${raw.tolerance}`);

  const seen = new Set<string>();
  for (const c of raw.cases) {
    for (const f of ['id', 'namespace', 'app', 'goal'] as const) {
      if (!c[f] || typeof c[f] !== 'string') throw new RejectError(`case 缺字段 ${f}: ${JSON.stringify(c)}`);
    }
    if (seen.has(c.id)) throw new RejectError(`case id 重复：${c.id}`);
    seen.add(c.id);
    if (!(c.weight > 0)) throw new RejectError(`case ${c.id} weight 非法：${c.weight}`);
    if (typeof c.rubric_weight !== 'number' || c.rubric_weight < 0 || c.rubric_weight > 1) {
      throw new RejectError(`case ${c.id} rubric_weight 须在 [0,1]：${c.rubric_weight}`);
    }
    const dims = Object.keys(c.dimensions ?? {});
    if (dims.length === 0) throw new RejectError(`case ${c.id} 未声明 dimensions`);
    for (const d of dims) {
      if (!(d in KNOWN_DIMENSIONS)) {
        throw new RejectError(
          `case ${c.id} 含非法维度 '${d}'（白名单：${Object.keys(KNOWN_DIMENSIONS).join(', ')}）`,
        );
      }
    }
    const sum = dims.reduce((s, d) => s + (c.dimensions[d as Dimension] ?? 0), 0);
    if (Math.abs(sum - 1.0) > 1e-6) {
      throw new RejectError(`case ${c.id} dimensions 权重合计 ${sum} ≠ 1.0`);
    }
  }
  const total = raw.cases.reduce((s, c) => s + c.weight, 0);
  if (Math.abs(total - 1.0) > 1e-6) {
    throw new RejectError(`cases weight 合计 ${total} ≠ 1.0`);
  }
  return raw;
}

class RejectError extends Error {}

// ── 证据提取（机械面；缺失记 0）────────────────────────────────────────────

function appDir(c: Case): string {
  return join(ROOT, 'Pre-Proc', c.namespace, 'Apps', c.app);
}
function readJson(path: string): unknown | null {
  try {
    return JSON.parse(readFileSync(path, 'utf8'));
  } catch {
    return null;
  }
}

function dimExtensionVerify(c: Case): DimResult {
  const dir = appDir(c);
  const canonical = join(dir, 'extension-verify.json');
  const degraded = join(dir, 'extension-verify.degraded.json');
  const doc = readJson(canonical) as { verdict?: string } | null;
  if (doc) {
    const verdict = String(doc.verdict ?? '');
    return {
      dimension: 'extension_verify',
      status: verdict === 'passed' ? 'pass' : verdict === 'failed' ? 'fail' : 'degraded',
      score: verdict === 'passed' ? 1 : 0,
      detail: `extension-verify.json verdict=${verdict}`,
      evidence: canonical,
    };
  }
  if (existsSync(degraded)) {
    return {
      dimension: 'extension_verify',
      status: 'degraded',
      score: 0,
      detail: '仅存在 extension-verify.degraded.json（验证面未执行完）',
      evidence: degraded,
    };
  }
  return {
    dimension: 'extension_verify',
    status: 'missing',
    score: 0,
    detail: '未找到 extension-verify.json（该层未执行）',
    evidence: null,
  };
}

function dimE2e(c: Case): DimResult {
  const dir = appDir(c);
  const canonical = join(dir, 'e2e-report.json');
  const doc = readJson(canonical) as { passed?: boolean; status?: string } | null;
  if (doc) {
    const passed = doc.passed === true && doc.status !== 'degraded';
    return {
      dimension: 'e2e',
      status: passed ? 'pass' : doc.status === 'degraded' ? 'degraded' : 'fail',
      score: passed ? 1 : 0,
      detail: `e2e-report.json passed=${String(doc.passed)} status=${String(doc.status)}`,
      evidence: canonical,
    };
  }
  const degraded = join(dir, 'e2e-report.degraded.json');
  if (existsSync(degraded)) {
    return {
      dimension: 'e2e',
      status: 'degraded',
      score: 0,
      detail: '仅存在 e2e-report.degraded.json（浏览器层未执行）',
      evidence: degraded,
    };
  }
  return { dimension: 'e2e', status: 'missing', score: 0, detail: '未找到 e2e-report.json', evidence: null };
}

function dimClosureAudit(c: Case): DimResult {
  // 裁决产出根 = `Pre-Proc/{ns}/AppAgentTraces`（运行期唯一写入者
  // `app_agent::closure_audit::audit_dir` / `trace::traces_dir`；文档正本 META_AI_SPEC §562、§189）。
  // 评测运行轨迹目录（`{ROOT}/AppAgentTraces/eval/`）是本脚本自有产物，与裁决根不同层。
  const dir = join(ROOT, 'Pre-Proc', c.namespace, 'AppAgentTraces', 'closure-audit', c.app);
  if (!existsSync(dir)) {
    return { dimension: 'closure_audit', status: 'missing', score: 0, detail: '无闭包裁决记录', evidence: null };
  }
  const files = readdirSync(dir)
    .filter((f) => f.endsWith('.json'))
    .sort();
  const latest = files[files.length - 1];
  if (!latest) {
    return { dimension: 'closure_audit', status: 'missing', score: 0, detail: '闭包裁决目录为空', evidence: null };
  }
  const path = join(dir, latest);
  const doc = readJson(path) as { verdict?: string } | null;
  const verdict = String(doc?.verdict ?? '');
  return {
    dimension: 'closure_audit',
    status: verdict === 'approved' ? 'pass' : 'fail',
    score: verdict === 'approved' ? 1 : 0,
    detail: `closure-audit ${latest} verdict=${verdict || '(unparsable)'}`,
    evidence: path,
  };
}

function dimEvalReportRules(c: Case): DimResult {
  const path = join(appDir(c), 'eval-report.json');
  const doc = readJson(path) as { violations?: unknown[]; score?: number } | null;
  if (!doc) {
    return { dimension: 'eval_report_rules', status: 'missing', score: 0, detail: '未找到可解析的 eval-report.json', evidence: null };
  }
  if (Array.isArray(doc.violations)) {
    const ok = doc.violations.length === 0;
    return {
      dimension: 'eval_report_rules',
      status: ok ? 'pass' : 'fail',
      score: ok ? 1 : 0,
      detail: `violations=${doc.violations.length}`,
      evidence: path,
    };
  }
  if (typeof doc.score === 'number') {
    const ok = doc.score >= 0.6;
    return {
      dimension: 'eval_report_rules',
      status: ok ? 'pass' : 'fail',
      score: ok ? 1 : 0,
      detail: `score=${doc.score}`,
      evidence: path,
    };
  }
  return {
    dimension: 'eval_report_rules',
    status: 'missing',
    score: 0,
    detail: 'eval-report.json 形态未知（无 violations 数组也无 score）——按缺失记 0',
    evidence: path,
  };
}

function dimArtifacts(c: Case): DimResult {
  const path = join(appDir(c), 'app.json');
  const doc = readJson(path) as { config?: { modules?: unknown[] } } | null;
  if (!doc) {
    return { dimension: 'artifacts', status: 'missing', score: 0, detail: 'app.json 缺失或不可解析', evidence: null };
  }
  const modules = doc.config?.modules;
  const ok = Array.isArray(modules) && modules.length > 0;
  return {
    dimension: 'artifacts',
    status: ok ? 'pass' : 'fail',
    score: ok ? 1 : 0,
    detail: `app.json 可解析，config.modules=${Array.isArray(modules) ? modules.length : 'n/a'}`,
    evidence: path,
  };
}

const EXTRACTORS: Record<Dimension, (c: Case) => DimResult> = {
  extension_verify: dimExtensionVerify,
  e2e: dimE2e,
  closure_audit: dimClosureAudit,
  eval_report_rules: dimEvalReportRules,
  artifacts: dimArtifacts,
};

// ── 评分 ────────────────────────────────────────────────────────────────────

function scoreCase(c: Case, rubric: Record<string, number> | null): CaseResult {
  const dims = (Object.keys(c.dimensions) as Dimension[]).map((d) => EXTRACTORS[d](c));
  const mechanical = dims.reduce((s, r) => s + (c.dimensions[r.dimension] ?? 0) * r.score, 0);
  const rw = c.rubric_weight ?? 0;
  const rubricScore = rubric?.[c.id] ?? null;
  const score = rw > 0 && rubricScore !== null ? mechanical * (1 - rw) + rw * rubricScore : mechanical;
  const degraded =
    dims.some((r) => r.status === 'degraded' || r.status === 'missing') || (rw > 0 && rubricScore === null);
  return {
    id: c.id,
    namespace: c.namespace,
    app: c.app,
    weight: c.weight,
    score,
    dimensions: dims,
    rubric: { weight: rw, score: rubricScore, source: rubricScore === null ? 'not_executed' : 'provided' },
    degraded,
    build: null,
  };
}

// ── plans/ 投递（复用既有队列通道）──────────────────────────────────────────

function failuresFingerprint(parts: string[]): string {
  return createHash('sha256').update(parts.sort().join('\n')).digest('hex').slice(0, 16);
}

function enqueuePlan(c: Case, source: string, title: string, body: string, fingerprint: string): string | null {
  const dir = join(appDir(c), 'plans');
  mkdirSync(dir, { recursive: true });
  // 同指纹去重：既有 pending/picked 同 hash → 不重复新建
  for (const f of readdirSync(dir)) {
    if (!f.endsWith('.md') || !f.includes(source)) continue;
    const text = readFileSync(join(dir, f), 'utf8');
    if (text.includes(`failures_hash: ${fingerprint}`) && !text.includes('status: done')) return null;
  }
  const ts = new Date().toISOString().replace(/[:.]/g, '-');
  const file = join(dir, `${ts}-${source}.md`);
  const frontmatter = [
    '---',
    'priority: normal',
    `source: ${source}`,
    'status: pending',
    `title: ${title}`,
    `failures_hash: ${fingerprint}`,
    '---',
    '',
  ].join('\n');
  const selfResolve = [
    '## 自解决条款',
    '',
    `拾取方先读本 app 的证据产物：若对应面已转通过（如 \`extension-verify.json\` verdict=passed /`,
    '`e2e-report.json` passed=true），则在沙箱内经显式写入把本项标 `done`，**不得重复修复**。',
    '',
    '## 约束',
    '',
    '- 禁止修改断言/门禁来「修好」评测：证据面（extension-verify / e2e-report / closure-audit）',
    '  由独立机制产出，改判据 = 造假。',
    '',
  ].join('\n');
  writeFileSync(file, `${frontmatter}${title}\n\n${body}\n\n${selfResolve}`);
  return file;
}

// ── run ─────────────────────────────────────────────────────────────────────

async function cmdRun(): Promise<number> {
  let taskSet: TaskSet;
  try {
    taskSet = loadTaskSet();
  } catch (e) {
    if (e instanceof RejectError) {
      console.error(`❌ ${e.message}`);
      return 2;
    }
    throw e;
  }

  const rubric = RUBRIC_PATH ? (JSON.parse(readFileSync(resolve(RUBRIC_PATH), 'utf8')) as Record<string, number>) : null;
  const selected = CASE_FILTER.length
    ? taskSet.cases.filter((c) => CASE_FILTER.includes(c.id))
    : taskSet.cases;
  if (selected.length === 0) {
    console.error(`❌ --case 未匹配任何基准项：${CASE_FILTER.join(', ')}`);
    return 2;
  }

  const results: CaseResult[] = [];
  for (const c of selected) {
    if (BUILD) {
      try {
        const { sessionId, outcome } = await runTurn(
          await login(),
          c.namespace,
          `eval:${c.id}:${Date.now()}`,
          c.goal,
          Number(process.env.EVAL_TURN_TIMEOUT_MS ?? 900_000),
        );
        const r = scoreCase(c, rubric);
        r.build = { mode: 'dialog', status: outcome.status, sessionId, elapsedMs: outcome.elapsedMs };
        if (outcome.status !== 'succeeded') r.degraded = true;
        results.push(r);
      } catch (e) {
        if (e instanceof UnreachableError) {
          const r = scoreCase(c, rubric);
          r.degraded = true;
          r.build = { mode: 'dialog', status: 'unreachable' };
          r.dimensions.push({
            dimension: 'artifacts',
            status: 'degraded',
            score: 0,
            detail: `环境不可达：${e.message}`,
            evidence: null,
          });
          results.push(r);
          continue;
        }
        throw e;
      }
    } else {
      const r = scoreCase(c, rubric);
      r.build = { mode: 'evidence', status: 'not_driven' };
      results.push(r);
    }
  }

  const weightSum = results.reduce((s, r) => s + r.weight, 0);
  const overall = weightSum > 0 ? results.reduce((s, r) => s + r.weight * r.score, 0) / weightSum : 0;
  const degraded = results.some((r) => r.degraded);
  const report: RunReport = {
    schema: SCHEMA,
    generated_at: new Date().toISOString(),
    root: ROOT,
    threshold: taskSet.threshold,
    tolerance: taskSet.tolerance,
    overall_score: overall,
    degraded,
    cases: results,
  };

  const runDir = join(ROOT, 'AppAgentTraces', 'eval', new Date().toISOString().replace(/[:.]/g, '-'));
  mkdirSync(runDir, { recursive: true });
  writeFileSync(join(runDir, 'scores.json'), JSON.stringify(report, null, 2));
  writeFileSync(join(runDir, 'report.md'), renderMarkdown(report));

  // 失败/回归 → plans/ 修复意图（不自动改任何产物）
  let enqueued = 0;
  for (const r of results.filter((x) => x.score < 1)) {
    const c = selected.find((x) => x.id === r.id)!;
    const failed = r.dimensions.filter((d) => d.score === 0).map((d) => `${d.dimension}:${d.status}`);
    const fp = failuresFingerprint([r.id, ...failed]);
    const path = enqueuePlan(
      c,
      'eval-regression',
      `评测回归：${r.id}（score=${r.score.toFixed(2)}）`,
      `## 失败维度\n\n${failed.map((f) => `- ${f}`).join('\n')}\n\n证据目录：\`${runDir}\``,
      fp,
    );
    if (path) enqueued++;
  }

  if (degraded) {
    writeFileSync(
      join(runDir, 'degraded.json'),
      JSON.stringify(
        {
          schema: SCHEMA,
          reasons: results
            .filter((r) => r.degraded)
            .flatMap((r) => r.dimensions.filter((d) => d.status !== 'pass').map((d) => `${r.id} → ${d.dimension}: ${d.status} (${d.detail})`)),
          note: 'degraded ≠ 通过：本次不写基线；重跑环境恢复后再评',
        },
        null,
        2,
      ),
    );
    for (const r of results.filter((x) => x.degraded)) {
      const c = selected.find((x) => x.id === r.id)!;
      const fx = failuresFingerprint(['degraded', r.id]);
      enqueuePlan(
        c,
        'eval-degraded',
        `评测降级（未执行面）：${r.id}`,
        '## 未执行面\n\n' +
          r.dimensions
            .filter((d) => d.status !== 'pass')
            .map((d) => `- ${d.dimension}: ${d.status} — ${d.detail}`)
            .join('\n') +
          `\n\n证据目录：\`${runDir}\``,
        fx,
      );
    }
    console.error(
      `⚠️  degraded（未执行面，≠ 通过）：${results.filter((r) => r.degraded).length} 个 case 存在缺失/不可达证据；基线未覆盖`,
    );
  }

  if (WRITE_BASELINE) {
    if (degraded) {
      console.error('❌ 拒绝写基线：本次为 degraded（未执行面）——基线不得由降级运行覆盖');
      return 4;
    }
    writeFileSync(
      BASELINE_PATH,
      JSON.stringify(
        {
          schema: SCHEMA,
          generated_at: new Date().toISOString(),
          threshold: taskSet.threshold,
          tolerance: taskSet.tolerance,
          overall_score: overall,
          cases: Object.fromEntries(results.map((r) => [r.id, r.score])),
        },
        null,
        2,
      ),
    );
    console.log(`✅ 基线已写：${BASELINE_PATH}`);
  }

  console.log(`评测运行 ${runDir}`);
  console.log(`overall=${overall.toFixed(3)} threshold=${taskSet.threshold} degraded=${degraded} 修复意图入队=${enqueued}`);
  for (const r of results) {
    console.log(`  ${r.score.toFixed(2)}  ${r.id}  (${r.namespace}/${r.app})  degraded=${r.degraded}`);
  }

  if (degraded) return 4;
  if (!GATE) return 0;

  const verdict = gateVerdict(report, existsSync(BASELINE_PATH) ? BASELINE_PATH : null);
  if (verdict.regressed) {
    console.error(`❌ 检出回归：${verdict.reasons.join('; ')}`);
    return 1;
  }
  console.log('✅ gate 通过');
  return 0;
}

function gateVerdict(report: RunReport, baselinePath: string | null): { regressed: boolean; reasons: string[] } {
  const reasons: string[] = [];
  if (report.overall_score < report.threshold) {
    reasons.push(`overall ${report.overall_score.toFixed(3)} < threshold ${report.threshold}`);
  }
  if (baselinePath) {
    const b = JSON.parse(readFileSync(baselinePath, 'utf8')) as { cases: Record<string, number>; tolerance?: number };
    const tol = report.tolerance ?? b.tolerance ?? 0;
    for (const c of report.cases) {
      const prev = b.cases[c.id];
      if (prev === undefined) continue;
      const drop = prev - c.score;
      if (drop > tol) reasons.push(`case ${c.id} 降幅 ${drop.toFixed(2)} > tolerance ${tol}`);
    }
  }
  return { regressed: reasons.length > 0, reasons };
}

function renderMarkdown(r: RunReport): string {
  const lines = [
    `# AppAgent 构建回归评测（${r.generated_at}）`,
    '',
    `- overall: **${r.overall_score.toFixed(3)}** / threshold ${r.threshold}`,
    `- degraded: ${r.degraded}`,
    '',
    '| case | app | score | weight | degraded | 维度 |',
    '| --- | --- | --- | --- | --- | --- |',
  ];
  for (const c of r.cases) {
    const dims = c.dimensions.map((d) => `${d.dimension}=${d.status}`).join(' ');
    lines.push(`| ${c.id} | ${c.namespace}/${c.app} | ${c.score.toFixed(2)} | ${c.weight} | ${c.degraded} | ${dims} |`);
  }
  lines.push('', '## 证据', '');
  for (const c of r.cases) {
    lines.push(`### ${c.id}`, '');
    for (const d of c.dimensions) {
      lines.push(`- ${d.dimension}: **${d.status}** — ${d.detail}${d.evidence ? `（\`${d.evidence}\`）` : ''}`);
    }
    lines.push('');
  }
  return lines.join('\n');
}

// ── compare ─────────────────────────────────────────────────────────────────

function cmdCompare(baselineArg: string): number {
  const baseline = JSON.parse(readFileSync(resolve(baselineArg), 'utf8')) as {
    cases: Record<string, number>;
    overall_score: number;
    tolerance?: number;
  };
  const runsDir = join(ROOT, 'AppAgentTraces', 'eval');
  if (!existsSync(runsDir)) {
    console.error('❌ 无运行轨迹（AppAgentTraces/eval/）');
    return 2;
  }
  const runs = readdirSync(runsDir).filter((d) => existsSync(join(runsDir, d, 'scores.json'))).sort();
  const latest = runs[runs.length - 1];
  if (!latest) {
    console.error('❌ 无可用运行轨迹');
    return 2;
  }
  const cur = JSON.parse(readFileSync(join(runsDir, latest, 'scores.json'), 'utf8')) as RunReport;
  const tol = cur.tolerance ?? baseline.tolerance ?? 0;
  console.log(`compare 基线 ${resolve(baselineArg)} ↔ 运行 ${latest}`);
  console.log(`overall ${baseline.overall_score.toFixed(3)} → ${cur.overall_score.toFixed(3)}`);
  let regressed = false;
  for (const c of cur.cases) {
    const prev = baseline.cases[c.id];
    if (prev === undefined) {
      console.log(`  + ${c.id}（新增，无基线）`);
      continue;
    }
    const delta = c.score - prev;
    const flag = prev - c.score > tol ? ' ← 回归' : '';
    if (flag) regressed = true;
    console.log(`  ${delta >= 0 ? '+' : ''}${delta.toFixed(2)}  ${c.id}（${prev.toFixed(2)} → ${c.score.toFixed(2)}）${flag}`);
  }
  return regressed ? 1 : 0;
}

// ── main ────────────────────────────────────────────────────────────────────

switch (cmd) {
  case 'validate': {
    try {
      const t = loadTaskSet();
      console.log(`✅ 任务集合法：${t.cases.length} cases，threshold=${t.threshold}，tolerance=${t.tolerance}`);
      process.exit(0);
    } catch (e) {
      if (e instanceof RejectError) {
        console.error(`❌ ${e.message}`);
        process.exit(2);
      }
      throw e;
    }
    break;
  }
  case 'run':
    process.exit(await cmdRun());
    break;
  case 'compare': {
    const arg = argv[1];
    if (!arg || arg.startsWith('--')) {
      console.error('用法：compare <baseline.json> [--root <dir>]');
      process.exit(2);
    }
    process.exit(cmdCompare(arg));
    break;
  }
  default:
    console.error(
      '用法：validate | run [--case <id>]... [--build] [--gate] [--write-baseline] [--rubric <f>] [--root <dir>] | compare <baseline.json>',
    );
    process.exit(2);
}
