#!/usr/bin/env bun
/// <reference types="bun" />
/**
 * check-nav-hrefs.ts
 *
 * 静态回归守卫：模块 sidebar nav item 的 href 不变量。
 *
 * 背景：Gateway `Navigation.tsx` 以 URL 首段作为模块挂载前缀拼接跳转目标
 * `/{baseSegment}/{normalizedHref}`。该契约要求每个 nav item 的有效跳转目标
 * （href ?? id）归一化后为单段 kebab-case。违反时出现：前缀翻倍（404）、
 * Gateway wildcard 命中、或模块重挂载后内容区空白。
 *
 * 检查目标（运行时真实来源）：`Pre-Proc/*\/Sources/Modules/*\/frontend/src/App.tsx`
 * 中传给 `createModuleLayout` 的 `useNavItems()` 返回的对象字面量数组。
 * （运行时契约：`createModuleLayout` 仅在同时传入 `blockAssembly`+`blockNavKeys`
 * 时才从 module.json 派生 nav，否则以 `useNavItems` 为准。）
 *
 * 级别：
 *   fail — href/id 缺失、非单段 kebab、模块内重复（破坏上述路由契约）
 *   warn — href 为计算表达式（无法静态判定）、createModuleLayout 无 useNavItems
 *   skip — 模块前端无 createModuleLayout 用法
 *
 * 用法：
 *   bun scripts/check/check-nav-hrefs.ts           全量审计（非阻塞，exit 0）
 *   bun scripts/check/check-nav-hrefs.ts --fail    严格模式（fail 级违规 exit 1，基线内降级）
 *   bun scripts/check/check-nav-hrefs.ts --fail --ns WZ   限定 namespace（AppAgent gate 作用域）
 *
 * 基线：scripts/check/.nav-hrefs-baseline.txt —— 基板两列格式
 * `<模块前端路径> <baseline_sha>`（每行一条）；条目内模块自 sha 起未变更时，
 * 其 fail 级违规降级为 baseline 报告（不阻断）；已变更则按正常检查。
 */

import { existsSync, readFileSync, readdirSync } from "fs";
import { spawnSync } from "node:child_process";
import { fileURLToPath } from "node:url";
import { join, resolve } from "path";
import { parseModule } from "meriyah";

// 解析链：Bun.Transpiler 结构化擦除 TS 类型（tsx→js）→ meriyah ESTree AST。
// 避免依赖 TypeScript compiler API 与聚合 parser 桶文件的重模块图。

const __dirname = resolve(fileURLToPath(import.meta.url), "..");
const REPO_ROOT = resolve(__dirname, "../..");
const BASELINE_PATH = join(REPO_ROOT, "scripts/check/.nav-hrefs-baseline.txt");
const STRICT = process.argv.includes("--fail");
const nsFlagIdx = process.argv.indexOf("--ns");
const NS_FILTER = nsFlagIdx >= 0 ? process.argv[nsFlagIdx + 1] : undefined;

const KEBAB_SINGLE = /^[a-z][a-z0-9]*(-[a-z0-9]+)*$/;
const tsxTranspiler = new Bun.Transpiler({ loader: "tsx" });

type EstNode = {
  type: string;
  [key: string]: unknown;
};

type Level = "fail" | "warn" | "skip" | "baseline" | "pass";
interface Finding {
  level: Level;
  module: string; // 相对仓库根的模块目录
  item?: string;
  message: string;
}

/** 基板两列格式：`<path> <baseline_sha>`（每行一个模块前端路径，相对仓库根） */
function loadBaseline(): Map<string, string> {
  const map = new Map<string, string>();
  if (!existsSync(BASELINE_PATH)) return map;
  for (const raw of readFileSync(BASELINE_PATH, "utf-8").split("\n")) {
    const line = raw.trim();
    if (!line || line.startsWith("#")) continue;
    const parts = line.split(/\s+/);
    if (parts.length >= 2) map.set(parts[0], parts[1]);
  }
  return map;
}

/** 模块自 baseline sha 起是否有变更（与 guard-pre-delivery.py 的 _has_changed_since 同语义） */
function hasChangedSince(dirRel: string, sha: string): boolean {
  const r = spawnSync("git", ["diff", "--name-only", `${sha}..HEAD`, "--", dirRel], {
    cwd: REPO_ROOT,
    encoding: "utf-8",
    timeout: 30000,
  });
  return (r.stdout ?? "").trim().length > 0;
}

