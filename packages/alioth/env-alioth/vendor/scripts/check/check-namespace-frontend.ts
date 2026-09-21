#!/usr/bin/env bun
/// <reference types="bun" />
/**
 * check-namespace-frontend.ts
 *
 * Scans every namespace module frontend and verifies the minimum engineering
 * skeleton defined by openspec/specs/frontend-namespace-consistency.
 *
 * Exit codes:
 *   0  all non-baseline modules pass
 *   1  at least one non-baseline module fails a MUST-level check
 *
 * Uses Bun + project parser utilities (scripts/lib/parsers.ts).
 */

import { existsSync, readFileSync, statSync } from 'fs';
import { fileURLToPath } from 'node:url';
import { dirname, join, relative, resolve } from 'path';
import { js } from '../lib/parsers.ts';

const __filename = fileURLToPath(import.meta.url);
const __dirname = resolve(__filename, '..');
const REPO_ROOT = resolve(__dirname, '../..');
const BASELINE_PATH = join(REPO_ROOT, 'scripts/check/.namespace-frontend-baseline.txt');

const MODULE_FRONTEND_GLOBS = ['Pre-Proc/*/Sources/Modules/*/frontend', 'Pre-Proc/*/Sources/Apps/Modules/*/frontend', 'Modules/*/*/frontend'];

const REQUIRED_FILES = [
  'src/App.tsx',
  'src/theme.css',
  'src/locales/zh-CN.json',
  'src/locales/en.json',
];

const ENTRY_FILES = ['src/single-spa.tsx', 'src/main.tsx'];

/**
 * 库构建入口链是否触达目标文件（如 theme.css）。
 *
 * 背景（2026-09-17 ct-bv-local 战役实测）：模块把 `import './theme.css'` 只写在
 * `src/main.tsx`（dev 入口），而库构建入口是 `src/single-spa.tsx` ⇒ CSS 不进构建图，
 * `vite build` 产不出 `dist/theme.css`，线上模块无样式——这是**生产**缺陷且既有门禁
 * （必需文件存在性 / vite externals / 视觉冒烟）全部放行。
 *
 * 实现：用 Bun 原生 TSX 导入扫描（非正则模拟解析）做有界 BFS（depth ≤ 3）。
 */
const TSX_TRANSPILER = new Bun.Transpiler({ loader: 'tsx' });
const IMPORT_WALK_MAX_DEPTH = 3;

function importGraphReaches(entryAbs: string, targetSuffix: string): boolean {
  const seen = new Set<string>();
  const queue: Array<{ file: string; depth: number }> = [{ file: entryAbs, depth: 0 }];
  while (queue.length > 0) {
    const item = queue.shift();
    if (!item) break;
    if (seen.has(item.file)) continue;
    seen.add(item.file);
    let code: string;
    try {
      code = readFileSync(item.file, 'utf8');
    } catch {
      continue;
    }
    let records: Array<{ path: string }>;
    try {
      records = TSX_TRANSPILER.scanImports(code) as Array<{ path: string }>;
    } catch {
      continue;
    }
    for (const rec of records) {
      if (rec.path.endsWith(targetSuffix)) return true;
      if (item.depth >= IMPORT_WALK_MAX_DEPTH || !rec.path.startsWith('.')) continue;
      const base = resolve(dirname(item.file), rec.path);
      const next = [base, `${base}.tsx`, `${base}.ts`, join(base, 'index.tsx'), join(base, 'index.ts')].find(
        (p) => existsSync(p) && statSync(p).isFile(),
      );
      if (next) queue.push({ file: next, depth: item.depth + 1 });
    }
  }
  return false;
}

const STANDARD_SHARED_RUNTIME: Record<string, true> = {
  react: true,
  'react-dom': true,
  'react-dom/client': true,
  'react-router-dom': true,
  jotai: true,
  '@tanstack/react-query': true,
  '@alioth/api': true,
  '@alioth/components': true,
  '@alioth/hooks': true,
  '@alioth/i18n': true,
  '@alioth/utils': true,
  'single-spa': true,
};

