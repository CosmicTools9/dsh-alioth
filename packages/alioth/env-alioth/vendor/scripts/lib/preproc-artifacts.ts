/**
 * preproc-artifacts.ts — Pre-Proc 产物枚举与基线比对（check-*.ts 系列共用设施）
 *
 * 布局契约：`Pre-Proc/{ns}/Sources/Apps/{Modules,Blocks,Services,...}`，兼容
 * OpenActivity 的 `Sources/Open/*` 与未迁移的扁平 `Sources/*`（见 preproc-layout.mjs）。
 * 基线约定：`scripts/check/baselines/*.json` 登记存量违规指纹，门禁只阻断**基线之外的新增**。
 */
import { existsSync, readFileSync, readdirSync, writeFileSync } from "node:fs";
import { join, relative, resolve } from "node:path";
import { spawnSync } from "node:child_process";
import { gatewaySourcesKindDir } from "./preproc-layout.mjs";

export interface Violation { ns: string; file: string; rule: string; detail: string }

export interface Baseline {
  path: string;
  entries: Set<string>;
}

/** 产物目录判据：排除隐藏目录（`.x`）与备份（`*.bak`）——备份目录不作为组合真相源 */
export function isArtifactDir(name: string): boolean {
  return !name.startsWith(".") && !name.endsWith(".bak");
}

export function listNamespaces(root: string): string[] {
  return readdirSync(join(root, "Pre-Proc"), { withFileTypes: true })
    .filter((e) => e.isDirectory() && isArtifactDir(e.name))
    .map((e) => e.name)
    .sort();
}

/** 某类单元的候选根（Apps 镜像优先、Open 次之、扁平回退已由 gatewaySourcesKindDir 覆盖） */
export function kindRoots(root: string, ns: string, kind: string): string[] {
  return [
    gatewaySourcesKindDir(root, ns, kind),
    join(root, "Pre-Proc", ns, "Sources", "Open", kind),
  ].filter((p) => existsSync(p));
}

/** 某类单元的目录名集合（去重） */
export function unitIds(root: string, ns: string, kind: string): Set<string> {
  const ids = new Set<string>();
  for (const r of kindRoots(root, ns, kind)) {
    for (const e of readdirSync(r, { withFileTypes: true })) if (e.isDirectory() && isArtifactDir(e.name)) ids.add(e.name);
  }
  return ids;
}

/** 某类单元的 `<id>/<file>` 绝对路径清单 */
export function unitFiles(root: string, ns: string, kind: string, file: string): string[] {
  const out: string[] = [];
  for (const r of kindRoots(root, ns, kind)) {
    for (const e of readdirSync(r, { withFileTypes: true })) {
      if (!e.isDirectory() || !isArtifactDir(e.name)) continue;
      const f = join(r, e.name, file);
      if (existsSync(f)) out.push(f);
    }
  }
  return out.sort();
}

/** 由同 ns 原型产物推导 `prototypeVersion`（`b-v{N}.html` 取最大 N；仅有 `llm-tsx` 源 → `v1`；无痕迹 → undefined） */
export function derivePrototypeVersion(root: string, ns: string, blockId: string): string | undefined {
  const bases = [
    `Pre-Proc/${ns}/Prototypes/Blocks/${blockId}`,
    `Pre-Proc/${ns}/Prototypes/Apps/Blocks/${blockId}`,
    `Pre-Proc/${ns}/Prototypes/Open/Blocks/${blockId}`,
  ];
  const dir = bases.map((d) => join(root, d)).find((d) => existsSync(d));
  if (!dir) return undefined;
  let names: string[];
  try {
    names = readdirSync(dir);
  } catch {
    return undefined;
  }
  const htmls = names.filter((n) => /^b-v\d+\.html$/.test(n));
  if (htmls.length > 0) {
    const maxN = Math.max(...htmls.map((n) => Number(n.match(/b-v(\d+)/)?.[1] ?? 0)));
    return `v${maxN}`; // 规范格式（BLOCK_SCHEMA §1：prototypeVersion="v{N}" 指向 b-v{N}.html）
  }
  return names.includes("llm-tsx") ? "v1" : undefined;
}

/** 该块在 Prototypes/ 下是否有任何痕迹（构建产物或 `llm-tsx` 源） */
export function hasPrototypeTraces(root: string, ns: string, blockId: string): boolean {
  return derivePrototypeVersion(root, ns, blockId) !== undefined;
}

export function fingerprint(v: Violation): string {
  return `${v.ns}|${v.file}|${v.rule}|${v.detail}`;
}

export function loadBaseline(path: string): Baseline {
  if (!existsSync(path)) return { path, entries: new Set() };
  try {
    const raw = JSON.parse(readFileSync(path, "utf-8")) as { violations?: unknown };
    const list = Array.isArray(raw.violations) ? (raw.violations as string[]) : [];
    return { path, entries: new Set(list) };
  } catch {
    return { path, entries: new Set() };
  }
}

export function writeBaseline(path: string, violations: Violation[], note: string): void {
  writeFileSync(path, JSON.stringify({
    updated_at: new Date().toISOString(),
    note,
    violations: [...violations.map(fingerprint)].sort(),
  }, null, 2) + "\n");
}

export function summarize(violations: Violation[]): string {
  const byRule = violations.reduce<Record<string, number>>((acc, v) => { acc[v.rule] = (acc[v.rule] ?? 0) + 1; return acc; }, {});
  return Object.entries(byRule).map(([k, n]) => `${k}=${n}`).join(" ") || "无";
}

export function relPath(root: string, p: string): string {
  return relative(root, p);
}

export function resolveFrom(root: string, p: string): string {
  return resolve(root, p);
}

/** DB 维度码集（isahl.zc_id_{dim}）；查询失败返回空集 → 调用方须显式声明该维度跳过 */
export function dbDimensionCodes(root: string, dim: string): Set<string> {
  const r = spawnSync("mise", ["run", "schema-info", "--", "raw-sql",
    `SELECT code FROM isahl.zc_id_${dim} ORDER BY code`],
    { cwd: join(root, "Meta/backend"), encoding: "utf-8", timeout: 20000 });
  if (r.status !== 0) return new Set();
  try {
    const rows = (JSON.parse(r.stdout.toString()) as { rows?: Array<{ code: unknown }> }).rows ?? [];
    return new Set<string>(rows.map((x) => String(x.code).trim()).filter(Boolean));
  } catch { return new Set(); }
}
