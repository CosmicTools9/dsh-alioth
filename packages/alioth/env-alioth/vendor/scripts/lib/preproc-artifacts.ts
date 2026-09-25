/**
 * preproc-artifacts.ts — Pre-Proc 产物枚举与基线比对（check-*.ts 系列共用设施）
 *
 * 布局契约：`Pre-Proc/{ns}/Sources/Apps/{Modules,Blocks,Services,...}`，兼容
 * OpenActivity 的 `Sources/Open/*` 与未迁移的扁平 `Sources/*`（见 preproc-layout.mjs）。
 * 基线约定：`scripts/check/baselines/*.json` 登记存量违规指纹，门禁只阻断**基线之外的新增**。
 */
import { existsSync, readdirSync } from "node:fs";
import { join, relative, resolve } from "node:path";
import { spawnSync } from "node:child_process";
import { isRecord } from "./type-guards";
import { gatewaySourcesKindDir } from "./preproc-layout.mjs";
import {
  readBaselineText,
  repoRoot,
  writeBaseline as guardedWriteBaseline,
} from "../check/lib/baseline-store";

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
  // 经唯一入口读取（INV-2：本仓为 ns 仓时丢弃其他 ns 的条目；判定结果不变，仅消除死条目）
  const text = readBaselineText(repoRoot(), path);
  if (text === null) return { path, entries: new Set() };
  try {
    const raw = JSON.parse(text) as { violations?: unknown };
    const list = Array.isArray(raw.violations) ? (raw.violations as string[]) : [];
    return { path, entries: new Set(list) };
  } catch {
    return { path, entries: new Set() };
  }
}

export function writeBaseline(path: string, violations: Violation[], note: string): void {
  // 经唯一入口写入（INV-1：写入方身份 = **进程所在仓**，不是目标路径所在仓；
  // ns 仓写共享面会被拒绝，见 scripts/check/lib/baseline-store.ts）。
  guardedWriteBaseline(repoRoot(), path, JSON.stringify({
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

/**
 * 维度输入不可得（维度码查询非零退出 / 零行 / 不可解析，或维度行 id 通道缺 `DATABASE_URL`）——
 * 门禁 MUST 驳回至人在回路，MUST NOT 降级跳过依赖该输入的判据（用户裁定 2026-09-23）。
 */
export class DimensionCodesUnavailableError extends Error {
  readonly subject: string;
  constructor(subject: string, detail: string) {
    super(`${subject} 不可得（${detail}）——门禁驳回：请恢复该输入后重跑`);
    this.name = "DimensionCodesUnavailableError";
    this.subject = subject;
  }
}

// ── DB 只读探针（schema-info 二进制直呼，不经 mise）─────────────────────
//
// 2026-09-24 修（用户报「ontology-contract 门禁反复 timeout」）：原实现经
// `mise run schema-info`，而 `Meta/backend/.env` 的 DATABASE_URL 是 `enc:` 密文 ——
// mise 载入/解密它在实测中阻塞 **~156s**（user+sys ≈ 0.4s，纯等待）⇒ 门禁「60s × 3 维度」
// 撞满 push 阶段 180s 上限（status=timeout）；同因，`check-dimension-codes-fail-closed`
// 的负例（注入不可连 DSN 期望退出码 2）也因探针偶发"迅速成功"而拿不到 2。
//
// 现直呼**预编译二进制**（与 `Meta/backend/.mise.toml` 同顺序：release → debug），并显式
// 注入 DSN ⇒ 单次实测 ~0.4s，且**尊重调用方传入的 `DATABASE_URL`**（不可连即快速失败，
// 负例可复现）；未设或为密文时回落到仓内 dev 库约定（与 check-block-json 同默认）。
function schemaInfoBinary(root: string): string | null {
  const t = spawnSync("bash", [join(root, "scripts/cargo-target.sh"), "meta"], {
    encoding: "utf-8",
  }).stdout?.trim();
  if (!t) return null;
  for (const p of [join(t, "release/schema-info"), join(t, "debug/schema-info")]) {
    if (existsSync(p)) return p;
  }
  return null;
}

/** 只读 DSN：显式 `DATABASE_URL`（须为真 DSN；`enc:` 密文不算）> 仓内 dev 库约定。 */
function schemaInfoDsn(): string {
  const env = process.env.DATABASE_URL ?? "";
  return env.startsWith("postgres") ? env : "postgres://isahl@localhost:5432/aliothstudio_dev";
}

/** 直呼 `schema-info` 执行任意子命令（不经 mise）；失败原因走返回值，由调用方决定驳回文案。 */
export function schemaInfoRun(
  root: string,
  args: readonly string[],
  timeoutMs = 8_000,
): { ok: true; stdout: string } | { ok: false; detail: string } {
  const bin = schemaInfoBinary(root);
  if (!bin) {
    return {
      ok: false,
      detail: "schema-info 二进制缺失（cd Meta/backend && cargo build --bin schema-info）",
    };
  }
  const r = spawnSync(bin, args as string[], {
    encoding: "utf-8",
    timeout: timeoutMs,
    env: { ...process.env, DATABASE_URL: schemaInfoDsn() },
  });
  if (r.status !== 0) {
    const tail =
      `${r.stdout ?? ""}${r.stderr ?? ""}`.trim().split("\n").filter(Boolean).slice(-1)[0] ?? "";
    return {
      ok: false,
      detail:
        r.status === null
          ? `schema-info 超时（${timeoutMs}ms）`
          : `schema-info 退出码 ${r.status}${tail ? `：${tail.slice(0, 160)}` : ""}`,
    };
  }
  return { ok: true, stdout: r.stdout.toString() };
}

/** 直呼 `schema-info raw-sql` 执行只读 SQL；失败原因走返回值，由调用方决定驳回文案（禁静默降级）。 */
export function schemaInfoRawSql(
  root: string,
  sql: string,
  timeoutMs = 8_000,
): { ok: true; rows: Record<string, unknown>[] } | { ok: false; detail: string } {
  const run = schemaInfoRun(root, ["raw-sql", sql], timeoutMs);
  if (!run.ok) return run;
  try {
    const parsed: unknown = JSON.parse(run.stdout);
    const rows =
      isRecord(parsed) && Array.isArray(parsed.rows) ? parsed.rows.filter(isRecord) : [];
    return { ok: true, rows };
  } catch {
    return { ok: false, detail: "schema-info 输出非 JSON（无法解析查询结果）" };
  }
}

/** DB 维度码集（isahl.zc_id_{dim}）；不可得即抛 DimensionCodesUnavailableError（禁降级放行） */
export function dbDimensionCodes(root: string, dim: string): Set<string> {
  const subject = `DB 维度码 isahl.zc_id_${dim}`;
  const probe = schemaInfoRawSql(root, `SELECT code FROM isahl.zc_id_${dim} ORDER BY code`);
  if (!probe.ok) throw new DimensionCodesUnavailableError(subject, probe.detail);
  const codes = new Set<string>();
  for (const row of probe.rows) {
    const code = String(row.code ?? "").trim();
    if (code !== "") codes.add(code);
  }
  if (codes.size === 0) throw new DimensionCodesUnavailableError(subject, "查询结果零行");
  return codes;
}
