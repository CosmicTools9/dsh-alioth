#!/usr/bin/env bun
/**
 * appagent-build-eval.selftest.ts — 评测 CLI 自测（含负例）
 *
 * 判据自测（SPEC_ENFORCEMENT_SUBSTRATE R1）：豁免/拒绝类判据 MUST 带负例——
 * 证明「非法输入被拒」「缺证据不按通过」「降级不覆盖基线」诸门未被放宽。
 *
 * 用法：bun scripts/eval/appagent-build-eval.selftest.ts
 * 退出码：0 全绿 / 1 有断言失败
 */
import { spawnSync } from 'child_process';
import {
  existsSync,
  mkdirSync,
  mkdtempSync,
  readFileSync,
  readdirSync,
  rmSync,
  writeFileSync,
} from 'fs';
import { tmpdir } from 'os';
import { join } from 'path';

const RUNNER = join(import.meta.dirname, 'appagent-build-eval.ts');
let failures = 0;

const check = (name: string, cond: boolean, detail = '') => {
  console.log(`${cond ? '✅' : '❌'} ${name}${detail ? ` — ${detail}` : ''}`);
  if (!cond) failures++;
};

const DIMS = { extension_verify: 0.35, e2e: 0.25, closure_audit: 0.2, eval_report_rules: 0.1, artifacts: 0.1 };

const CASES_YAML = (over: Partial<Record<string, string>> = {}) =>
  `schema: appagent-build-eval/v1
threshold: 0.8
tolerance: 0.1
cases:
  - id: c1
    namespace: UT
    app: a1
    goal: g1
    weight: 0.5
    rubric_weight: 0.0
    dimensions: ${over.dims1 ?? JSON.stringify(DIMS)}
  - id: c2
    namespace: UT
    app: a2
    goal: g2
    weight: 0.3
    rubric_weight: 0.0
    dimensions: { extension_verify: 1.0 }
  - id: c3
    namespace: UT
    app: a3
    goal: g3
    weight: 0.1
    rubric_weight: 0.0
    dimensions: { artifacts: 1.0 }
  - id: c4
    namespace: UT
    app: a4
    goal: g4
    weight: 0.06
    rubric_weight: 0.0
    dimensions: { artifacts: 1.0 }
  - id: c5
    namespace: UT
    app: a5
    goal: g5
    weight: 0.04
    rubric_weight: 0.0
    dimensions: { artifacts: 1.0 }
`;

function newRoot(casesYaml: string): string {
  const root = mkdtempSync(join(tmpdir(), 'abe-selftest-'));
  mkdirSync(join(root, 'Meta', 'backend', 'app-agent', 'eval'), { recursive: true });
  writeFileSync(join(root, 'Meta', 'backend', 'app-agent', 'eval', 'cases.yaml'), casesYaml);
  return root;
}

function plantEvidence(root: string, app: string): void {
  const dir = join(root, 'Pre-Proc', 'UT', 'Apps', app);
  mkdirSync(join(dir, 'extensions'), { recursive: true });
  writeFileSync(join(dir, 'app.json'), JSON.stringify({ config: { modules: [{ id: 'm' }] } }));
  writeFileSync(join(dir, 'extension-verify.json'), JSON.stringify({ verdict: 'passed' }));
  writeFileSync(join(dir, 'e2e-report.json'), JSON.stringify({ passed: true, status: 'passed' }));
  writeFileSync(join(dir, 'eval-report.json'), JSON.stringify({ violations: [] }));
  // 与运行期同根：`Pre-Proc/{ns}/AppAgentTraces/closure-audit/{app}`（见 dimClosureAudit 注释）
  const ca = join(root, 'Pre-Proc', 'UT', 'AppAgentTraces', 'closure-audit', app);
  mkdirSync(ca, { recursive: true });
  writeFileSync(join(ca, '0001.json'), JSON.stringify({ verdict: 'approved' }));
}

const run = (args: string[], root: string) =>
  spawnSync('bun', [RUNNER, ...args, '--root', root], { encoding: 'utf8' });

type ScoresDoc = {
  overall_score: number;
  cases: {
    id: string;
    score: number;
    dimensions: { dimension: string; status: string; score: number }[];
  }[];
};

/** 读 fixture 根下最近一次运行的分数文档（结构化断言，不解析 stdout 文本） */
function latestScores(root: string): ScoresDoc | null {
  const dir = join(root, 'AppAgentTraces', 'eval');
  if (!existsSync(dir)) return null;
  const runs = readdirSync(dir)
    .filter((d) => existsSync(join(dir, d, 'scores.json')))
    .sort();
  const latest = runs[runs.length - 1];
  return latest
    ? (JSON.parse(readFileSync(join(dir, latest, 'scores.json'), 'utf8')) as ScoresDoc)
    : null;
}

// ── S1：合法集 + 全证据 → 通过 ──────────────────────────────────────────────
{
  const root = newRoot(CASES_YAML());
  for (const app of ['a1', 'a2', 'a3', 'a4', 'a5']) plantEvidence(root, app);
  const r = run(['run'], root);
  check('S1 全证据 → exit 0', r.status === 0, `exit=${r.status} ${r.stdout.trim()}`);
  check('S1 overall=1.0', r.stdout.includes('overall=1.000'), r.stdout.trim().split('\n').pop());
  rmSync(root, { recursive: true, force: true });
}