/** 收集所有模块前端目录：Pre-Proc/{ns}/Sources/Modules/{mod}/frontend；--ns 时限定单个 namespace */
function discoverModuleFrontends(): string[] {
  const out: string[] = [];
  const preProc = join(REPO_ROOT, "Pre-Proc");
  if (!existsSync(preProc)) return out;
  for (const ns of readdirSync(preProc)) {
    if (NS_FILTER && ns !== NS_FILTER) continue;
    const modulesDir = join(preProc, ns, "Sources", "Modules");
    if (!existsSync(modulesDir)) continue;
    for (const mod of readdirSync(modulesDir)) {
      const fe = join(modulesDir, mod, "frontend");
      if (existsSync(join(fe, "src"))) out.push(fe);
    }
  }
  return out.sort();
}

interface NavItemLit {
  id?: string;
  href?: string;
  hrefComputed: boolean;
}

/** 通用 ESTree 遍历（先序） */
function walk(node: EstNode | null | undefined, visit: (n: EstNode) => void): void {
  if (!node || typeof node !== "object") return;
  visit(node);
  for (const value of Object.values(node)) {
    if (Array.isArray(value)) {
      for (const child of value) walk(child as EstNode, visit);
    } else if (value && typeof value === "object" && (value as EstNode).type) {
      walk(value as EstNode, visit);
    }
  }
}

/** 从 tsx 源码提取 useNavItems 返回数组中的 { id, href } 字面量；null = 未找到 createModuleLayout */
function extractNavItems(source: string, filePath: string): { items: NavItemLit[]; hasUseNavItems: boolean } | null {
  const js = tsxTranspiler.transformSync(source);
  const ast = parseModule(js, { next: true }) as unknown as EstNode;

  let hasCreateModuleLayout = false;
  let hasUseNavItems = false;
  const items: NavItemLit[] = [];

  const propName = (p: EstNode): string | undefined => {
    const key = p.key as EstNode | undefined;
    if (!key) return undefined;
    if (key.type === "Identifier") return key.name as string;
    if (key.type === "Literal") return String(key.value);
    return undefined;
  };
  const readArrayLiteral = (arr: EstNode): void => {
    for (const el of (arr.elements as EstNode[]) ?? []) {
      if (!el || el.type !== "ObjectExpression") continue;
      const lit: NavItemLit = { hrefComputed: false };
      for (const prop of (el.properties as EstNode[]) ?? []) {
        if (prop.type !== "Property") continue;
        const key = propName(prop);
        const val = prop.value as EstNode | undefined;
        if (!val) continue;
        if (key === "id" && val.type === "Literal") lit.id = String(val.value);
        if (key === "href") {
          if (val.type === "Literal") lit.href = String(val.value);
          else lit.hrefComputed = true;
        }
      }
      items.push(lit);
    }
  };
  const visitFunctionBody = (body: EstNode | undefined): void => {
    if (!body) return;
    if (body.type === "ArrayExpression") {
      readArrayLiteral(body);
      return;
    }
    if (body.type === "BlockStatement") {
      for (const stmt of (body.body as EstNode[]) ?? []) {
        const arg = stmt.argument as EstNode | undefined;
        if (stmt.type === "ReturnStatement" && arg && arg.type === "ArrayExpression") {
          readArrayLiteral(arg);
        }
      }
    }
  };

  walk(ast, (n) => {
    // createModuleLayout({...}) 调用与 options 中的 useNavItems
    if (
      n.type === "CallExpression" &&
      (n.callee as EstNode)?.type === "Identifier" &&
      ((n.callee as EstNode).name as string) === "createModuleLayout"
    ) {
      hasCreateModuleLayout = true;
      walk(n, (inner) => {
        if (inner.type === "Property" && propName(inner) === "useNavItems") hasUseNavItems = true;
      });
    }
    // function useNavItems() { return [...] }
    if (n.type === "FunctionDeclaration" && ((n.id as EstNode)?.name as string) === "useNavItems") {
      visitFunctionBody(n.body as EstNode);
    }
    // const useNavItems = () => [...] / () => { return [...] }
    if (n.type === "VariableDeclarator" && ((n.id as EstNode)?.name as string) === "useNavItems") {
      const init = n.init as EstNode | undefined;
      if (init && (init.type === "ArrowFunctionExpression" || init.type === "FunctionExpression")) {
        visitFunctionBody(init.body as EstNode);
      }
    }
  });

  if (!hasCreateModuleLayout) return null;
  return { items, hasUseNavItems };
}

