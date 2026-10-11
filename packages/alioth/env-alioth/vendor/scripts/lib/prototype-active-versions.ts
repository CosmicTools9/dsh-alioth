#!/usr/bin/env bun
/**
 * prototype-active-versions.ts — 原型**活动版本指针**解析（版本窗口治理的保护输入）。
 *
 * 为什么需要：`cleanup-prototype-artifacts.sh --keep N` 只按版本号尾部裁剪，不读活动指针
 * （`module.json#prototypeVersion` / `block.json#prototypeVersion`）——若活动版本不在最新 N 版内，
 * 裁剪会把**活动原型**连同 `.bundle.js` / `-report.json` 一起删掉。
 *
 * 输出（stdout，TSV，稳定排序）：
 *   `<原型目录><TAB><活动版本 base 名>`   # base 名 = 原型 HTML 去扩展名（如 `b-v4` / `m-v15`）
 * 无 `prototypeVersion` 字段 / 配置缺失的单元不输出（调用方按「无保护」处理）。
 *
 * 用法:
 *   bun scripts/lib/prototype-active-versions.ts [--root <repo>] [--ns <ns>]
 */
import { existsSync, readdirSync, readFileSync } from "node:fs";
import { basename, dirname, join } from "node:path";
import { unitFiles } from "./preproc-artifacts";

/** 原型类别 → 活动指针所在的单元配置文件（Apps 无 `prototypeVersion` 字段，故不参与） */
const KIND_CONFIG: Record<string, string> = { Modules: "module.json", Blocks: "block.json" };

/** 原型文件名前缀（`module.json#prototypeVersion: "v{N}"` 形指针需补此前缀） */
const KIND_PREFIX: Record<string, string> = { Modules: "m", Blocks: "b" };

/** ns 目录枚举：**必须**容忍符号链接——`Pre-Proc/{ns}` 在平台仓是指向 ns 仓的 symlink，
 *  而 `Dirent.isDirectory()` 对 symlink 恒 false（`listNamespaces()` 因此对它们失明 ⇒
 *  不带 `--ns` 时会得到空保护集，等于无保护）。 */
function namespacesOf(root: string): string[] {
  return readdirSync(join(root, "Pre-Proc"), { withFileTypes: true })
    .filter(
      (e) =>
        (e.isDirectory() || e.isSymbolicLink()) &&
        !e.name.startsWith(".") &&
        !e.name.endsWith(".bak"),
    )
    .map((e) => e.name)
    .sort();
}

function parseArgs(argv: string[]): { root: string; ns: string } {
  let root = resolveRoot();
  let ns = "";
  for (let i = 0; i < argv.length; i += 1) {
    if (argv[i] === "--root") root = argv[++i] ?? root;
    else if (argv[i] === "--ns") ns = argv[++i] ?? "";
  }
  return { root, ns };
}

function resolveRoot(): string {
  // scripts/lib/ → 仓库根
  return dirname(dirname(import.meta.dirname));
}

const { root, ns } = parseArgs(process.argv.slice(2));
const namespaces = ns !== "" ? [ns] : namespacesOf(root);
const rows: string[] = [];
for (const namespace of namespaces) {
  for (const [kind, cfgFile] of Object.entries(KIND_CONFIG)) {
    for (const cfgPath of unitFiles(root, namespace, kind, cfgFile)) {
      const unitId = basename(dirname(cfgPath));
      // 层级感知（布局契约 preproc-layout.mjs）：单元配置位于 Sources/Open/** 时，其设计腿在
      // Prototypes/Open/{kind}/{unitId}（否则 Apps 形）。缺此分派会把 Open 单元的活动指针
      // 键到 Apps 形设计目录，按单元名跨层级串味（实测 Open m-v2 覆盖 Apps m-v188）。
      const isOpenTier = cfgPath.includes(join("Sources", "Open"));
      const protoDir = isOpenTier
        ? join(root, "Pre-Proc", namespace, "Prototypes", "Open", kind, unitId)
        : join(root, "Pre-Proc", namespace, "Prototypes", kind, unitId);
      if (!existsSync(protoDir)) continue;
      let raw: unknown;
      try {
        const parsed: unknown = JSON.parse(readFileSync(cfgPath, "utf8"));
        raw =
          parsed !== null && typeof parsed === "object" && "prototypeVersion" in parsed
            ? parsed.prototypeVersion
            : undefined;
      } catch {
        continue; // 配置不可解析：不声明保护（交由 check-block-json 等门禁报错）
      }
      if (typeof raw !== "string" || raw.trim() === "") continue;
      // 指针存在两形：`v{N}`（BLOCK_SCHEMA §1 规范形，实际指向 `b-v{N}.html`）与 `b-v{N}`/`m-v{N}`。
      // `v{N}` MUST 补类别前缀，否则保护集与实际文件名不匹配（2026-09-27 实测 35 个单元为该形）。
      const v = raw.trim();
      const bases = /^v\d+$/.test(v) ? [v, `${KIND_PREFIX[kind]}-${v}`] : [v];
      for (const base of bases) rows.push(`${protoDir}\t${base}`);
    }
  }
}
process.stdout.write(rows.sort().join("\n") + (rows.length > 0 ? "\n" : ""));
