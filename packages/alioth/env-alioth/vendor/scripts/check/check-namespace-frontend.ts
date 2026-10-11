#!/usr/bin/env bun
/// <reference types="bun" />

// ── 判定面注入（旧自解析 argv 移除；由基座 ctx 供给）──────────────────────
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
let SELF_TEST_MODE = false;
let NEXT_PORT: string | null = null;
let REQUIRE_CANDIDATES = false;
let SCAN_PATHS: readonly string[] = [];
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

import {
  existsSync,
  readFileSync,
  statSync,
  readdirSync,
  realpathSync,
  mkdtempSync,
  writeFileSync,
  rmSync,
} from "fs";
import { tmpdir } from "node:os";
import { fileURLToPath } from "node:url";
import { portSlotsFromPortsConf } from "../lib/table-code-registry.ts";
import { dirname, join, relative, resolve } from "path";
import { css, js } from "../lib/parsers.ts";
// @ts-ignore — 框架 vite 预设为 JS 模块（无 .d.ts），按其运行时契约使用
import { defineModuleViteConfig } from "../../Framework/frontend/config/vite/index.js";
// 单副本清单**唯一正本**（正本 = 预设侧的 `singletons.ts`；此处派生，MUST NOT 复制字面）
import { SINGLETON_DEDUPE as REQUIRED_SINGLETON_DEDUPE } from "../../Framework/frontend/config/vite/singletons.ts";
import { defineCheck } from "./lib/check-interface";

const __filename = fileURLToPath(import.meta.url);
const __dirname = resolve(__filename, "..");
const REPO_ROOT = resolve(__dirname, "../..");
const BASELINE_PATH = join(
  REPO_ROOT,
  "scripts/check/.namespace-frontend-baseline.txt",
);

const FRONTEND_GLOBS = [
  // 模块前端（两代路径；平台根 `Modules/` 位已随旧 workspace 清理退役）
  "Pre-Proc/*/Sources/Modules/*/frontend",
  "Pre-Proc/*/Sources/Apps/Modules/*/frontend",
  // Block 前端（同一工程契约：package.json / vite.config.ts / tsconfig / 必需源文件 / 端口带）——
  // capability `frontend-namespace-consistency :: module-frontend-required-source-files-exist`
  // 明文覆盖 Block（"Every Block frontend, like every Module frontend, MUST contain the required source files"），
  // 此前未纳入扫描面 ⇒ 80 个 Block 的工程面长期无判据。
  "Pre-Proc/*/Sources/Apps/Blocks/*/frontend",
  "Pre-Proc/*/Sources/Blocks/*/frontend",
];

const REQUIRED_FILES = [
  "src/App.tsx",
  "src/theme.css",
  "src/locales/zh-CN.json",
  "src/locales/en.json",
];

const ENTRY_FILES = ["src/single-spa.tsx", "src/main.tsx"];

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
const TSX_TRANSPILER = new Bun.Transpiler({ loader: "tsx" });
const IMPORT_WALK_MAX_DEPTH = 3;

function importGraphReaches(entryAbs: string, targetSuffix: string): boolean {
  const seen = new Set<string>();
  const queue: Array<{ file: string; depth: number }> = [
    { file: entryAbs, depth: 0 },
  ];
  while (queue.length > 0) {
    const item = queue.shift();
    if (!item) break;
    if (seen.has(item.file)) continue;
    seen.add(item.file);
    let code: string;
    try {
      code = readFileSync(item.file, "utf8");
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
      if (item.depth >= IMPORT_WALK_MAX_DEPTH || !rec.path.startsWith("."))
        continue;
      const base = resolve(dirname(item.file), rec.path);
      const next = [
        base,
        `${base}.tsx`,
        `${base}.ts`,
        join(base, "index.tsx"),
        join(base, "index.ts"),
      ].find((p) => existsSync(p) && statSync(p).isFile());
      if (next) queue.push({ file: next, depth: item.depth + 1 });
    }
  }
  return false;
}

// ── 模块 dev 端口带 ──────────────────────────────────────────────────────────
// 正本 = openspec/specs/dev-tooling :: module-dev-port-namespace-band
// 带 = `6500 + 100 * idx(ns)`，idx = ns 在 `Deploy/ports.conf` 的 0 基行序（不新增表行：
// 该表每行恰 4 列，加行即破坏 `check-ports-conf.ts`）。起 6500 的判据：5xxx 会覆盖
// PostgreSQL 5432 与 Vite 默认 5173，镜像站 6464 亦在 6xxx；6500-6999 实测无监听语义占用。
const MODULE_PORT_BASE = 6500;
const MODULE_PORT_SPAN = 100;

/** 禁落端口区间（已被别的服务语义占用；命中即 fail，不只是「越带」）。 */
const FORBIDDEN_MODULE_PORTS: readonly {
  min: number;
  max: number;
  why: string;
}[] = [
  { min: 5432, max: 5432, why: "PostgreSQL" },
  { min: 5173, max: 5173, why: "Vite 默认端口" },
  { min: 6464, max: 6464, why: "自建镜像站（本机 nginx 可能监听）" },
  { min: 8001, max: 8099, why: "OpenActivity BE 带" },
  { min: 9001, max: 9010, why: "Gateway BE/SSO 槽位" },
  { min: 31717, max: 31799, why: "OpenActivity FE 带" },
  {
    min: 41600,
    max: 41999,
    why: "Gateway FE 槽位区与带级预留端口（41716-41799）",
  },
];

/** 平台仓根：ns 仓内 `scripts` 是符号链接 ⇒ 平台面判定（`Deploy/ports.conf`）取 realpath 真身。 */
const PLATFORM_ROOT = existsSync(join(REPO_ROOT, "pnpm-workspace.yaml"))
  ? REPO_ROOT
  : dirname(realpathSync(join(REPO_ROOT, "scripts")));
const PORTS_CONF_PATH = join(PLATFORM_ROOT, "Deploy/ports.conf");

/** 槽位表行序（`#`/空行跳过，取首列）→ 模块端口带。未登记 ns 返回 null（判据不可用 ≠ 合规）。
 *  行序读取委托 `table-code-registry::portSlotsFromPortsConf`（Deploy/ports.conf 的 TS 侧唯一
 *  读取器；2026-10-11 前此处自持第二份逐行解析）；带公式常数与 bash 侧 ns-ports.sh
 *  `ns_module_port_band` 同源（MUST 同步，freshness 门禁守行序双侧一致）。 */
