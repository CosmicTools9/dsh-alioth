#!/usr/bin/env bun
/// <reference types="bun" />
/**
 * gate-refs.ts — 门禁「声明面」的单一解析器（argv ∪ coveredBy ∪ inventory）
 *
 * 为什么要有这一份：门禁声明面有三处——`scripts/pre/gates.ts` 的 `argv` / `coveredBy` 与
 * `scripts/pre/gate-inventory.txt`（迁移前 hook 的机械提取清单）——此前被四个消费者**各自**重新解析：
 *   · `scripts/pre/run-gates.ts` 的 `--audit` 只解析 **argv** 的存在性（`coveredBy` 与清单里的
 *     已删脚本静默通过：`check-db-pipeline-guard.ts` 即实测一例）；
 *   · `scripts/check/check-script-refs.ts` 的扫描面只有 `scripts/**.sh` + `.mise.toml` + hook 薄壳
 *     （看不见 `.ts` 门禁声明）；
 *   · `scripts/check/check-all-attribution.ts` 自带 `parseGateSeats`（文本正则解析 gates.ts argv）
 *     与 `parseChecks`（CHECKS 行文法）；
 *   · `scripts/check/check-gate-hygiene.ts` 自带 `checkersFromCheckAll`（第二份 CHECKS 行文法）。
 * 多份实现必然漂移（`scripts/` 前缀表、CHECKS 行文法、`${CHECK}` 模板各一份）⇒ 收敛到本模块：
 *   · `SCRIPT_REF_SOURCE` / `extractScriptRefs()` —— 声明面脚本路径的**唯一形状**；
 *   · `resolveScriptRefs()` —— argv ∪ coveredBy ∪ inventory 的引用清单 + 存在性 + 执行面归属 + 退役登记；
 *   · `parseChecks()` —— `scripts/check/check-all.sh` CHECKS 数组的唯一解析；
 *   · `RETIRED_SCRIPT_REFS` —— 显式退役登记（唯一来源：登记即「允许缺失」，未登记缺失 = 硬失败）。
 *
 * 四个消费者（run-gates --audit / check-script-refs / check-all-attribution / check-gate-hygiene）
 * MUST 消费本模块；新增消费者 MUST NOT 再写第二份前缀表 / 行文法 / 退役表。
 *
 * 用法（判据自测：含缺失 / 退役 / 无执行位三类负例）:
 *   bun scripts/lib/gate-refs.ts --self-test
 */
import { existsSync, readFileSync } from "node:fs";
import { join, resolve } from "node:path";

import { GATES, type Gate } from "../pre/gates";
import { renderCheckAllBash } from "../pre/registry-view";

// ── 唯一形状：声明面脚本路径 ────────────────────────────────

/** 声明面脚本路径的唯一形状（**正则源串**；消费者 MUST 经 `scriptRefRe()`/`extractScriptRefs()` 使用）。 */
export const SCRIPT_REF_SOURCE =
  "scripts\\/[A-Za-z0-9_./-]+\\.(?:ts|sh|mjs|js)";

/** 以唯一形状构造新正则（`/g`，每次调用独立实例，避免 lastIndex 串扰）。 */
export function scriptRefRe(): RegExp {
  return new RegExp(SCRIPT_REF_SOURCE, "g");
}

/** 提取文本中的声明面脚本引用（去重、保序）。 */
export function extractScriptRefs(text: string): string[] {
  const out = new Set<string>();
  for (const m of text.matchAll(scriptRefRe())) out.add(m[0]);
  return [...out];
}

// ── 显式退役登记（唯一来源）────────────────────────────────

/**
 * 声明的脚本**已按设计下线**时在此逐条登记（ref → 退役理由）。
 * 未登记 ⇒ 缺失即硬失败（fail-closed，不许静默通过）；登记项在审计输出中显式列出（不得静默）。
 */
export const RETIRED_SCRIPT_REFS: Readonly<Record<string, string>> = {
  "scripts/check/check-db-pipeline-guard.ts":
    "随 db_guard 机制退役（change remove-db-pipeline-guard）；其 safety-guard 行为段由 isahl-ddl-guard gate 承接",
  "scripts/check/check-age-drift.ts":
    "随 NGAC 侧 AGE 图投影退役（change retire-ngac-age-graph-projection）；判据对象 isahl_auth.age_ngac_projection_diff() 已无生产方，门禁恒绿 = 静默死角（无法检出任何真实缺陷）；后续归零由 ngac-unwired-registry gate 与 NGAC_SPEC §12 登记承担",
};

// ── 声明面来源与执行面 ─────────────────────────────────────

/** 声明面来源三处（合称「声明面」）。 */
export type RefSource = "argv" | "coveredBy" | "inventory";