function auditModule(feDir: string, baseline: Map<string, string>): Finding[] {
  const moduleRel = feDir.replace(`${REPO_ROOT}/`, "");
  const sha = baseline.get(moduleRel);
  const grandfathered = sha !== undefined && !hasChangedSince(moduleRel, sha);
  const demote = (f: Finding): Finding =>
    grandfathered && f.level === "fail" ? { ...f, level: "baseline" } : f;

  const appTsx = join(feDir, "src", "App.tsx");
  if (!existsSync(appTsx)) {
    return [{ level: "skip", module: moduleRel, message: "无 src/App.tsx" }];
  }
  const extracted = extractNavItems(readFileSync(appTsx, "utf-8"), appTsx);
  if (!extracted) {
    return [{ level: "skip", module: moduleRel, message: "未使用 createModuleLayout" }];
  }
  if (!extracted.hasUseNavItems) {
    return [
      demote({
        level: "warn",
        module: moduleRel,
        message: "createModuleLayout 未提供 useNavItems（可能走 blockAssembly 派生，v1 不校验）",
      }),
    ];
  }
  if (extracted.items.length === 0) {
    return [
      demote({
        level: "warn",
        module: moduleRel,
        message: "useNavItems 存在但未提取到字面量数组（可能定义在外部文件或动态构建）",
      }),
    ];
  }

  const findings: Finding[] = [];
  const seenIds = new Map<string, number>();
  const seenTargets = new Map<string, string>(); // href 目标 -> 首个 item id

  for (const item of extracted.items) {
    const label = item.id ?? item.href ?? "(匿名)";
    if (!item.id) {
      findings.push(demote({ level: "fail", module: moduleRel, item: label, message: "缺少 id 字面量" }));
      continue;
    }
    seenIds.set(item.id, (seenIds.get(item.id) ?? 0) + 1);
    if (item.hrefComputed) {
      findings.push(
        demote({ level: "warn", module: moduleRel, item: label, message: "href 为计算表达式，无法静态判定" }),
      );
      continue;
    }
    if (!item.href) {
      findings.push(
        demote({
          level: "fail",
          module: moduleRel,
          item: item.id,
          message: "缺少 href 字面量（Gateway Navigation 直接消费 item.href，缺失会产生 /undefined 跳转）",
        }),
      );
      continue;
    }
    const target = item.href.replace(/^\//, "");
    if (!KEBAB_SINGLE.test(target)) {
      findings.push(
        demote({
          level: "fail",
          module: moduleRel,
          item: item.id,
          message: `有效目标 "${target}" 不是单段 kebab-case（Gateway baseSegment 拼接契约要求）`,
        }),
      );
    }
    const prev = seenTargets.get(target);
    if (prev !== undefined) {
      findings.push(
        demote({
          level: "fail",
          module: moduleRel,
          item: item.id,
          message: `有效目标 "${target}" 与 item "${prev}" 重复（两个 nav 项同路由）`,
        }),
      );
    } else {
      seenTargets.set(target, item.id);
    }
  }
  for (const [id, count] of seenIds) {
    if (count > 1) {
      findings.push(demote({ level: "fail", module: moduleRel, item: id, message: `id 重复 ×${count}` }));
    }
  }
  if (findings.length === 0) {
    findings.push({ level: "pass", module: moduleRel, message: `${extracted.items.length} 项全部合规` });
  }
  return findings;
}

// ── main ──────────────────────────────────────────────────────────────
const baseline = loadBaseline();
const frontends = discoverModuleFrontends();
const all: Finding[] = [];
for (const fe of frontends) all.push(...auditModule(fe, baseline));

const counts: Record<Level, number> = { fail: 0, warn: 0, skip: 0, baseline: 0, pass: 0 };
for (const f of all) {
  counts[f.level] += 1;
  const tag = f.level.toUpperCase().padEnd(8);
  console.log(`[${tag}] ${f.module}${f.item ? ` :: ${f.item}` : ""} — ${f.message}`);
}
console.log(
  `\n汇总: ${frontends.length} 模块前端 | pass=${counts.pass} fail=${counts.fail} warn=${counts.warn} baseline=${counts.baseline} skip=${counts.skip}`,
);

if (STRICT && counts.fail > 0) {
  console.error(`\nFAIL: ${counts.fail} 条 fail 级违规（基线外）`);
  process.exit(1);
}
process.exit(0);