export function modulePortBand(
  ns: string,
  tablePath: string = PORTS_CONF_PATH,
): { min: number; max: number } | null {
  const order = portSlotsFromPortsConf(PLATFORM_ROOT, tablePath).map((r) => r.ns);
  const idx = order.indexOf(ns);
  if (idx < 0) return null;
  const min = MODULE_PORT_BASE + MODULE_PORT_SPAN * idx;
  return { min, max: min + MODULE_PORT_SPAN - 1 };
}

/** 从 vite.config.ts AST 取 `port` 数值字面量（工厂调用参数 `port: N` 或 `server: { port: N }` 两形态）。 */
export function extractDevPorts(ast: unknown): number[] {
  const out: number[] = [];
  walkAst(ast, (node) => {
    if (node.type !== "Property") return;
    const key = node.key as Record<string, unknown> | undefined;
    const keyName =
      key?.type === "Identifier"
        ? (key.name as string)
        : key?.type === "Literal"
          ? String(key.value)
          : "";
    if (keyName !== "port") return;
    const value = node.value as Record<string, unknown> | undefined;
    if (value?.type === "Literal" && typeof value.value === "number")
      out.push(value.value);
  });
  return out;
}

/** 模块配置是否注册了 Tailwind 插件（MUST NOT：唯一编译点 = Shell 的 `index.css`）。 */
export function hasTailwindPluginImport(ast: unknown): boolean {
  let hit = false;
  walkAst(ast, (node) => {
    if (node.type !== "ImportDeclaration") return;
    const src = node.source as Record<string, unknown> | undefined;
    const value = src?.value;
    if (value === "@tailwindcss/vite" || value === "@tailwindcss/postcss")
      hit = true;
  });
  return hit;
}

/** 模块 `src/single-spa.tsx` 是否经共享工厂导出生命周期（导入 + 调用两面）。 */
export function inspectSharedLifecycle(source: string): {
  imports: boolean;
  calls: boolean;
} {
  let imports = false;
  let calls = false;
  try {
    const transpiled = new Bun.Transpiler({ loader: "tsx" }).transformSync(
      source,
      { loader: "tsx" },
    );
    const ast = js.parseModule(transpiled) as unknown;
    walkAst(ast, (node) => {
      if (node.type === "ImportDeclaration") {
        const src = node.source as Record<string, unknown> | undefined;
        if (src?.value !== "@alioth/api") return;
        for (const s of (node.specifiers as
          Record<string, unknown>[] | undefined) ?? []) {
          const imported = s.imported as Record<string, unknown> | undefined;
          if (imported?.name === "createMicroAppLifecycle") imports = true;
        }
      }
      if (node.type === "CallExpression") {
        const callee = node.callee as Record<string, unknown> | undefined;
        if (
          callee?.type === "Identifier" &&
          callee.name === "createMicroAppLifecycle"
        )
          calls = true;
      }
    });
  } catch {
    return { imports: false, calls: false };
  }
  return { imports, calls };
}

/** 从 `src/single-spa.tsx` AST 取 `createMicroAppLifecycle({ moduleName: '…' })` 的字面量值。 */
export function extractModuleName(source: string): string | null {
  try {
    const transpiled = new Bun.Transpiler({ loader: "tsx" }).transformSync(
      source,
      { loader: "tsx" },
    );
    const ast = js.parseModule(transpiled) as unknown;
    let found: string | null = null;
    walkAst(ast, (node) => {
      if (node.type !== "Property") return;
      const key = node.key as Record<string, unknown> | undefined;
      if (key?.type !== "Identifier" || key.name !== "moduleName") return;
      const value = node.value as Record<string, unknown> | undefined;
      if (value?.type === "Literal" && typeof value.value === "string")
        found = value.value;
    });
    return found;
  } catch {
    return null;
  }
}

/** CSS 源是否 `@import "tailwindcss"`（模块侧出现 = 第二份 preflight/utilities）。 */
export function cssImportsTailwind(text: string): boolean {
  let hit = false;
  try {
    css.walk(css.parse(text), (node: Record<string, unknown>) => {
      if (node.type !== "Atrule" || node.name !== "import") return;
      const prelude = node.prelude ? css.generate(node.prelude as never) : "";
      if (prelude.includes("tailwindcss")) hit = true;
    });
  } catch {
    return false;
  }
  return hit;
}

// 与单副本清单（正本 `singletons.ts` 的 `SINGLETON_DEDUPE`）是**两个不同的判据面**，且**不构成嵌套**（2026-10-05 实测：
// 正本有 react/jsx-runtime/jsx-dev-runtime/react-router/jotai 不在本表；本表有 single-spa 与 @alioth/* 不在正本）。
// 「正本是本表的实例敏感子集」的旧叙述**不成立**，MUST NOT 据此新增子集判据。
const STANDARD_SHARED_RUNTIME: Record<string, true> = {
  react: true,
  "react-dom": true,
  "react-dom/client": true,
  "react-router-dom": true,
  jotai: true,
  "@tanstack/react-query": true,
  "@alioth/api": true,
  "@alioth/components": true,
  "@alioth/hooks": true,
  "@alioth/i18n": true,
  "@alioth/utils": true,
  "single-spa": true,
};

type CheckStatus = "pass" | "fail" | "warn" | "baseline";
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
  /** 模块声明的 dev 端口（供 main() 做同 ns 唯一性判定） */
  devPort?: number;
}

interface BaselineEntry {
  path: string;
  sha: string;
}

function readBaseline(): BaselineEntry[] {
  if (!existsSync(BASELINE_PATH)) return [];
  const text = readFileSync(BASELINE_PATH, "utf-8");
  const entries: BaselineEntry[] = [];
  for (const line of text.split(/\r?\n/)) {
    const trimmed = line.trim();
    if (!trimmed || trimmed.startsWith("#")) continue;
    const parts = trimmed.split(/\s+/);
    if (parts.length >= 2) {
      entries.push({ path: parts[0], sha: parts[1] });
    }
  }
  return entries;
}

function normalizeModulePath(p: string): string {
  return p.replace(/\\/g, "/").replace(/\/$/, "");
}

function isBaseline(modulePath: string, baseline: BaselineEntry[]): boolean {
  const rel = normalizeModulePath(relative(REPO_ROOT, modulePath));
  return baseline.some((b) => normalizeModulePath(b.path) === rel);
}

function parseJsonFile(path: string): unknown | Error {
  try {
    const text = readFileSync(path, "utf-8");
    return JSON.parse(text);
  } catch (err) {
    return err instanceof Error ? err : new Error(String(err));
  }
}