/**
 * 执行面归属：
 *   argv      —— 某 gate 的 argv 直接执行（真执行位）；
 *   aggregate —— 聚合执行面文件（check-all.sh CHECKS / repo-wide-gates / test-all / hook 薄壳）执行；
 *   manual    —— 仅 notify/hint 提示（人工通道；gate 显式输出，非静默）；
 *   none      —— 无任何归属（声明了却永不执行 = 静默死角，审计硬失败）。
 */
export type RefSeat = "argv" | "aggregate" | "manual" | "none";

/**
 * 聚合执行面（gate 表之外的执行器）：覆盖 `coveredBy` 认领的「由聚合面兜底」来源。
 *
 * `scripts/lib/derived-artifacts.ts`（change `unify-derived-freshness`）：派生新鲜度**族清单唯一来源**；
 * 族成员的 probe 脚本经该表被统一接口 `scripts/check/derived-freshness.ts` 以子进程执行 ⇒ 其声明面
 * 归属 = `aggregate`（否则成员从 gate argv 退出后成无执行位，`run-gates --audit` ③ 硬失败）。
 */
export const AGGREGATE_FACE_FILES = [
  "scripts/check/check-all.sh",
  "scripts/check/check-repo-wide-gates.sh",
  "scripts/test-all.sh",
  ".husky/pre-commit",
  ".husky/pre-push",
  "scripts/pre/ns-hook.sh",
  "scripts/lib/derived-artifacts.ts",
] as const;

export interface ScriptRef {
  /** 仓库根相对脚本路径（`scripts/…`）。 */
  path: string;
  source: RefSource;
  /** argv/coveredBy 所属 gate id（inventory 无）。 */
  gateId?: string;
  exists: boolean;
  /** 显式退役理由（登记于 `RETIRED_SCRIPT_REFS`）；存在时允许缺失。 */
  retired?: string;
  seat: RefSeat;
}

export interface ResolveOptions {
  /** 声明面来源（默认真实表 `GATES`；测试注入合成表）。 */
  gates?: readonly Gate[];
  /** 机械清单条目（默认读 `gate-inventory.txt`）。 */
  inventory?: readonly string[];
  /** 存在性判定基准（默认模块位置推导的仓库根）。 */
  root?: string;
}

/** 机械清单条目（去注释/空行；`gate-inventory.txt` 与 gates.ts 同目录，ns 仓经 `scripts` 符号链接同源）。 */
export function readInventory(): string[] {
  const file = join(import.meta.dirname, "..", "pre", "gate-inventory.txt");
  const text = existsSync(file) ? readFileSync(file, "utf8") : "";
  return text
    .split("\n")
    .map((line) => line.trim())
    .filter((line) => line !== "" && !line.startsWith("#"));
}

/** 清单里的非路径条目（工具调用形态，如 `npx lint-staged`）：无存在性语义，跳过路径解析。 */
function isNonPathEntry(line: string): boolean {
  return !line.includes("scripts/");
}

/** 读取聚合执行面文本（不存在 ⇒ 空串，不报错——ns 仓/测试根可能没有这些文件）。 */
function aggregateFaceText(root: string): string {
  return AGGREGATE_FACE_FILES.map((rel) => {
    const abs = join(root, rel);
    return existsSync(abs) ? readFileSync(abs, "utf8") : "";
  }).join("\n");
}

/**
 * 聚合执行面的**判定文本**（含 `check-all.sh` 的派生清单）。
 *
 * `check-all.sh` 自 change `unify-gate-registration` 起不再内嵌 `declare -a CHECKS`——
 * 清单唯一真源 = `scripts/pre/gates.ts` 的 `CHECK_ALL`，由 `registry-view.ts` 派生。故这里
 * MUST 消费**派生文本**（`renderCheckAllBash()`），否则 check-all 承载的脚本会失去 `aggregate`
 * 执行位归属（`check-script-refs` 与 `run-gates --audit` 会据此转红）。
 */
function aggregateFaceJudgementText(root: string): string {
  return `${aggregateFaceText(root)}\n${renderCheckAllBash()}`;
}

/**
 * 解析声明面全部脚本引用：argv ∪ coveredBy ∪ inventory。
 * 每条返回存在性、执行面归属与退役登记（判定语义由调用方决定：缺失/无执行位 ⇒ 硬失败）。
 */
