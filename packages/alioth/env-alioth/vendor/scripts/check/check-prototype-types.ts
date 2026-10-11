#!/usr/bin/env bun
/// <reference types="bun" />
/**
 * check-prototype-types.ts — 原型源码机械判据（构建期可捕获、运行期致命的缺陷类）
 *
 * 判据:
 *   T1  TS2304 / TS2552（未定义标识符）零容差 —— 该类在 esbuild 构建期无感
 *       （esbuild 不做类型检查），但在渲染期抛 ReferenceError 并卸载整棵 React 树。
 *   T2  Block `llm-tsx/mock.json` 各行键集同构 —— 行键缺失/异构会让表格与详情
 *       渲染出 `undefined` / `NaN`（原型保真度直接受损，且不被任何静态评分拦截）。
 *
 * 用法:
 *   bun scripts/check/check-prototype-types.ts            # 报告，恒 exit 0
 *   bun scripts/check/check-prototype-types.ts --fail     # 有偏离时 exit 1（门禁语义）
 *   bun scripts/check/check-prototype-types.ts --json     # 机器可读
 *   bun scripts/check/check-prototype-types.ts --rebuild-baseline   # 登记当前偏离为存量基线
 *
 * 设计说明: 仅为 T1 生成**临时内联 tsconfig**（jsx: react-jsx / skipLibCheck /
 * strict:false / types:[]），因此无需在仓库内维护额外 tsconfig，也不受各 ns 前端
 * 配置差异影响；除 T1 外的诊断类（临时配置下的模块解析噪声）一律忽略。
 */