function getPropertyValue(
  node: Record<string, unknown> | undefined,
  name: string,
): Record<string, unknown> | undefined {
  if (!node || node.type !== "ObjectExpression") return undefined;
  const props = node.properties as Array<Record<string, unknown>> | undefined;
  if (!props) return undefined;
  for (const prop of props) {
    if (prop.type !== "Property" || prop.computed) continue;
    const key = prop.key as Record<string, unknown> | undefined;
    if (key && key.type === "Identifier" && key.name === name) {
      return prop.value as Record<string, unknown>;
    }
  }
  return undefined;
}

function getStringValues(node: Record<string, unknown> | undefined): string[] {
  if (!node || node.type !== "ArrayExpression") return [];
  const elements = node.elements as
    Array<Record<string, unknown> | null> | undefined;
  if (!elements) return [];
  return elements
    .map((el) => {
      if (!el) return undefined;
      if (el.type === "Literal" && typeof el.value === "string")
        return el.value as string;
      if (el.type === "TemplateLiteral") {
        const quasis = el.quasis as Array<Record<string, unknown>> | undefined;
        if (
          quasis?.length === 1 &&
          (el.expressions as unknown[] | undefined)?.length === 0
        ) {
          const cooked = (quasis[0].value as Record<string, unknown>).cooked;
          return typeof cooked === "string" ? cooked : undefined;
        }
      }
      return undefined;
    })
    .filter((v): v is string => v !== undefined);
}