export function resolveScriptRefs(options: ResolveOptions = {}): ScriptRef[] {
  const gates = options.gates ?? GATES;
  const inventory = options.inventory ?? readInventory();
  // 默认基准 = 模块位置推导的仓库根（`scripts/lib/` → 仓库根）；ns 仓经符号链接消费时同源。
  const root = options.root ?? resolve(import.meta.dirname, "..", "..");

  const refs: ScriptRef[] = [];
  const seen = new Set<string>();
  const push = (path: string, source: RefSource, gateId?: string): void => {
    const key = `${path}\u0000${source}\u0000${gateId ?? ""}`;
    if (seen.has(key)) return;
    seen.add(key);
    refs.push({
      path,
      source,
      ...(gateId === undefined ? {} : { gateId }),
      exists: existsSync(join(root, path)),
      ...(RETIRED_SCRIPT_REFS[path] === undefined
        ? {}
        : { retired: RETIRED_SCRIPT_REFS[path] }),
      seat: "none", // 先占位，下面按执行面统一回填
    });
  };

  for (const gate of gates) {
    for (const arg of gate.argv)
      for (const ref of extractScriptRefs(arg)) push(ref, "argv", gate.id);
    for (const arg of gate.coveredBy ?? [])
      for (const ref of extractScriptRefs(arg)) push(ref, "coveredBy", gate.id);
  }
  for (const entry of inventory) {
    if (isNonPathEntry(entry)) continue;
    for (const ref of extractScriptRefs(entry)) push(ref, "inventory");
  }

  const argvRefs = new Set<string>();
  for (const gate of gates)
    for (const arg of gate.argv)
      for (const ref of extractScriptRefs(arg)) argvRefs.add(ref);
  const aggregateRefs = new Set(
    extractScriptRefs(aggregateFaceJudgementText(root)),
  );
  const manualRefs = new Set<string>();
  for (const gate of gates) {
    for (const arg of [...(gate.notify ?? []), ...(gate.hint ?? [])]) {
      for (const ref of extractScriptRefs(arg)) manualRefs.add(ref);
    }
  }

  for (const ref of refs) {
    if (ref.source === "argv" || argvRefs.has(ref.path)) ref.seat = "argv";
    else if (aggregateRefs.has(ref.path)) ref.seat = "aggregate";
    else if (manualRefs.has(ref.path)) ref.seat = "manual";
  }
  return refs;
}

/** 缺失且未登记退役的声明面引用（硬失败面）。 */
export function missingScriptRefs(options: ResolveOptions = {}): ScriptRef[] {
  return resolveScriptRefs(options).filter(
    (ref) => !ref.exists && ref.retired === undefined,
  );
}

/** 有执行面归属问题的声明面引用：**存在**却无任何执行/提示面（`none`）、非 argv 直执行且未退役。 */
export function seatlessScriptRefs(options: ResolveOptions = {}): ScriptRef[] {
  return resolveScriptRefs(options).filter(
    (ref) => ref.exists && ref.seat === "none" && ref.retired === undefined,
  );
}

/** 显式退役的声明面引用（审计输出 MUST 列出，不得静默）。 */
export function retiredScriptRefs(options: ResolveOptions = {}): ScriptRef[] {
  return resolveScriptRefs(options).filter((ref) => ref.retired !== undefined);
}

/** 引用的来源描述（审计输出的单一格式源）。 */
export function refOrigin(ref: ScriptRef): string {
  if (ref.source === "inventory") return "清单 gate-inventory.txt";
  return `${ref.source}@${ref.gateId ?? "?"}`;
}

/** gate id → 该 gate 的 argv 脚本引用（完整仓库相对路径；seat 审计的唯一解析入口）。 */
export function gateArgvRefsById(
  gates: readonly Gate[] = GATES,
): Map<string, string[]> {
  const out = new Map<string, string[]>();
  for (const gate of gates) {
    const refs = new Set<string>();
    for (const arg of gate.argv)
      for (const ref of extractScriptRefs(arg)) refs.add(ref);
    out.set(gate.id, [...refs]);
  }
  return out;
}

// ── check-all.sh CHECKS 的唯一解析 ──────────────────────────

export interface CheckEntry {
  name: string;
  level: string;
  cmd: string;
  /** 仓库相对脚本路径（行内脚本字段的首个空白分词）。 */
  script: string;
  /** 行内 `|` 分隔的其余参数。 */
  args: string[];
}

/**
 * 解析 `check-all.sh` 的 CHECKS 条目（`name|level|cmd|script|args…`）。
 * 优先取 `declare -a CHECKS=( … )` 块（块外不存在同形行时与全文解析等价），否则回退全文。
 */
export function parseChecks(text: string): CheckEntry[] {
  const block = /declare -a CHECKS=\(([\s\S]*?)\n\)/m.exec(text)?.[1];
  const src = block ?? text;
  return [
    ...src.matchAll(
      /^\s*"([^|"]+)\|(T\d)\|([a-z]+)\|([^"|]+)((?:\|[^"]*)?)"/gm,
    ),
  ].map((m) => ({
    name: m[1]!,
    level: m[2]!,
    cmd: m[3]!,
    script: m[4]!.trim().split(" ")[0]!,
    args: (m[5] ?? "").split("|").filter((arg) => arg !== ""),
  }));
}

// ── 判据自测（正例 + 负例；消费者与 CI 可直接跑）────────────