import { existsSync, mkdtempSync, readFileSync, readdirSync, rmSync, statSync, writeFileSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { join, relative, resolve } from 'node:path';
import { writeBaseline } from './lib/baseline-store';
import { callerRepoRoot } from '../lib/caller-root';
import { nsFaceSkipNotice, nsFaceStatus, nsSourcesTracked } from '../lib/ns-surface';

// 判定面归属（2026-09-25）：REPO_ROOT = **调用方仓**（ns 仓内调用判 ns 面；平台仓调用仍落平台仓）。
const REPO_ROOT = callerRepoRoot();
/** 工具链回落根 = 脚本位置派生（平台 checkout）：ns 仓无 `node_modules`，T1 的 tsc 只能取自平台
 *  （`gate-adaptation-assessment.md` §3.1）——判定面归 ns 仓、工具链仍跨仓可用。 */
const SCRIPT_ROOT = resolve(import.meta.dir, '../..');
const NS_DIRS = ['Pre-Proc'];
const SOURCE_GLOBS = [
  'Pre-Proc/*/Prototypes/Blocks/*/llm-tsx/**/*.tsx',
  'Pre-Proc/*/Prototypes/Blocks/*/llm-tsx/**/*.ts',
  'Pre-Proc/*/Prototypes/Modules/*/llm-tsx/**/*.tsx',
  'Pre-Proc/*/Prototypes/Modules/*/llm-tsx/**/*.ts',
  'Pre-Proc/*/Prototypes/Apps/*/llm-tsx/**/*.tsx',
  'Pre-Proc/*/Prototypes/Apps/*/llm-tsx/**/*.ts',
  // OpenActivity 层（层级契约 preproc-layout.mjs）：缺此分支则 Open 单元的设计腿**不进 T1/T2/T3 扫描面**
  // ——2026-10-05 实证：`Prototypes/Open/Blocks/block-manage-{theme,language}` 的悬空
  // `SettingsTheme.displayName`（求值期 ReferenceError，原型空白）**未被 T1 捕获**。
  'Pre-Proc/*/Prototypes/Open/Blocks/*/llm-tsx/**/*.tsx',
  'Pre-Proc/*/Prototypes/Open/Blocks/*/llm-tsx/**/*.ts',
  'Pre-Proc/*/Prototypes/Open/Modules/*/llm-tsx/**/*.tsx',
  'Pre-Proc/*/Prototypes/Open/Modules/*/llm-tsx/**/*.ts',
  'Pre-Proc/*/Prototypes/Open/Apps/*/llm-tsx/**/*.tsx',
  'Pre-Proc/*/Prototypes/Open/Apps/*/llm-tsx/**/*.ts',
  'Pre-Proc/*/Prototypes/_shared/**/*.tsx',
  'Pre-Proc/*/Prototypes/_shared/**/*.ts',
];
const BLOCK_GLOBS = ['Pre-Proc/*/Prototypes/Blocks/*', 'Pre-Proc/*/Prototypes/Open/Blocks/*'];

import { defineCheck } from './lib/check-interface';

const META_CHANNEL = '__meta';

class ExitSignal {
  constructor(readonly rc: number) {}
}
let SINK: { out: (l: string) => void; err: (l: string) => void } = {
  out: (l) => process.stdout.write(`${l}\n`),
  err: (l) => process.stderr.write(`${l}\n`),
};
const exit: (rc: number) => never = (rc) => {
  throw new ExitSignal(rc);
};
/** 判定面注入（旧自解析 argv；现由基座 ctx 供给） */
let JSON_OUT = false;
/** `--rebuild-baseline`（自有写面；原名 `--update-baseline` 属基座旗标 ⇒ 改名，见文件头） */
let UPDATE_BASELINE = false;
let NO_BASELINE = false;
/** 门控位：旧 `FAIL && findings>0 ⇒ exit(1)` 的等价物 */
let BLOCKED = false;

type Finding = { rule: 'T1' | 'T2' | 'T3'; file: string; detail: string };

function expandGlob(pattern: string): string[] {
  const out: string[] = [];
  for (const raw of new Bun.Glob(pattern).scanSync({ cwd: REPO_ROOT, onlyFiles: true, dot: false })) {
    out.push(relative(REPO_ROOT, resolve(REPO_ROOT, raw)));
  }
  return out.sort();
}

function collectPrototypeSources(): string[] {
  const set = new Set<string>();
  for (const g of SOURCE_GLOBS) for (const f of expandGlob(g)) set.add(f);
  return [...set].sort();
}

function findTsc(): string | null {
  // 调用方仓优先；ns 仓无 node_modules 时回落脚本位置派生根（平台 checkout）——T1 不因 ROOT 归属而失效。
  for (const root of [REPO_ROOT, SCRIPT_ROOT]) {
    const local = join(root, 'node_modules/.bin/tsc');
    if (existsSync(local)) return local;
  }
  return null;
}

/** T1：临时内联 tsconfig 跑 tsc，只取未定义标识符两类诊断。 */
function checkUndefinedIdentifiers(files: string[]): Finding[] {
  if (files.length === 0) return [];
  const tsc = findTsc();
  if (!tsc) {
    SINK.err('[check-prototype-types] ⚠ 未找到 node_modules/.bin/tsc —— 跳过 T1（请先 pnpm install）');
    return [];
  }
  const dir = mkdtempSync(join(tmpdir(), 'proto-types-'));
  const cfgPath = join(dir, 'tsconfig.json');
  const cfg = {
    compilerOptions: {
      noEmit: true,
      jsx: 'react-jsx',
      target: 'es2020',
      module: 'esnext',
      moduleResolution: 'bundler',
      strict: false,
      skipLibCheck: true,
      types: [],
    },
    include: files.map((f) => join(REPO_ROOT, f)),
  };
  writeFileSync(cfgPath, JSON.stringify(cfg, null, 2), 'utf8');
  try {
    const proc = Bun.spawnSync([tsc, '-p', cfgPath, '--pretty', 'false'], { cwd: REPO_ROOT });
    const out = `${proc.stdout.toString()}\n${proc.stderr.toString()}`;
    const findings: Finding[] = [];
    for (const line of out.split('\n')) {
      if (!/error TS2304|error TS2552/.test(line)) continue;
      const m = /^(.+?)\(\d+,\d+\): error (TS\d+): (.*)$/.exec(line.trim());
      if (!m) continue;
      findings.push({ rule: 'T1', file: relative(REPO_ROOT, m[1]), detail: `${m[2]}: ${m[3]}` });
    }
    return findings;
  } finally {
    rmSync(dir, { recursive: true, force: true });
  }
}

/** T2：Block mock.json 各行键集必须同构。 */
function checkMockRowHomogeneity(): Finding[] {
  const findings: Finding[] = [];
  const seen = new Set<string>();
  for (const g of BLOCK_GLOBS) {
    for (const dir of new Bun.Glob(g).scanSync({ cwd: REPO_ROOT, onlyFiles: false })) {
      if (seen.has(dir)) continue;
      seen.add(dir);
      const mockPath = join(REPO_ROOT, dir, 'llm-tsx', 'mock.json');
      if (!existsSync(mockPath) || !statSync(mockPath).isFile()) continue;
      const rel = relative(REPO_ROOT, mockPath);
      let parsed: unknown;
      try {
        parsed = JSON.parse(readFileSync(mockPath, 'utf8'));
      } catch (err) {
        findings.push({ rule: 'T2', file: rel, detail: `mock.json 解析失败: ${(err as Error).message}` });
        continue;
      }
      const rows = (parsed as { rows?: unknown }).rows;
      if (!Array.isArray(rows) || rows.length === 0) continue;
      const keyOf = (row: unknown): string =>
        row && typeof row === 'object' ? Object.keys(row as Record<string, unknown>).sort().join(',') : '';
      const first = keyOf(rows[0]);
      const bad: number[] = [];
      rows.forEach((r, i) => {
        if (keyOf(r) !== first) bad.push(i);
      });
      if (bad.length > 0) {
        findings.push({
          rule: 'T2',
          file: rel,
          detail: `行键不一致：rows[0] 与 rows[${bad.slice(0, 5).join(',')}] 键集不同（异构行会渲染 undefined/NaN）`,
        });
      }
    }
  }
  return findings;
}

/** T3：**版本形态**的 `prototypeVersion` 必须能解析到产物。
 *  仓内存在两种命名惯例（裸 `vN` 与带前缀 `b-vN`/`m-vN`），且该字段亦被用作非版本标记
 *  （如 `design`）——故只有当声明形如 `^(b-|m-)?v\d+$` 时才判定，且两种命名任一命中即通过；
 *  其余取值不判（避免误报）。该判据覆盖「声明了版本却无产物 / 前缀写错」的断链类。 */
function checkPointerIntegrity(): Finding[] {
  const findings: Finding[] = [];
  const pairs: Array<{ jsonGlob: string; protoSub: string; prefix: string; kind: string }> = [
    { jsonGlob: 'Pre-Proc/*/Sources/Apps/Blocks/*/block.json', protoSub: 'Blocks', prefix: 'b', kind: 'block' },
    { jsonGlob: 'Pre-Proc/*/Sources/Apps/Modules/*/module.json', protoSub: 'Modules', prefix: 'm', kind: 'module' },
  ];
  for (const pair of pairs) {
    for (const jsonRel of expandGlob(pair.jsonGlob)) {
      let parsed: unknown;
      try {
        parsed = JSON.parse(readFileSync(join(REPO_ROOT, jsonRel), 'utf8'));
      } catch {
        continue;
      }
      const version = (parsed as { prototypeVersion?: unknown }).prototypeVersion;
      if (typeof version !== 'string' || !/^(?:[bm]-)?v\d+$/.test(version)) continue;
      const m = /^Pre-Proc\/([^/]+)\/Sources\/Apps\/(?:Blocks|Modules)\/([^/]+)\//.exec(jsonRel);
      if (!m) continue;
      const [, ns, id] = m;
      const protoDir = `Pre-Proc/${ns}/Prototypes/${pair.protoSub}/${id}`;
      const bare = version.replace(/^[bm]-/, '');
      const candidates = [`${version}.html`, `${pair.prefix}-${bare}.html`];
      if (!candidates.some((c) => existsSync(join(REPO_ROOT, protoDir, c)))) {
        findings.push({
          rule: 'T3',
          file: jsonRel,
          detail: `prototypeVersion="${version}" 无法解析到产物（尝试 ${candidates.join(' / ')}，目录 ${protoDir}）`,
        });
      }
    }
  }
  return findings;
}

/** 基线路径 + 键（键含 TAB ⇒ 自持解析/写盘，不适用基座「每行首段」口径）。 */
const BASELINE_PATH = join(REPO_ROOT, 'scripts/check/.prototype-types-baseline.txt');
const KEY_TAB = '\t';
export const keyOf = (f: Finding): string => `${f.rule}${KEY_TAB}${f.file}${KEY_TAB}${f.detail}`;

interface PrototypeTypesOutcome {
  out: string[];
  err: string[];
  rc: number;
  blocked: boolean;
}

/** 判定面（T1 未定义标识符 / T2 mock 行键同构 / T3 prototypeVersion 指针）；输出经 SINK 记录。 */
export function prototypeTypesState(opts: { json: boolean; rebuildBaseline: boolean; noBaseline: boolean }): PrototypeTypesOutcome {
  const OUT: string[] = [];
  const ERR: string[] = [];
  const saved = SINK;
  SINK = { out: (l) => OUT.push(l), err: (l) => ERR.push(l) };
  JSON_OUT = opts.json;
  UPDATE_BASELINE = opts.rebuildBaseline;
  NO_BASELINE = opts.noBaseline;
  BLOCKED = false;
  try {
const sources = collectPrototypeSources();

// 判定面归属（2026-09-25）：0 文件 = 本仓不持有 ns 面（显式跳过）或扫描面失效（fail-loud）——不得打印 ✅ 假绿。
if (sources.length === 0) {
  if (!nsSourcesTracked(REPO_ROOT)) {
    nsFaceSkipNotice('check-prototype-types', REPO_ROOT);
    exit(0);
  }
  const nsLabel = nsFaceStatus(REPO_ROOT).realDirs.join(', ');
  SINK.err(
    `❌ check-prototype-types: ns「${nsLabel}」被本仓跟踪但解析出 0 个原型源码文件（ROOT=${REPO_ROOT}）—— 扫描面失效，MUST NOT 静默通过`,
  );
  exit(1);
}
const rawFindings = [...checkUndefinedIdentifiers(sources), ...checkMockRowHomogeneity(), ...checkPointerIntegrity()];


let baseline: Set<string> = new Set();
if (!NO_BASELINE && existsSync(BASELINE_PATH)) {
  for (const line of readFileSync(BASELINE_PATH, 'utf8').split('\n')) {
    const t = line.trim();
    if (t.length === 0 || t.startsWith('#')) continue;
    baseline.add(t);
  }
}

if (UPDATE_BASELINE) {
  const header = [
    '# check-prototype-types 基线（存量偏离，仅登记不阻断；--fail 只拦基线外新增）',
    '# 生成: bun scripts/check/check-prototype-types.ts --rebuild-baseline',
    '# 形态: <rule>TAB<file>TAB<detail>',
    `# 生成时命中: ${rawFindings.length}`,
  ];
  writeBaseline(REPO_ROOT, 'scripts/check/.prototype-types-baseline.txt', [...header, ...[...new Set(rawFindings.map(keyOf))].sort()].join('\n') + '\n');
  SINK.out(`[check-prototype-types] 已写入基线：${BASELINE_PATH}（${rawFindings.length} 项）`);
  exit(0);
}

const findings = rawFindings.filter((f) => !baseline.has(keyOf(f)));
const grandfathered = rawFindings.length - findings.length;

if (JSON_OUT) {
  SINK.out(JSON.stringify({ scanned: sources.length, findings, grandfathered }, null, 2));
} else {
  SINK.out(`[check-prototype-types] 扫描原型源码 ${sources.length} 个文件（ns 根: ${NS_DIRS.join(',')}）`);
  if (findings.length === 0) {
    SINK.out(
      `[check-prototype-types] ✅ 基线外 0 命中（T1 未定义标识符 / T2 mock 行键同构 / T3 prototypeVersion 指针）；基线登记存量 ${grandfathered} 项`,
    );
  } else {
    for (const f of findings) SINK.err(`❌ ${f.rule} ${f.file}\n    ${f.detail}`);
    SINK.err(`[check-prototype-types] ❌ 基线外共 ${findings.length} 项偏离（基线登记存量 ${grandfathered} 项）`);
  }
}

BLOCKED = findings.length > 0; // 门控位（旧 `FAIL && findings>0 ⇒ exit(1)`）
  } catch (e) {
    if (!(e instanceof ExitSignal)) throw e;
    return { out: OUT, err: ERR, rc: e.rc, blocked: BLOCKED };
  } finally {
    SINK = saved;
  }
  return { out: OUT, err: ERR, rc: 0, blocked: BLOCKED };
}

const gate = defineCheck({
  id: 'prototype-types',
  label: 'check-prototype-types',
  flags: ['--rebuild-baseline', '--no-baseline'],
  predicate: () => [],
  report: (ctx, res, io) => {
    const st = prototypeTypesState({
      json: res.json,
      rebuildBaseline: ctx.flags.has('--rebuild-baseline'),
      noBaseline: ctx.flags.has('--no-baseline'),
    });
    for (const line of st.out) io.out(`${line}\n`);
    for (const line of st.err) io.err(`${line}\n`);
    if (st.rc !== 0) return st.rc;
    return res.fail && st.blocked ? 1 : 0;
  },
  selfTest() {
    // 判据自证（R1）：基线键形态（TAB 三分段）+ 扫描面非空（glob 面可用性）
    const k = keyOf({ rule: 'T1', file: 'a.ts', detail: 'd' });
    if (k.split('\t').length !== 3 || k !== 'T1\ta.ts\td') {
      process.stderr.write(`❌ prototype-types self-test 失败：基线键形态 ${JSON.stringify(k)}\n`);
      return 1;
    }
    const globbed = expandGlob('Pre-Proc/*/Prototypes/Blocks/*');
    if (!Array.isArray(globbed) || globbed.some((f) => f.startsWith('/'))) {
      process.stderr.write('❌ prototype-types self-test 失败：glob 面返回非仓内相对路径\n');
      return 1;
    }
    process.stdout.write(`✅ prototype-types self-test 通过（基线键 TAB 三分段 + glob 面 ${globbed.length} 项）\n`);
    return 0;
  },
});

if (import.meta.main) process.exit(await gate.main());