// ── S2：负例——缺证据记 0 且整轮 degraded，基线不被覆盖 ─────────────────────
{
  const root = newRoot(CASES_YAML());
  plantEvidence(root, 'a1');
  // a2..a5 无证据
  const r = run(['run', '--write-baseline'], root);
  check('S2 缺证据 → exit 4（degraded，非 0 也非 1）', r.status === 4, `exit=${r.status}`);
  check(
    'S2 degraded.json 落盘',
    existsSync(join(root, 'AppAgentTraces', 'eval')) &&
      readdirSync(join(root, 'AppAgentTraces', 'eval')).some((d) =>
        existsSync(join(root, 'AppAgentTraces', 'eval', d, 'degraded.json')),
      ),
  );
  check(
    'S2 拒绝写基线（降级不得覆盖）',
    !existsSync(join(root, 'Meta', 'backend', 'app-agent', 'eval', 'baseline.json')),
  );
  const s2 = latestScores(root);
  const c3 = s2?.cases.find((c) => c.id === 'c3');
  check(
    'S2 缺证据维度记 0（c3 artifacts missing → 0）',
    c3?.score === 0 && c3.dimensions.some((d) => d.status === 'missing' && d.score === 0),
    `score=${c3?.score} dims=${JSON.stringify(c3?.dimensions.map((d) => [d.dimension, d.status, d.score]))}`,
  );
  rmSync(root, { recursive: true, force: true });
}

// ── S3：负例——非法维度键被拒执行 ───────────────────────────────────────────
{
  const root = newRoot(CASES_YAML({ dims1: '{"not_a_dimension": 1.0}' }));
  const r = run(['validate'], root);
  check('S3 非法维度 → exit 2', r.status === 2, `exit=${r.status} ${r.stderr.trim()}`);
  check('S3 报错指名非法维度', r.stderr.includes('not_a_dimension'), r.stderr.trim());
  rmSync(root, { recursive: true, force: true });
}

// ── S4：负例——维度权重 ≠ 1.0 / case 权重 ≠ 1.0 被拒 ────────────────────────
{
  const root = newRoot(CASES_YAML({ dims1: '{"artifacts": 0.5}' }));
  const r = run(['validate'], root);
  check('S4 维度权重合计≠1 → exit 2', r.status === 2, `exit=${r.status} ${r.stderr.trim()}`);

  const root2 = newRoot(CASES_YAML().replace('weight: 0.5', 'weight: 0.9'));
  const r2 = run(['validate'], root2);
  check('S4 case 权重合计≠1 → exit 2', r2.status === 2, `exit=${r2.status} ${r2.stderr.trim()}`);
  rmSync(root, { recursive: true, force: true });
  rmSync(root2, { recursive: true, force: true });
}

// ── S5：--gate 未达标 → exit 1；对比基线检出单 case 降幅 → exit 1 ──────────
{
  const root = newRoot(CASES_YAML());
  for (const app of ['a1', 'a2', 'a3', 'a4', 'a5']) plantEvidence(root, app);
  const base = run(['run', '--write-baseline'], root);
  check('S5 全绿可写基线', base.status === 0, `exit=${base.status}`);
  const gate = run(['run', '--gate'], root);
  check('S5 达标 → gate exit 0', gate.status === 0, `exit=${gate.status} ${gate.stdout.trim()}`);
  const baselinePath = join(root, 'Meta', 'backend', 'app-agent', 'eval', 'baseline.json');
  const cmpOk = run(['compare', baselinePath], root);
  check('S5 compare 基线↔最新运行 → exit 0', cmpOk.status === 0, `exit=${cmpOk.status} ${cmpOk.stdout.trim()}`);

  // 让 c2（weight 0.3）失败：extension-verify 判 failed（而非缺失 → 避免 degraded 掩盖回归）
  writeFileSync(
    join(root, 'Pre-Proc', 'UT', 'Apps', 'a2', 'extension-verify.json'),
    JSON.stringify({ verdict: 'failed' }),
  );
  const regressed = run(['run', '--gate'], root);
  check('S5 单 case 降幅超容差 → exit 1', regressed.status === 1, `exit=${regressed.status}`);
  check('S5 回归原因列出该 case', regressed.stderr.includes('c2'), regressed.stderr.trim());
  const cmpRegressed = run(['compare', baselinePath], root);
  check('S5 compare 检出降幅 → exit 1', cmpRegressed.status === 1, `exit=${cmpRegressed.status}`);
  check(
    'S5 compare 输出逐 case 归因',
    cmpRegressed.stdout.includes('c2') && cmpRegressed.stdout.includes('回归'),
    cmpRegressed.stdout.trim(),
  );
  rmSync(root, { recursive: true, force: true });
}

// ── S6：负例——回归投递 plans/ 且同指纹去重 ─────────────────────────────────
{
  const root = newRoot(CASES_YAML());
  plantEvidence(root, 'a1');
  const first = run(['run'], root);
  const plansDir = join(root, 'Pre-Proc', 'UT', 'Apps', 'a2', 'plans');
  const count1 = existsSync(plansDir) ? readdirSync(plansDir).filter((f) => f.endsWith('.md')).length : 0;
  const second = run(['run'], root);
  const count2 = existsSync(plansDir) ? readdirSync(plansDir).filter((f) => f.endsWith('.md')).length : 0;
  check('S6 失败/降级投递 plans/', count1 > 0, `plans=${count1}`);
  check('S6 同指纹不重复入队', count1 === count2, `first=${count1} second=${count2}`);
  const planFile = existsSync(plansDir) ? readdirSync(plansDir).find((x) => x.endsWith('.md')) : undefined;
  const planBody = planFile ? readFileSync(join(plansDir, planFile), 'utf8') : '';
  check('S6 plan 含自解决条款', planBody.includes('## 自解决条款'));
  check('S6 两次运行均 degraded 退出', first.status === 4 && second.status === 4, `${first.status}/${second.status}`);
  rmSync(root, { recursive: true, force: true });
}

console.log(failures === 0 ? '\n✅ 全部自测通过' : `\n❌ ${failures} 项自测失败`);
process.exit(failures === 0 ? 0 : 1);