/** 提取 external 数组中的正则字面量 source（如 /^@alioth(\/.*)?$/ → "^@alioth(\\/.*)?$"）。 */
export function getRegexLiteralSources(
  node: Record<string, unknown> | undefined,
): string[] {
  if (!node || node.type !== "ArrayExpression") return [];
  const elements = node.elements as
    Array<Record<string, unknown> | null> | undefined;
  if (!elements) return [];
  return elements
    .map((el) => {
      if (!el || el.type !== "Literal") return undefined;
      const regex = el.regex as Record<string, unknown> | undefined;
      // meriyah 的 regex.pattern 不含斜杠；还原为源码形态（/pattern/flags）供 regexCoversDep 解析
      return typeof regex?.pattern === "string"
        ? `/${regex.pattern}/${typeof regex.flags === "string" ? regex.flags : ""}`
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
    const prefix = mA[1].replace(/\\\//g, "/");
    return dep === prefix || dep.startsWith(prefix + "/");
  }
  const mB = regexSource.match(/^\/\^([A-Za-z0-9@._/-]+?)\\\/\//);
  if (mB) {
    const prefix = mB[1].replace(/\\\//g, "/");
    return dep.startsWith(prefix + "/");
  }
  return false;
}

/**
 * estree 节点递归遍历（meriyah 输出）。
 */
function walkAst(
  node: unknown,
  visit: (n: Record<string, unknown>) => void,
): void {
  if (!node || typeof node !== "object") return;
  const n = node as Record<string, unknown>;
  visit(n);
  for (const key of Object.keys(n)) {
    const v = n[key];
    if (Array.isArray(v)) {
      for (const c of v) walkAst(c, visit);
    } else if (
      v &&
      typeof v === "object" &&
      (v as Record<string, unknown>).type
    ) {
      walkAst(v, visit);
    }
  }
}

/**
 * 读取框架 vite 预设（`@alioth/config/vite`）的**真实**构建配置。
 *
 * external / lib.formats 的单一事实源在预设里：使用预设的模块不内联声明它们，
 * 因此门禁不能再只读内联 AST（否则收敛到预设的模块被误判 MUST 失败——2026-09-23
 * 实测：预设 external 覆盖全部声明共享依赖，内联读取却得到 "none"）。
 * 这里调用预获取其真值参与判定：预设若漏覆盖某依赖，门禁仍会如实报 FAIL。
 */
function presetResolvedConfig(
  modulePath: string,
): Record<string, unknown> | undefined {
  try {
    const moduleName = modulePath.split("/").slice(-2)[0];
    const produced: unknown = defineModuleViteConfig({
      rootDir: modulePath,
      moduleName,
    });
    const resolved =
      typeof produced === "function"
        ? (produced as (env: { mode: string; command: string }) => unknown)({
            mode: "production",
            command: "build",
          })
        : produced;
    return resolved as Record<string, unknown> | undefined;
  } catch (err) {
    // 静默失败会让「用了预设」的模块被误判缺失 external —— 必须可见。
    SINK.err(
      `[check-namespace-frontend] 读取框架预设失败（${modulePath}）: ${err instanceof Error ? err.message : err}`,
    );
    return undefined;
  }
}

/** build 面（external / lib.formats）——`presetResolvedConfig` 的窄视图。 */
function presetBuildConfig(
  modulePath: string,
): Record<string, unknown> | undefined {
  return presetResolvedConfig(modulePath)?.build as
    Record<string, unknown> | undefined;
}

/**
 * split-store 单例契约（change `move-ns-frontend-assembly-to-ns`，2026-10-05）：
 * 供给通道迁 ns 仓自持 store 后，「物理单 store」不再成立（模块 store 与平台包 `link:` 源码各持一份）
 * ⇒ 单副本只能由预设 `resolve.dedupe` 保证（构建与 vitest 双面消费同一清单）。
 * 判定集 = **派生自正本** `Framework/frontend/config/vite/singletons.ts` 的 `SINGLETON_DEDUPE`
 * （React 家族含 JSX 运行时 + router + 状态库）；旧「预设改列 MUST 同步此处」的手工约定已退役。
 */

function parseViteConfig(modulePath: string): {
  ast?: unknown;
  defaultExport?: Record<string, unknown>;
  error?: Error;
} {
  const vitePath = join(modulePath, "vite.config.ts");
  if (!existsSync(vitePath)) return {};
  try {
    const text = readFileSync(vitePath, "utf-8");
    // typescript@7（原生编译器）主入口不再提供 TS5 编译器 API；
    // 用 Bun.Transpiler 去类型后 meriyah 解析（标准解析器，符合 NO_REGEX 规约）
    const transpiled = new Bun.Transpiler().transformSync(text, {
      loader: "ts",
    });
    const ast = js.parseModule(transpiled) as unknown;
    let defaultExport: Record<string, unknown> | undefined;
    walkAst(ast, (node) => {
      if (node.type === "ExportDefaultDeclaration" && !defaultExport) {
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
    if (node.type !== "ImportDeclaration") return;
    const src = node.source as Record<string, unknown> | undefined;
    if (src?.value !== "@alioth/config/vite") return;
    const specifiers = node.specifiers as
      Array<Record<string, unknown>> | undefined;
    if (!specifiers) return;
    for (const s of specifiers) {
      const imported = s.imported as Record<string, unknown> | undefined;
      if (
        s.type === "ImportSpecifier" &&
        imported?.name === "defineModuleViteConfig"
      ) {
        found = true;
      }
    }
  });
  return found;
}

function checkModule(
  modulePath: string,
  baseline: BaselineEntry[],
): ModuleReport {
  const rel = relative(REPO_ROOT, modulePath);
  const segments = rel.split("/");
  const namespace = segments[1] ?? "";
  const moduleName = segments[segments.length - 2] ?? "";
  const baselineFlag = isBaseline(modulePath, baseline);
  const results: CheckResult[] = [];
  let devPort: number | undefined;

  const add = (name: string, status: CheckStatus, message: string) => {
    if (baselineFlag && status === "fail") {
      results.push({
        name,
        status: "baseline",
        message: `${message} (baseline-grandfathered)`,
      });
    } else {
      results.push({ name, status, message });
    }
  };

  const packageJsonPath = join(modulePath, "package.json");
  if (!existsSync(packageJsonPath)) {
    add("package.json", "fail", "missing package.json");
  } else {
    const parsed = parseJsonFile(packageJsonPath);
    if (parsed instanceof Error) {
      add("package.json", "fail", `invalid JSON: ${parsed.message}`);
    } else {
      add("package.json", "pass", "package.json exists and is valid JSON");
    }
  }

  let viteAst: unknown;

  const vitePath = join(modulePath, "vite.config.ts");
  if (!existsSync(vitePath)) {
    add("vite.config.ts", "fail", "missing vite.config.ts");
  } else {
    const { ast, defaultExport, error } = parseViteConfig(modulePath);
    if (error || !ast) {
      add(
        "vite.config.ts",
        "fail",
        `cannot parse: ${error?.message ?? "unknown error"}`,
      );
    } else {
      viteAst = ast;
      let configObject: Record<string, unknown> | undefined = defaultExport;
      if (configObject && configObject.type === "CallExpression") {
        const args = configObject.arguments as
          Array<Record<string, unknown>> | undefined;
        configObject = args?.[0];
      }

      const usesPreset = hasDefineModuleViteConfigImport(ast);
      // 预设使用时：lib.formats / rollupOptions.external 的事实源在预设里，读其**真值**参与
      // 判定（预设漏覆盖仍会 FAIL），而不是仅凭「用了预设」放行。
      const presetBuild = usesPreset
        ? presetBuildConfig(modulePath)
        : undefined;
      const presetLib = presetBuild?.lib as { formats?: unknown } | undefined;
      const presetRollup = presetBuild?.rollupOptions as
        { external?: unknown } | undefined;
      const buildNode = configObject
        ? getPropertyValue(configObject, "build")
        : undefined;
      const libNode = buildNode
        ? getPropertyValue(buildNode, "lib")
        : undefined;
      const formatsNode = libNode
        ? getPropertyValue(libNode, "formats")
        : undefined;
      const formats = usesPreset
        ? Array.isArray(presetLib?.formats)
          ? (presetLib.formats as string[])
          : []
        : getStringValues(formatsNode);
      if (!formats.includes("es")) {
        add(
          "vite.lib.formats",
          "fail",
          `build.lib.formats MUST include "es" (got: ${formats.join(", ") || "none"}${usesPreset ? " [来自框架预设]" : ""})`,
        );
      } else {
        add(
          "vite.lib.formats",
          "pass",
          usesPreset
            ? 'build.lib.formats includes "es"（来自框架预设）'
            : 'build.lib.formats includes "es"',
        );
      }

      const pkg = parseJsonFile(packageJsonPath);
      const declaredSharedDeps = new Set<string>();
      if (!(pkg instanceof Error) && typeof pkg === "object" && pkg !== null) {
        const deps = (pkg as Record<string, unknown>).dependencies;
        const peers = (pkg as Record<string, unknown>).peerDependencies;
        if (typeof deps === "object" && deps !== null) {
          Object.keys(deps).forEach((k) => {
            if (STANDARD_SHARED_RUNTIME[k]) declaredSharedDeps.add(k);
          });
        }
        if (typeof peers === "object" && peers !== null) {
          Object.keys(peers).forEach((k) => {
            if (STANDARD_SHARED_RUNTIME[k]) declaredSharedDeps.add(k);
          });
        }
      }

      const rollupOptions = buildNode
        ? getPropertyValue(buildNode, "rollupOptions")
        : undefined;
      const externalNode = rollupOptions
        ? getPropertyValue(rollupOptions, "external")
        : undefined;
      // Rollup 允许 external 使用正则字面量（如 /^@alioth(\/.*)?$/），此处同时提取正则
      // 的 source 以便识别可证明覆盖的前缀。使用框架预设时 external 的事实源在预设里
      // （真值 string | RegExp），一并纳入覆盖判定。
      const presetExternalValues =
        usesPreset && Array.isArray(presetRollup?.external)
          ? (presetRollup.external as unknown[])
          : [];
      const externals = [
        ...getStringValues(externalNode),
        ...presetExternalValues.filter(
          (v): v is string => typeof v === "string",
        ),
      ];
      const regexExternals: string[] = [
        ...getRegexLiteralSources(externalNode),
        // 真实 RegExp → 还原为 regexCoversDep 期望的源码形态（/pattern/flags）
        ...presetExternalValues
          .filter((v): v is RegExp => v instanceof RegExp)
          .map((re) => `/${re.source}/${re.flags}`),
      ];
      const externalSet = new Set(externals);
      const missingExternals: string[] = [];
      for (const dep of declaredSharedDeps) {
        let covered = externalSet.has(dep);
        if (!covered) {
          for (const ext of externalSet) {
            if (
              ext === dep ||
              (ext.endsWith("/*") && dep.startsWith(ext.slice(0, -1)))
            ) {
              covered = true;
              break;
            }
            if (ext === "@alioth/*" && dep.startsWith("@alioth/")) {
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
          "vite.externals",
          "fail",
          `missing externals for declared deps: ${missingExternals.join(", ")}`,
        );
      } else {
        add(
          "vite.externals",
          "pass",
          "all declared dependencies/peerDependencies are externalized",
        );
      }

      if (hasDefineModuleViteConfigImport(ast)) {
        add(
          "vite.shared-preset",
          "pass",
          "uses defineModuleViteConfig from @alioth/config/vite",
        );
      } else {
        add(
          "vite.shared-preset",
          "fail",
          "MUST export defineModuleViteConfig from @alioth/config/vite（手写配置 = React 单副本锚、共享 external 清单与条件化入口失守）",
        );
      }

      // split-store 单例契约（判据面 = REQUIRED_SINGLETON_DEDUPE）：用预设的模块，其单副本保证是预设的
      // `resolve.dedupe`——供给通道迁 ns 自持 store 后这是**唯一**保证（物理单 store 不再成立）。
      if (usesPreset) {
        const dedupeRaw = (
          presetResolvedConfig(modulePath)?.resolve as
            { dedupe?: unknown } | undefined
        )?.dedupe;
        const dedupe = Array.isArray(dedupeRaw)
          ? dedupeRaw.filter((v): v is string => typeof v === "string")
          : [];
        const missingSingleton = REQUIRED_SINGLETON_DEDUPE.filter(
          (d) => !dedupe.includes(d),
        );
        if (missingSingleton.length > 0) {
          add(
            "vite.react-singleton",
            "fail",
            `preset resolve.dedupe missing: ${missingSingleton.join(", ")}（React/router/状态库双副本 ⇒ hooks/上下文跨实例失配）`,
          );
        } else {
          add(
            "vite.react-singleton",
            "pass",
            "preset dedupe covers react/router/state singletons",
          );
        }
      }
    }
  }

  // Tailwind 单编译点（MUST NOT）：模块侧注册插件或在模块 CSS 内 `@import "tailwindcss"`
  // 都会在 Shell（`Gateway/frontend/src/index.css` 的 `@source` glob）之外再产一份
  // preflight/utilities ⇒ 与其余 ns 不一致且样式双份。
  {
    const pluginRegistered = viteAst ? hasTailwindPluginImport(viteAst) : false;
    const cssHits: string[] = [];
    const srcDir = join(modulePath, "src");
    if (existsSync(srcDir)) {
      const glob = new Bun.Glob("**/*.css");
      for (const hit of glob.scanSync({ cwd: srcDir, onlyFiles: true })) {
        const text = readFileSync(join(srcDir, hit), "utf8");
        if (cssImportsTailwind(text)) cssHits.push(`src/${hit}`);
      }
    }
    if (pluginRegistered || cssHits.length > 0) {
      const parts = [
        pluginRegistered ? "vite.config.ts 注册了 Tailwind 插件" : "",
        cssHits.length > 0
          ? `${cssHits.join(", ")} 内 @import "tailwindcss"`
          : "",
      ].filter(Boolean);
      add(
        "tailwind.single-compile-point",
        "fail",
        `Tailwind MUST 只由 Shell 编译；模块侧命中：${parts.join("；")}`,
      );
    } else {
      add(
        "tailwind.single-compile-point",
        "pass",
        "no module-level Tailwind compile point",
      );
    }
  }

  // dev 端口带（MUST）：本 ns 带内且不落已占区间；同 ns 唯一性在 main() 汇总判定。
  {
    const band = modulePortBand(namespace);
    const ports = viteAst ? extractDevPorts(viteAst) : [];
    if (band === null) {
      add(
        "dev-port.band",
        "warn",
        `ns ${namespace} 未登记于 Deploy/ports.conf，端口带判据不可用（未登记 ≠ 合规）`,
      );
    } else if (ports.length === 0) {
      add(
        "dev-port.band",
        "fail",
        `vite.config.ts 未声明 server.port（模块 dev server MUST 落 ${namespace} 带 ${band.min}-${band.max}）`,
      );
    } else if (ports.length > 1) {
      add(
        "dev-port.band",
        "fail",
        `vite.config.ts 端口字面量不唯一：${ports.join(", ")}`,
      );
    } else {
      const port = ports[0];
      const forbidden = FORBIDDEN_MODULE_PORTS.find(
        (f) => port >= f.min && port <= f.max,
      );
      if (forbidden) {
        add(
          "dev-port.band",
          "fail",
          `server.port ${port} 落已占区间 ${forbidden.min}-${forbidden.max}（${forbidden.why}）`,
        );
      } else if (port < band.min || port > band.max) {
        add(
          "dev-port.band",
          "fail",
          `server.port ${port} 越带（${namespace} 带 = ${band.min}-${band.max}）`,
        );
      } else {
        add(
          "dev-port.band",
          "pass",
          `server.port ${port} ∈ ${namespace} 带 ${band.min}-${band.max}`,
        );
        devPort = port;
      }
    }
  }

  const tsconfigPath = join(modulePath, "tsconfig.json");
  if (!existsSync(tsconfigPath)) {
    add("tsconfig.json", "fail", "missing tsconfig.json");
  } else {
    const parsed = parseJsonFile(tsconfigPath);
    if (parsed instanceof Error) {
      add("tsconfig.json", "fail", `invalid JSON: ${parsed.message}`);
    } else {
      add("tsconfig.json", "pass", "tsconfig.json exists and is valid JSON");
    }
  }

  for (const file of REQUIRED_FILES) {
    const filePath = join(modulePath, file);
    const key = file.replace(/^src\//, "").replace(/\//g, ".");
    if (!existsSync(filePath)) {
      add(`file.${key}`, "fail", `missing ${file}`);
    } else {
      add(`file.${key}`, "pass", `${file} exists`);
    }
  }

  const entryPath = ENTRY_FILES.find((f) => existsSync(join(modulePath, f)));
  if (!entryPath) {
    add("entry", "fail", `missing entry file (${ENTRY_FILES.join(" or ")})`);
  } else {
    add("entry", "pass", `entry file ${entryPath} exists`);
    if (entryPath === "src/main.tsx") {
      add(
        "entry.canonical",
        "warn",
        "SHOULD use src/single-spa.tsx as canonical entry",
      );
    }
  }

  // css.entry-reachability（MUST）：库构建入口链必须触达 src/theme.css，
  // 否则 vite lib build 不产出 dist/theme.css（线上模块无样式，见脚本头注释）。
  {
    const libEntries = ["src/single-spa.tsx", "src/App.tsx"]
      .map((f) => join(modulePath, f))
      .filter((p) => existsSync(p) && statSync(p).isFile());
    if (libEntries.length === 0) {
      add(
        "css.entry-reachability",
        "warn",
        "no lib entry candidate (src/single-spa.tsx | src/App.tsx)",
      );
    } else if (libEntries.some((p) => importGraphReaches(p, "theme.css"))) {
      add(
        "css.entry-reachability",
        "pass",
        "src/theme.css reachable from lib entry chain",
      );
    } else {
      add(
        "css.entry-reachability",
        "fail",
        "src/theme.css NOT reachable from lib entry (src/single-spa.tsx | src/App.tsx) — vite lib build 不会产出 theme.css",
      );
    }
  }

  const zhPath = join(modulePath, "src/locales/zh-CN.json");
  const enPath = join(modulePath, "src/locales/en.json");
  // 命名空间判据：键前缀 MUST 逐字等于**本单元目录名**，或（Block）其 `block.json::sharing.ownerModule` 的
  // 模块 id —— 后者是「block 与所属模块共享命名空间」的合法形态（Alioth/Cosmic-Tools 的
  // system-settings 子 block 即此：键为 `system-settings.*`，theme.css 亦注释「shares system-settings tokens」）。
  // 通用前缀（`block.` / `list.`）不在白名单：字典会被 Shell 合并，泛化键跨单元冲突。
  const namespacePrefixes = [moduleName];
  {
    const blockJsonPath = join(modulePath, "..", "block.json");
    if (existsSync(blockJsonPath)) {
      const parsedBlock = parseJsonFile(blockJsonPath);
      let owner = "";
      if (
        typeof parsedBlock === "object" &&
        parsedBlock !== null &&
        "sharing" in parsedBlock
      ) {
        const sharing = parsedBlock.sharing;
        if (
          typeof sharing === "object" &&
          sharing !== null &&
          "ownerModule" in sharing
        ) {
          const raw = sharing.ownerModule;
          if (typeof raw === "string") owner = raw;
        }
      }
      const ownerId = owner.includes("/") ? owner.split("/").pop() : owner;
      if (ownerId) namespacePrefixes.push(ownerId);
    }
  }
  const expectedPrefixes = namespacePrefixes.map((p) => `${p}.*`).join(" 或 ");
  for (const [path, label] of [
    [zhPath, "locales.zh-CN"],
    [enPath, "locales.en"],
  ] as const) {
    if (!existsSync(path)) continue;
    const parsed = parseJsonFile(path);
    if (parsed instanceof Error) {
      add(label, "fail", `invalid JSON: ${parsed.message}`);
    } else if (typeof parsed !== "object" || parsed === null) {
      add(label, "fail", "locale file must be a JSON object");
    } else {
      const keys = Object.keys(parsed);
      const hasUnitKey = keys.some((k) =>
        namespacePrefixes.some((p) => k.startsWith(`${p}.`)),
      );
      if (keys.length === 0) {
        add(label, "fail", "locale file is empty");
      } else if (!hasUnitKey) {
        add(
          label,
          "warn",
          `locale file has no key matching "${expectedPrefixes}"`,
        );
      } else {
        add(label, "pass", `locale file has ${keys.length} keys`);
      }
    }
  }

  // entry.shared-lifecycle（MUST）：`src/single-spa.tsx` 必须经共享工厂导出生命周期。
  {
    const spaPath = join(modulePath, "src/single-spa.tsx");
    if (existsSync(spaPath)) {
      const spaSource = readFileSync(spaPath, "utf8");
      const probe = inspectSharedLifecycle(spaSource);
      if (probe.imports && probe.calls) {
        add(
          "entry.shared-lifecycle",
          "pass",
          "entry exports createMicroAppLifecycle from @alioth/api",
        );
      } else {
        add(
          "entry.shared-lifecycle",
          "fail",
          `src/single-spa.tsx MUST 经 createMicroAppLifecycle(@alioth/api) 导出（import=${probe.imports}, call=${probe.calls}）——手写 createRoot 会丢 QueryClient/Jotai/I18n Provider 与 RouterSync`,
        );
      }
      // entry.module-name（MUST）：moduleName MUST 逐字等于本单元目录名——它决定容器 scoped 类
      // `mod-<name>`（theme.css 选择器依赖）与 lifecycle 日志前缀；实测曾有复制粘贴遗留
      // （WZ 多 block 的 moduleName 全为 `income-receivable`）。
      const declaredName = extractModuleName(spaSource);
      if (declaredName === null) {
        add(
          "entry.module-name",
          "fail",
          "src/single-spa.tsx 未声明字面量 moduleName",
        );
      } else if (declaredName !== moduleName) {
        add(
          "entry.module-name",
          "fail",
          `moduleName '${declaredName}' ≠ 目录名 '${moduleName}'（scoped 类 mod-* 与 theme.css 选择器将失配）`,
        );
      } else {
        add(
          "entry.module-name",
          "pass",
          `moduleName '${declaredName}' 与目录名一致`,
        );
      }
    }
  }

  return {
    namespace,
    module: moduleName,
    path: rel,
    baseline: baselineFlag,
    results,
    devPort,
  };
}

function discoverModuleFrontends(extraPaths: string[] = []): string[] {
  const found = new Set<string>();

  // Default glob patterns
  for (const pattern of FRONTEND_GLOBS) {
    const dirPattern = pattern.endsWith("/") ? pattern : `${pattern}/`;
    const glob = new Bun.Glob(dirPattern);
    // `followSymlinks: true` 必需：`Pre-Proc/{ns}` 是符号链接（指向各 `Alioth-NS-*` 仓），
    // Bun.Glob 默认不跟随符号链接 ⇒ 深层模式恒零匹配 ⇒ 本检查在 namespace 拆分后**失效**
    // （2026-09-25 实证：`Pre-Proc/*` 匹配 6 项、`Pre-Proc/*/Sources` 为 0、加该选项后为 5）。
    for (const match of glob.scanSync({
      cwd: REPO_ROOT,
      absolute: true,
      onlyFiles: false,
      followSymlinks: true,
    })) {
      if (match.endsWith("/frontend") || match.endsWith("\\frontend")) {
        found.add(match);
      }
    }
  }

  // 过滤迁移残留空壳（仅剩 node_modules/dist 等生成物）。
  //
  // 判据 = **是否含完整模块骨架**（REQUIRED_FILES：src/App.tsx + theme.css + locales），
  // MUST NOT 用「是否被 git 跟踪」判据：本仓 `Pre-Proc/**/frontend` 是生成物（不随 git 跟踪），
  // 按跟踪过滤会让本检查在**任何环境**都零候选 ⇒ 检查实际已死（2026-09-25 实证：
  // dev checkout 与干净 worktree 同样「No module frontend directories found」）。
  for (const dir of Array.from(found)) {
    const hasSkeleton = REQUIRED_FILES.every((f) => existsSync(join(dir, f)));
    if (!hasSkeleton) found.delete(dir);
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
    case "pass":
      return "\x1b[32m";
    case "fail":
      return "\x1b[31m";
    case "warn":
      return "\x1b[33m";
    case "baseline":
      return "\x1b[34m";
    default:
      return "";
  }
}

function reset(): string {
  return "\x1b[0m";
}

/** 自测（SPEC_ENFORCEMENT_SUBSTRATE §R1）：正例 + **负例**（证明豁免/判据未被放宽）。 */
function runSelfTest(): boolean {
  const results: { name: string; ok: boolean }[] = [];
  const check = (name: string, ok: boolean) => results.push({ name, ok });
  const astOf = (src: string): unknown =>
    js.parseModule(
      new Bun.Transpiler({ loader: "ts" }).transformSync(src, { loader: "ts" }),
    );

  const tmp = mkdtempSync(join(tmpdir(), "nsfe-selftest-"));
  try {
    const table = join(tmp, "ports.conf");
    writeFileSync(
      table,
      [
        "# namespace FE BE SSO",
        "Alioth 41717 9001 9002",
        "Cosmic-Tools 41718 9003 9004",
        "WZ 41719 9005 9006",
        "AVIC-CAASEC 41720 9007 9008",
        "SE 41721 9009 9010",
      ].join("\n") + "\n",
    );

    check(
      "带派生：Alioth（idx=0）⇒ 6500-6599",
      JSON.stringify(modulePortBand("Alioth", table)) ===
        '{"min":6500,"max":6599}',
    );
    check(
      "带派生：WZ（idx=2）⇒ 6700-6799",
      JSON.stringify(modulePortBand("WZ", table)) === '{"min":6700,"max":6799}',
    );
    check(
      "带派生：SE（idx=4）⇒ 6900-6999",
      JSON.stringify(modulePortBand("SE", table)) === '{"min":6900,"max":6999}',
    );
    check(
      "负例：未登记 ns ⇒ null（判据不可用 ≠ 合规）",
      modulePortBand("NoSuchNs", table) === null,
    );

    check(
      "端口抽取：工厂调用形态 port: 6500",
      JSON.stringify(
        extractDevPorts(
          astOf(
            "export default defineModuleViteConfig({ rootDir: import.meta.dirname, moduleName: 'x', port: 6500 });",
          ),
        ),
      ) === "[6500]",
    );
    check(
      "端口抽取：手写 server.port 形态",
      JSON.stringify(
        extractDevPorts(
          astOf("export default defineConfig({ server: { port: 5200 } });"),
        ),
      ) === "[5200]",
    );
    check(
      "负例：无 port 字面量 ⇒ 空（上层判 MUST 失败，不静默放行）",
      extractDevPorts(
        astOf("export default defineConfig({ plugins: [react()] });"),
      ).length === 0,
    );

    check(
      "负例：越带判定（Alioth 带 6500-6599 下 5200 越带）",
      ((): boolean => {
        const band = modulePortBand("Alioth", table)!;
        return 5200 < band.min || 5200 > band.max;
      })(),
    );
    check(
      "负例：已占端口判定（5432 PostgreSQL / 6464 镜像站命中禁落集；6700 带内端口不命中）",
      FORBIDDEN_MODULE_PORTS.filter((f) => 5432 >= f.min && 5432 <= f.max)
        .length === 1 &&
        FORBIDDEN_MODULE_PORTS.filter((f) => 6464 >= f.min && 6464 <= f.max)
          .length === 1 &&
        FORBIDDEN_MODULE_PORTS.filter((f) => 6700 >= f.min && 6700 <= f.max)
          .length === 0,
    );

    check(
      "负例：模块级 Tailwind 插件命中",
      hasTailwindPluginImport(
        astOf("import tailwindcss from '@tailwindcss/vite';"),
      ) &&
        !hasTailwindPluginImport(
          astOf("import react from '@vitejs/plugin-react';"),
        ),
    );
    check(
      '负例：模块 CSS 内 @import "tailwindcss" 命中',
      cssImportsTailwind('@import "tailwindcss";') &&
        !cssImportsTailwind(".a { color: red; }"),
    );

    const shared =
      "import { createMicroAppLifecycle } from '@alioth/api';\nexport const { bootstrap, mount, unmount } = createMicroAppLifecycle({ moduleName: 'x', App });";
    const handRolled =
      "import { createRoot } from 'react-dom/client';\nexport async function mount(props) { createRoot(props.domElement).render(App()); }";
    check(
      "负例：手写 createRoot 生命周期不被放行",
      (() => {
        const a = inspectSharedLifecycle(shared);
        const b = inspectSharedLifecycle(handRolled);
        return a.imports && a.calls && !b.imports && !b.calls;
      })(),
    );
    check(
      "entry.module-name 抽取：字面量取真值、缺失/非字面量 ⇒ null（漂移可判）",
      extractModuleName(
        "createMicroAppLifecycle({ moduleName: 'income-receivable', App })",
      ) === "income-receivable" &&
        extractModuleName(
          "createMicroAppLifecycle({ moduleName: name, App })",
        ) === null &&
        extractModuleName(
          "export function mount() { createRoot(el).render(App()); }",
        ) === null,
    );
  } finally {
    rmSync(tmp, { recursive: true, force: true });
  }

  for (const r of results) SINK.out(`  ${r.ok ? "PASS" : "FAIL"}  ${r.name}`);
  const failed = results.filter((r) => !r.ok).length;
  SINK.out(`\nself-test: ${results.length - failed}/${results.length} passed`);
  return failed === 0;
}

/** `--next-port <ns>`：该 ns 带内下一个空闲模块 dev 端口（单一实现 = 本判据；生成器与
 *  `namespace-ports.sh next-module` 均调此处，避免第二套分配逻辑）。 */
function printNextModulePort(ns: string): number {
  const band = modulePortBand(ns);
  if (band === null) {
    SINK.err(`[next-port] ns ${ns} 未登记于 Deploy/ports.conf，无法分配端口带`);
    exit(1);
  }
  const used = new Set<number>();
  for (const modulePath of discoverModuleFrontends()) {
    if ((relative(REPO_ROOT, modulePath).split("/")[1] ?? "") !== ns) continue;
    const { ast } = parseViteConfig(modulePath);
    if (!ast) continue;
    for (const p of extractDevPorts(ast)) used.add(p);
  }
  for (let port = band.min; port <= band.max; port += 1) {
    if (!used.has(port)) return port;
  }
  SINK.err(`[next-port] ns ${ns} 带 ${band.min}-${band.max} 已无空闲端口`);
  exit(1);
}

function runBody() {
  if (SELF_TEST_MODE) {
    exit(runSelfTest() ? 0 : 1);
  }
  if (NEXT_PORT !== null) {
    const ns = NEXT_PORT;
    if (!ns) {
      SINK.err("[next-port] 用法：--next-port <namespace>");
      exit(1);
    }
    SINK.out(String(printNextModulePort(ns)));
    exit(0);
  }
  const extraPaths: string[] = [...SCAN_PATHS];
  /** `--require-candidates`：本环境**必须**存在模块前端（缺失即失败）；默认缺失视为「无可检对象」放行。 */
  const requireCandidates = REQUIRE_CANDIDATES;

  const baseline = readBaseline();
  const modulePaths = discoverModuleFrontends(extraPaths);

  if (modulePaths.length === 0) {
    // 无候选 = 「本快照没有可校验的模块前端」（本仓 Pre-Proc/**/frontend 内容不随 git 跟踪，
    // 干净 worktree 与 dev checkout 都可能为空）。门禁类检查 MUST NOT 因「待检对象不存在」判失败：
    // 旧实现 exit(1) 令本检查在任何干净快照恒失败，被 reference-transaction 用作
    // blocking 门禁后**阻断全部 main 更新**（2026-09-25 实证）。显式 `--scan <path>` 仍严格
    // （调用方声明了期望路径，路径缺失即失败），另提供 `--require-candidates` 给「本环境必须有模块」的场景。
    SINK.err("No module frontend directories found.");
    exit(requireCandidates || extraPaths.length > 0 ? 1 : 0);
  }

  const reports = modulePaths.map((p) => checkModule(p, baseline));

  // dev-port.unique（MUST）：同 ns 内模块 dev 端口必须唯一（跨模块判定，故在汇总面做）。
  {
    const byNs = new Map<string, Map<number, string[]>>();
    for (const r of reports) {
      if (r.devPort === undefined) continue;
      const ports = byNs.get(r.namespace) ?? new Map<number, string[]>();
      byNs.set(r.namespace, ports);
      ports.set(r.devPort, [...(ports.get(r.devPort) ?? []), r.module]);
    }
    for (const [ns, ports] of byNs) {
      for (const [port, mods] of ports) {
        if (mods.length < 2) continue;
        for (const r of reports) {
          if (r.namespace !== ns || !mods.includes(r.module)) continue;
          const message = `server.port ${port} 同 ns 重复：${mods.join(", ")}`;
          r.results.push(
            r.baseline
              ? {
                  name: "dev-port.unique",
                  status: "baseline",
                  message: `${message} (baseline-grandfathered)`,
                }
              : { name: "dev-port.unique", status: "fail", message },
          );
        }
      }
    }
  }

  let mustFailures = 0;
  let warnings = 0;
  let baselineCount = 0;

  for (const report of reports) {
    const ns = report.namespace || "(unknown)";
    SINK.out(
      `\n# ${ns} / ${report.module}${report.baseline ? " [baseline]" : ""}`,
    );
    SINK.out(`  path: ${report.path}`);
    for (const r of report.results) {
      const label =
        r.status === "baseline" ? "BASELINE" : r.status.toUpperCase();
      SINK.out(
        `  ${color(r.status)}${label}${reset()}  ${r.name}: ${r.message}`,
      );
      if (r.status === "fail") mustFailures++;
      if (r.status === "warn") warnings++;
      if (r.status === "baseline") baselineCount++;
    }
  }

  SINK.out("\n" + "=".repeat(60));
  SINK.out(`Modules scanned: ${reports.length}`);
  SINK.out(`MUST failures: ${mustFailures}`);
  SINK.out(`Baseline-grandfathered: ${baselineCount}`);
  SINK.out(`Warnings: ${warnings}`);
  if (baseline.length > 0) {
    SINK.out(`Baseline file: ${BASELINE_PATH}`);
  }

  if (mustFailures > 0) {
    SINK.out(
      "\n" +
        color("fail") +
        "FAILED" +
        reset() +
        ": non-baseline modules violate MUST-level requirements.",
    );
    exit(1);
  }

  SINK.out(
    "\n" +
      color("pass") +
      "PASSED" +
      reset() +
      ": all non-baseline modules satisfy the minimum engineering skeleton.",
  );
  exit(0);
}

interface NamespaceFrontendOutcome {
  out: string[];
  err: string[];
  rc: number;
  blocked: boolean;
}

/** 判定面（ns 模块前端工程骨架 MUST 集 + dev 端口唯一性 + 基线归线）；输出经 SINK 记录，报告面回放。 */
export function namespaceFrontendState(opts: {
  selfTestMode: boolean;
  nextPort: string | null;
  requireCandidates: boolean;
  scanPaths: readonly string[];
}): NamespaceFrontendOutcome {
  const OUT: string[] = [];
  const ERR: string[] = [];
  const saved = SINK;
  SINK = { out: (l) => OUT.push(l), err: (l) => ERR.push(l) };
  SELF_TEST_MODE = opts.selfTestMode;
  NEXT_PORT = opts.nextPort;
  REQUIRE_CANDIDATES = opts.requireCandidates;
  SCAN_PATHS = opts.scanPaths;
  let rc = 0;
  try {
    runBody();
  } catch (e) {
    if (!(e instanceof ExitSignal)) throw e;
    rc = e.rc;
  } finally {
    SINK = saved;
  }
  return { out: OUT, err: ERR, rc, blocked: false };
}

/** 最近一次判定（同步判定核 ⇒ 同步报告面复用） */
let LAST: NamespaceFrontendOutcome = {
  out: [],
  err: [],
  rc: 0,
  blocked: false,
};

const gate = defineCheck({
  id: "namespace-frontend",
  label: "check-namespace-frontend",
  flags: ["--require-candidates"],
  valueFlags: ["--next-port", "--scan"],
  predicate: (ctx) => {
    LAST = namespaceFrontendState({
      selfTestMode: false,
      nextPort: ctx.valueFlags.get("--next-port")?.[0] ?? null,
      requireCandidates: ctx.flags.has("--require-candidates"),
      scanPaths: ctx.valueFlags.get("--scan") ?? [],
    });
    return [];
  },
  report: (_ctx, _res, io) => {
    const st = LAST;
    for (const line of st.out) io.out(`${line}\n`);
    for (const line of st.err) io.err(`${line}\n`);
    return st.rc; // MUST 违例恒阻断（旧实现不看 --fail）
  },
  selfTest() {
    const st = namespaceFrontendState({
      selfTestMode: true,
      nextPort: null,
      requireCandidates: false,
      scanPaths: [],
    });
    for (const line of st.out) process.stdout.write(`${line}\n`);
    for (const line of st.err) process.stderr.write(`${line}\n`);
    return st.rc;
  },
});

if (import.meta.main) process.exit(await gate.main());