type CheckStatus = 'pass' | 'fail' | 'warn' | 'baseline';

interface CheckResult {
  name: string;
  status: CheckStatus;
  message: string;
}

interface ModuleReport {
  namespace: string;
  module: string;
  path: string;
  baseline: boolean;
  results: CheckResult[];
}

interface BaselineEntry {
  path: string;
  sha: string;
}

function readBaseline(): BaselineEntry[] {
  if (!existsSync(BASELINE_PATH)) return [];
  const text = readFileSync(BASELINE_PATH, 'utf-8');
  const entries: BaselineEntry[] = [];
  for (const line of text.split(/\r?\n/)) {
    const trimmed = line.trim();
    if (!trimmed || trimmed.startsWith('#')) continue;
    const parts = trimmed.split(/\s+/);
    if (parts.length >= 2) {
      entries.push({ path: parts[0], sha: parts[1] });
    }
  }
  return entries;
}

function normalizeModulePath(p: string): string {
  return p.replace(/\\/g, '/').replace(/\/$/, '');
}

function isBaseline(modulePath: string, baseline: BaselineEntry[]): boolean {
  const rel = normalizeModulePath(relative(REPO_ROOT, modulePath));
  return baseline.some((b) => normalizeModulePath(b.path) === rel);
}

function parseJsonFile(path: string): unknown | Error {
  try {
    const text = readFileSync(path, 'utf-8');
    return JSON.parse(text);
  } catch (err) {
    return err instanceof Error ? err : new Error(String(err));
  }
}

function getPropertyValue(
  node: Record<string, unknown> | undefined,
  name: string,
): Record<string, unknown> | undefined {
  if (!node || node.type !== 'ObjectExpression') return undefined;
  const props = node.properties as Array<Record<string, unknown>> | undefined;
  if (!props) return undefined;
  for (const prop of props) {
    if (prop.type !== 'Property' || prop.computed) continue;
    const key = prop.key as Record<string, unknown> | undefined;
    if (key && key.type === 'Identifier' && key.name === name) {
      return prop.value as Record<string, unknown>;
    }
  }
  return undefined;
}

function getStringValues(node: Record<string, unknown> | undefined): string[] {
  if (!node || node.type !== 'ArrayExpression') return [];
  const elements = node.elements as Array<Record<string, unknown> | null> | undefined;
  if (!elements) return [];
  return elements
    .map((el) => {
      if (!el) return undefined;
      if (el.type === 'Literal' && typeof el.value === 'string') return el.value as string;
      if (el.type === 'TemplateLiteral') {
        const quasis = el.quasis as Array<Record<string, unknown>> | undefined;
        if (quasis?.length === 1 && (el.expressions as unknown[] | undefined)?.length === 0) {
          const cooked = (quasis[0].value as Record<string, unknown>).cooked;
          return typeof cooked === 'string' ? cooked : undefined;
        }
      }
      return undefined;
    })
    .filter((v): v is string => v !== undefined);
}

/** 提取 external 数组中的正则字面量 source（如 /^@alioth(\/.*)?$/ → "^@alioth(\\/.*)?$"）。 */
export function getRegexLiteralSources(node: Record<string, unknown> | undefined): string[] {
  if (!node || node.type !== 'ArrayExpression') return [];
  const elements = node.elements as Array<Record<string, unknown> | null> | undefined;
  if (!elements) return [];
  return elements
    .map((el) => {
      if (!el || el.type !== 'Literal') return undefined;
      const regex = el.regex as Record<string, unknown> | undefined;
      // meriyah 的 regex.pattern 不含斜杠；还原为源码形态（/pattern/flags）供 regexCoversDep 解析
      return typeof regex?.pattern === 'string'
        ? `/${regex.pattern}/${typeof regex.flags === 'string' ? regex.flags : ''}`
        : undefined;
    })
    .filter((v): v is string => v !== undefined);
}