function selfTest(): number {
  let bad = 0;
  const check = (label: string, ok: boolean): void => {
    if (!ok) bad += 1;
    console.log(`${ok ? "✔" : "✘"} ${label}`);
  };
  const mkGate = (partial: Partial<Gate> & { id: string }): Gate => ({
    stage: "commit",
    argv: [],
    blocking: true,
    cls: "read",
    ...partial,
  });

  // 正例：argv 指向真实存在的脚本，清单同项 ⇒ 无缺失、无无执行位
  {
    const gate = mkGate({
      id: "ok",
      argv: ["bun", "scripts/check/check-script-refs.ts"],
    });
    const missing = missingScriptRefs({
      gates: [gate],
      inventory: ["scripts/check/check-script-refs.ts"],
    });
    const seatless = seatlessScriptRefs({
      gates: [gate],
      inventory: ["scripts/check/check-script-refs.ts"],
    });
    check(
      "正例：argv ∪ inventory 指向真实脚本 ⇒ 缺失 0 / 无执行位 0",
      missing.length === 0 && seatless.length === 0,
    );
  }
  // 负例：argv 登记不存在的路径 ⇒ 缺失面点名该路径
  {
    const gate = mkGate({
      id: "broken",
      argv: ["bun", "scripts/check/check-does-not-exist-9f3a.ts"],
    });
    const missing = missingScriptRefs({ gates: [gate], inventory: [] });
    check(
      `负例：argv 缺失路径被点名（${missing.map((r) => r.path).join(",")}）`,
      missing.length === 1 &&
        missing[0]!.path === "scripts/check/check-does-not-exist-9f3a.ts",
    );
  }
  // 负例：coveredBy 登记不存在的路径（此前从不被解析）⇒ 缺失面点名
  {
    const gate = mkGate({
      id: "cov",
      argv: [],
      coveredBy: ["scripts/check/check-gone-4d21.ts"],
    });
    const missing = missingScriptRefs({ gates: [gate], inventory: [] });
    check(
      `负例：coveredBy 缺失路径被点名（${missing.map((r) => r.path).join(",")}）`,
      missing.length === 1 && missing[0]!.source === "coveredBy",
    );
  }
  // 正例：显式退役登记 ⇒ 缺失但不硬失败，且理由可见
  {
    const gate = mkGate({
      id: "retired",
      argv: [],
      coveredBy: ["scripts/check/check-db-pipeline-guard.ts"],
    });
    const missing = missingScriptRefs({ gates: [gate], inventory: [] });
    const retired = retiredScriptRefs({ gates: [gate], inventory: [] });
    check(
      "正例：退役登记项缺失不硬失败且理由可见",
      missing.length === 0 &&
        retired.length === 1 &&
        (retired[0]!.retired ?? "").includes("退役"),
    );
  }
  // 负例：声明了但无任何执行/提示面（真实存在却永不执行）⇒ 无执行位面点名
  {
    const gate = mkGate({
      id: "seatless",
      argv: [],
      coveredBy: ["scripts/lib/gate-refs.ts"],
    });
    const seatless = seatlessScriptRefs({ gates: [gate], inventory: [] });
    check(
      `负例：无执行位声明被点名（${seatless.map((r) => r.path).join(",")}）`,
      seatless.length === 1 && seatless[0]!.seat === "none",
    );
  }
  // 正例：聚合执行面（check-all.sh CHECKS）认领 ⇒ aggregate，不判无执行位
  {
    const ref = resolveScriptRefs({
      gates: [],
      inventory: ["scripts/check/check-code-semantics.ts"],
    });
    check(
      "正例：聚合面认领不判无执行位",
      ref.length === 1 && ref[0]!.seat === "aggregate",
    );
  }
  // parseChecks：唯一行文法（含 args 拆分与脚本内联分词）
  {
    const parsed = parseChecks(
      "declare -a CHECKS=(\n" +
        '  "a|T1|bun|scripts/check/check-a.ts|--fail"\n' +
        '  "b|T2|bash|scripts/check/check-b.sh --x"\n' +
        ")\n",
    );
    check(
      "parseChecks：CHECKS 块解析（含 args 与内联分词）",
      parsed.length === 2 &&
        parsed[0]!.script === "scripts/check/check-a.ts" &&
        parsed[0]!.args[0] === "--fail" &&
        parsed[1]!.script === "scripts/check/check-b.sh",
    );
  }
  console.log(
    bad === 0
      ? "✅ gate-refs self-test 通过（7 例）"
      : `❌ gate-refs self-test 失败 ${bad} 例`,
  );
  return bad === 0 ? 0 : 1;
}

if (import.meta.main) {
  if (process.argv.includes("--self-test")) process.exit(selfTest());
  console.log("用法: bun scripts/lib/gate-refs.ts --self-test");
  process.exit(2);
}
