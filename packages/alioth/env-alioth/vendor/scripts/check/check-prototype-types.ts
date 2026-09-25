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
 *   bun scripts/check/check-prototype-types.ts --update-baseline   # 登记当前偏离为存量基线
 *
 * 设计说明: 仅为 T1 生成**临时内联 tsconfig**（jsx: react-jsx / skipLibCheck /
 * strict:false / types:[]），因此无需在仓库内维护额外 tsconfig，也不受各 ns 前端
 * 配置差异影响；除 T1 外的诊断类（临时配置下的模块解析噪声）一律忽略。
 */
import { existsSync, mkdtempSync, readFileSync, readdirSync, rmSync, statSync, writeFileSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { join, relative, resolve } from 'node:path';
import { writeBaseline } from './lib/baseline-store';

const REPO_ROOT = resolve(import.meta.dir, '../..');
const NS_DIRS = ['Pre-Proc'];
const SOURCE_GLOBS = [
  'Pre-Proc/*/Prototypes/Blocks/*/llm-tsx/**/*.tsx',
  'Pre-Proc/*/Prototypes/Blocks/*/llm-tsx/**/*.ts',
  'Pre-Proc/*/Prototypes/Modules/*/llm-tsx/**/*.tsx',
  'Pre-Proc/*/Prototypes/Modules/*/llm-tsx/**/*.ts',
  'Pre-Proc/*/Prototypes/Apps/*/llm-tsx/**/*.tsx',
  'Pre-Proc/*/Prototypes/Apps/*/llm-tsx/**/*.ts',
  'Pre-Proc/*/Prototypes/_shared/**/*.tsx',
  'Pre-Proc/*/Prototypes/_shared/**/*.ts',
];
const BLOCK_GLOBS = ['Pre-Proc/*/Prototypes/Blocks/*'];

const args = process.argv.slice(2);
const FAIL = args.includes('--fail');
const JSON_OUT = args.includes('--json');

type Finding = { rule: 'T1' | 'T2'; file: string; detail: string };

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
  const local = join(REPO_ROOT, 'node_modules/.bin/tsc');
  return existsSync(local) ? local : null;
}

/** T1：临时内联 tsconfig 跑 tsc，只取未定义标识符两类诊断。 */
function checkUndefinedIdentifiers(files: string[]): Finding[] {
  if (files.length === 0) return [];
  const tsc = findTsc();
  if (!tsc) {
    console.error('[check-prototype-types] ⚠ 未找到 node_modules/.bin/tsc —— 跳过 T1（请先 pnpm install）');
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

const sources = collectPrototypeSources();
const rawFindings = [...checkUndefinedIdentifiers(sources), ...checkMockRowHomogeneity(), ...checkPointerIntegrity()];

const BASELINE_PATH = join(REPO_ROOT, 'scripts/check/.prototype-types-baseline.txt');
const UPDATE_BASELINE = args.includes('--update-baseline');
const keyOf = (f: Finding): string => `${f.rule}\t${f.file}\t${f.detail}`;

let baseline: Set<string> = new Set();
// --no-baseline：不读基线（供「裁决存量」阶段/并发 worker 使用，避免 mv 共享文件竞态）
const NO_BASELINE = args.includes('--no-baseline');
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
    '# 生成: bun scripts/check/check-prototype-types.ts --update-baseline',
    '# 形态: <rule>TAB<file>TAB<detail>',
    `# 生成时命中: ${rawFindings.length}`,
  ];
  writeBaseline(REPO_ROOT, 'scripts/check/.prototype-types-baseline.txt', [...header, ...[...new Set(rawFindings.map(keyOf))].sort()].join('\n') + '\n');
  console.log(`[check-prototype-types] 已写入基线：${BASELINE_PATH}（${rawFindings.length} 项）`);
  process.exit(0);
}

const findings = rawFindings.filter((f) => !baseline.has(keyOf(f)));
const grandfathered = rawFindings.length - findings.length;

if (JSON_OUT) {
  console.log(JSON.stringify({ scanned: sources.length, findings, grandfathered }, null, 2));
} else {
  console.log(`[check-prototype-types] 扫描原型源码 ${sources.length} 个文件（ns 根: ${NS_DIRS.join(',')}）`);
  if (findings.length === 0) {
    console.log(
      `[check-prototype-types] ✅ 基线外 0 命中（T1 未定义标识符 / T2 mock 行键同构 / T3 prototypeVersion 指针）；基线登记存量 ${grandfathered} 项`,
    );
  } else {
    for (const f of findings) console.error(`❌ ${f.rule} ${f.file}\n    ${f.detail}`);
    console.error(`[check-prototype-types] ❌ 基线外共 ${findings.length} 项偏离（基线登记存量 ${grandfathered} 项）`);
  }
}

process.exit(FAIL && findings.length > 0 ? 1 : 0);