/**
 * 判断正则 source 是否可证明覆盖依赖 dep。
 * 仅识别白名单形态（要求 `^` 锚定）：
 *   1) /^scope(\/.*)?$/ — 覆盖前缀本身或其子路径
 *   2) /^scope\//      — 仅覆盖前缀的子路径
 * 拒绝 alternation / 无锚点 / 其他形态，避免误判。
 */
export function regexCoversDep(regexSource: string, dep: string): boolean {
  const mA = regexSource.match(/^\/\^([A-Za-z0-9@._/-]+?)\(\\\/\.\*\)\?\$\//);
  if (mA) {
    const prefix = mA[1].replace(/\\\//g, '/');
    return dep === prefix || dep.startsWith(prefix + '/');
  }
  const mB = regexSource.match(/^\/\^([A-Za-z0-9@._/-]+?)\\\/\//);
  if (mB) {
    const prefix = mB[1].replace(/\\\//g, '/');
    return dep.startsWith(prefix + '/');
  }
  return false;
}

/**
 * estree 节点递归遍历（meriyah 输出）。
 */
function walkAst(node: unknown, visit: (n: Record<string, unknown>) => void): void {
  if (!node || typeof node !== 'object') return;
  const n = node as Record<string, unknown>;
  visit(n);
  for (const key of Object.keys(n)) {
    const v = n[key];
    if (Array.isArray(v)) {
      for (const c of v) walkAst(c, visit);
    } else if (v && typeof v === 'object' && (v as Record<string, unknown>).type) {
      walkAst(v, visit);
    }
  }
}

function parseViteConfig(modulePath: string): {
  ast?: unknown;
  defaultExport?: Record<string, unknown>;
  error?: Error;
} {
  const vitePath = join(modulePath, 'vite.config.ts');
  if (!existsSync(vitePath)) return {};
  try {
    const text = readFileSync(vitePath, 'utf-8');
    // typescript@7（原生编译器）主入口不再提供 TS5 编译器 API；
    // 用 Bun.Transpiler 去类型后 meriyah 解析（标准解析器，符合 NO_REGEX 规约）
    const transpiled = new Bun.Transpiler().transformSync(text, { loader: 'ts' });
    const ast = js.parseModule(transpiled) as unknown;
    let defaultExport: Record<string, unknown> | undefined;
    walkAst(ast, (node) => {
      if (node.type === 'ExportDefaultDeclaration' && !defaultExport) {
        defaultExport = node.declaration as Record<string, unknown>;
      }
    });
    return { ast, defaultExport };
  } catch (err) {
    return { error: err instanceof Error ? err : new Error(String(err)) };
  }
}

function hasDefineModuleViteConfigImport(sourceFile: unknown): boolean {
  let found = false;
  walkAst(sourceFile, (node) => {
    if (node.type !== 'ImportDeclaration') return;
    const src = node.source as Record<string, unknown> | undefined;
    if (src?.value !== '@alioth/config/vite') return;
    const specifiers = node.specifiers as Array<Record<string, unknown>> | undefined;
    if (!specifiers) return;
    for (const s of specifiers) {
      const imported = s.imported as Record<string, unknown> | undefined;
      if (s.type === 'ImportSpecifier' && imported?.name === 'defineModuleViteConfig') {
        found = true;
      }
    }
  });
  return found;
}

function checkModule(modulePath: string, baseline: BaselineEntry[]): ModuleReport {
  const rel = relative(REPO_ROOT, modulePath);
  const segments = rel.split('/');
  const namespace = segments[1] ?? '';
  const moduleName = segments[segments.length - 2] ?? '';
  const baselineFlag = isBaseline(modulePath, baseline);
  const results: CheckResult[] = [];

  const add = (name: string, status: CheckStatus, message: string) => {
    if (baselineFlag && status === 'fail') {
      results.push({ name, status: 'baseline', message: `${message} (baseline-grandfathered)` });
    } else {
      results.push({ name, status, message });
    }
  };

  const packageJsonPath = join(modulePath, 'package.json');
  if (!existsSync(packageJsonPath)) {
    add('package.json', 'fail', 'missing package.json');
  } else {
    const parsed = parseJsonFile(packageJsonPath);
    if (parsed instanceof Error) {
      add('package.json', 'fail', `invalid JSON: ${parsed.message}`);
    } else {
      add('package.json', 'pass', 'package.json exists and is valid JSON');
    }
  }

  const vitePath = join(modulePath, 'vite.config.ts');
  if (!existsSync(vitePath)) {
    add('vite.config.ts', 'fail', 'missing vite.config.ts');
  } else {
    const { ast, defaultExport, error } = parseViteConfig(modulePath);
    if (error || !ast) {
      add('vite.config.ts', 'fail', `cannot parse: ${error?.message ?? 'unknown error'}`);
    } else {
      let configObject: Record<string, unknown> | undefined = defaultExport;
      if (configObject && configObject.type === 'CallExpression') {
        const args = configObject.arguments as Array<Record<string, unknown>> | undefined;
        configObject = args?.[0];
      }

      const buildNode = configObject ? getPropertyValue(configObject, 'build') : undefined;
      const libNode = buildNode ? getPropertyValue(buildNode, 'lib') : undefined;
      const formatsNode = libNode ? getPropertyValue(libNode, 'formats') : undefined;
      const formats = getStringValues(formatsNode);
      if (!formats.includes('es')) {
        add(
          'vite.lib.formats',
          'fail',
          `build.lib.formats MUST include "es" (got: ${formats.join(', ') || 'none'})`,
        );
      } else {
        add('vite.lib.formats', 'pass', 'build.lib.formats includes "es"');
      }

      const pkg = parseJsonFile(packageJsonPath);
      const declaredSharedDeps = new Set<string>();
      if (!(pkg instanceof Error) && typeof pkg === 'object' && pkg !== null) {
        const deps = (pkg as Record<string, unknown>).dependencies;
        const peers = (pkg as Record<string, unknown>).peerDependencies;
        if (typeof deps === 'object' && deps !== null) {
          Object.keys(deps).forEach((k) => {
            if (STANDARD_SHARED_RUNTIME[k]) declaredSharedDeps.add(k);
          });
        }
        if (typeof peers === 'object' && peers !== null) {
          Object.keys(peers).forEach((k) => {
            if (STANDARD_SHARED_RUNTIME[k]) declaredSharedDeps.add(k);
          });
        }
      }

      const rollupOptions = buildNode ? getPropertyValue(buildNode, 'rollupOptions') : undefined;
      const externalNode = rollupOptions ? getPropertyValue(rollupOptions, 'external') : undefined;
      const externals = getStringValues(externalNode);
      // Rollup 允许 external 使用正则字面量（如 /^@alioth(\/.*)?$/），
      // 此处同时提取正则的 source 以便识别可证明覆盖的前缀。
      const regexExternals: string[] = getRegexLiteralSources(externalNode);
      const externalSet = new Set(externals);
      const missingExternals: string[] = [];
      for (const dep of declaredSharedDeps) {
        let covered = externalSet.has(dep);
        if (!covered) {
          for (const ext of externalSet) {
            if (ext === dep || (ext.endsWith('/*') && dep.startsWith(ext.slice(0, -1)))) {
              covered = true;
              break;
            }
            if (ext === '@alioth/*' && dep.startsWith('@alioth/')) {
              covered = true;
              break;
            }
          }
        }
        if (!covered) {
          covered = regexExternals.some((re) => regexCoversDep(re, dep));
        }
        if (!covered) missingExternals.push(dep);
      }
      if (missingExternals.length > 0) {
        add(
          'vite.externals',
          'fail',
          `missing externals for declared deps: ${missingExternals.join(', ')}`,
        );
      } else {
        add(
          'vite.externals',
          'pass',
          'all declared dependencies/peerDependencies are externalized',
        );
      }

      if (hasDefineModuleViteConfigImport(ast)) {
        add('vite.shared-preset', 'pass', 'uses defineModuleViteConfig from @alioth/config/vite');
      } else {
        add(
          'vite.shared-preset',
          'warn',
          'SHOULD import defineModuleViteConfig from @alioth/config/vite',
        );
      }
    }
  }

  const tsconfigPath = join(modulePath, 'tsconfig.json');
  if (!existsSync(tsconfigPath)) {
    add('tsconfig.json', 'fail', 'missing tsconfig.json');
  } else {
    const parsed = parseJsonFile(tsconfigPath);
    if (parsed instanceof Error) {
      add('tsconfig.json', 'fail', `invalid JSON: ${parsed.message}`);
    } else {
      add('tsconfig.json', 'pass', 'tsconfig.json exists and is valid JSON');
    }
  }

  for (const file of REQUIRED_FILES) {
    const filePath = join(modulePath, file);
    const key = file.replace(/^src\//, '').replace(/\//g, '.');
    if (!existsSync(filePath)) {
      add(`file.${key}`, 'fail', `missing ${file}`);
    } else {
      add(`file.${key}`, 'pass', `${file} exists`);
    }
  }

  const entryPath = ENTRY_FILES.find((f) => existsSync(join(modulePath, f)));
  if (!entryPath) {
    add('entry', 'fail', `missing entry file (${ENTRY_FILES.join(' or ')})`);
  } else {
    add('entry', 'pass', `entry file ${entryPath} exists`);
    if (entryPath === 'src/main.tsx') {
      add('entry.canonical', 'warn', 'SHOULD use src/single-spa.tsx as canonical entry');
    }
  }

  // css.entry-reachability（MUST）：库构建入口链必须触达 src/theme.css，
  // 否则 vite lib build 不产出 dist/theme.css（线上模块无样式，见脚本头注释）。
  {
    const libEntries = ['src/single-spa.tsx', 'src/App.tsx']
      .map((f) => join(modulePath, f))
      .filter((p) => existsSync(p) && statSync(p).isFile());
    if (libEntries.length === 0) {
      add('css.entry-reachability', 'warn', 'no lib entry candidate (src/single-spa.tsx | src/App.tsx)');
    } else if (libEntries.some((p) => importGraphReaches(p, 'theme.css'))) {
      add('css.entry-reachability', 'pass', 'src/theme.css reachable from lib entry chain');
    } else {
      add(
        'css.entry-reachability',
        'fail',
        'src/theme.css NOT reachable from lib entry (src/single-spa.tsx | src/App.tsx) — vite lib build 不会产出 theme.css',
      );
    }
  }

  const zhPath = join(modulePath, 'src/locales/zh-CN.json');
  const enPath = join(modulePath, 'src/locales/en.json');
  for (const [path, label] of [
    [zhPath, 'locales.zh-CN'],
    [enPath, 'locales.en'],
  ] as const) {
    if (!existsSync(path)) continue;
    const parsed = parseJsonFile(path);
    if (parsed instanceof Error) {
      add(label, 'fail', `invalid JSON: ${parsed.message}`);
    } else if (typeof parsed !== 'object' || parsed === null) {
      add(label, 'fail', 'locale file must be a JSON object');
    } else {
      const keys = Object.keys(parsed);
      const hasModuleKey = keys.some((k) => k.startsWith(`${moduleName}.`));
      if (keys.length === 0) {
        add(label, 'fail', 'locale file is empty');
      } else if (!hasModuleKey) {
        add(label, 'warn', `locale file has no key matching "${moduleName}.*"`);
      } else {
        add(label, 'pass', `locale file has ${keys.length} keys`);
      }
    }
  }

  return { namespace, module: moduleName, path: rel, baseline: baselineFlag, results };
}

function discoverModuleFrontends(extraPaths: string[] = []): string[] {
  const found = new Set<string>();

  // Default glob patterns
  for (const pattern of MODULE_FRONTEND_GLOBS) {
    const dirPattern = pattern.endsWith('/') ? pattern : `${pattern}/`;
    const glob = new Bun.Glob(dirPattern);
    for (const match of glob.scanSync({ cwd: REPO_ROOT, absolute: true, onlyFiles: false })) {
      if (match.endsWith('/frontend') || match.endsWith('\\frontend')) {
        found.add(match);
      }
    }
  }

  // 过滤仅剩 ignored 产物（node_modules/dist 等）的迁移残留空壳：tracked 内容为空则非有效模块
  for (const dir of Array.from(found)) {
    const rel = relative(REPO_ROOT, dir);
    const tracked = Bun.spawnSync(['git', 'ls-files', '--', `${rel}/`], { cwd: REPO_ROOT });
    if (tracked.stdout.toString().trim() === '') found.delete(dir);
  }

  // Extra explicit paths (used for negative tests)
  for (const p of extraPaths) {
    const abs = resolve(REPO_ROOT, p);
    if (existsSync(abs)) found.add(abs);
  }

  return Array.from(found).sort();
}

function color(status: CheckStatus): string {
  switch (status) {
    case 'pass':
      return '\x1b[32m';
    case 'fail':
      return '\x1b[31m';
    case 'warn':
      return '\x1b[33m';
    case 'baseline':
      return '\x1b[34m';
    default:
      return '';
  }
}

function reset(): string {
  return '\x1b[0m';
}

function main() {
  const args = process.argv.slice(2);
  const extraPaths: string[] = [];
  for (let i = 0; i < args.length; i++) {
    if (args[i] === '--scan' && i + 1 < args.length) {
      extraPaths.push(args[i + 1]);
      i++;
    }
  }

  const baseline = readBaseline();
  const modulePaths = discoverModuleFrontends(extraPaths);

  if (modulePaths.length === 0) {
    console.error('No module frontend directories found.');
    process.exit(1);
  }

  const reports = modulePaths.map((p) => checkModule(p, baseline));

  let mustFailures = 0;
  let warnings = 0;
  let baselineCount = 0;

  for (const report of reports) {
    const ns = report.namespace || '(unknown)';
    console.log(`\n# ${ns} / ${report.module}${report.baseline ? ' [baseline]' : ''}`);
    console.log(`  path: ${report.path}`);
    for (const r of report.results) {
      const label = r.status === 'baseline' ? 'BASELINE' : r.status.toUpperCase();
      console.log(`  ${color(r.status)}${label}${reset()}  ${r.name}: ${r.message}`);
      if (r.status === 'fail') mustFailures++;
      if (r.status === 'warn') warnings++;
      if (r.status === 'baseline') baselineCount++;
    }
  }

  console.log('\n' + '='.repeat(60));
  console.log(`Modules scanned: ${reports.length}`);
  console.log(`MUST failures: ${mustFailures}`);
  console.log(`Baseline-grandfathered: ${baselineCount}`);
  console.log(`Warnings: ${warnings}`);
  if (baseline.length > 0) {
    console.log(`Baseline file: ${BASELINE_PATH}`);
  }

  if (mustFailures > 0) {
    console.log(
      '\n' +
        color('fail') +
        'FAILED' +
        reset() +
        ': non-baseline modules violate MUST-level requirements.',
    );
    process.exit(1);
  }

  console.log(
    '\n' +
      color('pass') +
      'PASSED' +
      reset() +
      ': all non-baseline modules satisfy the minimum engineering skeleton.',
  );
  process.exit(0);
}

if (import.meta.main) {
  main();
}
