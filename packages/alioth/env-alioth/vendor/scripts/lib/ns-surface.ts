/**
 * ns-surface.ts — namespace 面「本仓是否持有」判定（2026-09-24 namespace 拆分后共用）。
 *
 * 判据 = **本仓是否跟踪 ns 源**（`Pre-Proc/<ns>/Sources/…` 有跟踪文件），而非「磁盘上有没有目录」：
 * 平台仓开发机以符号链接挂载 ns 仓（目录在、内容不属于本仓）⇒ MUST 视作「不持有」——
 * 否则同一提交在不同机器上判定不同（判定翻转）；ns 仓跟踪该面 ⇒ 照常判定。
 *
 * 用途：平台仓不持有 ns 面时，凡「以 ns 内容为判据面」的检查/门禁 MUST **显式跳过并打印原因**
 * （不静默绿）；对应判据归 ns 仓的门禁档位（见 `.planning/gate-adaptation-assessment.md`）。
 */
import { spawnSync } from "node:child_process";
import { existsSync, readdirSync } from "node:fs";
import { join } from "node:path";

import { repoRoleOf } from "./repo-role";

/**
 * 本仓是否跟踪 ns 源（`Pre-Proc/<ns>/Sources/…`）。
 *
 * @param root 仓库根（默认当前目录；脚本通常传自身解析出的 ROOT）
 */
export function nsSourcesTracked(root = "."): boolean {
  const r = spawnSync(
    "git",
    ["-C", root, "ls-files", "-z", "Pre-Proc/*/Sources/*"],
    {
      encoding: "utf8",
    },
  );
  const stdout = r.stdout ?? "";
  return stdout.split("\u0000").some((f) => f !== "");
}

/** `Pre-Proc/*` 挂载形态快照（逐项 `Dirent`；符号链接只在目标存在时才计入挂载）。 */
export interface NsFaceStatus {
  /** 本仓是否跟踪 ns 源（同 {@link nsSourcesTracked}） */
  tracked: boolean;
  /** 符号链接挂载的 ns 名（`Pre-Proc/<name>` 是链接且目标存在）—— 内容归 ns 仓 */
  symlinkMounts: string[];
  /** 真目录形态的 ns 名（非链接）—— 本仓持有其内容面 */
  realDirs: string[];
}

/**
 * 本仓 `Pre-Proc/*` 的挂载形态：区分「符号链接挂载」（内容归 ns 仓）与「真目录」（本仓持有）。
 *
 * 用途：跳过提示必须说清**为什么**看不见 ns 面（链接挂载 / 根本没有 `Pre-Proc/*`），
 * 而不是笼统打印「0 个单元」——后者与真判定通过无法区分（假绿）。
 *
 * @param root 仓库根（默认当前目录）
 */
export function nsFaceStatus(root = "."): NsFaceStatus {
  const symlinkMounts: string[] = [];
  const realDirs: string[] = [];
  const preProc = join(root, "Pre-Proc");
  if (existsSync(preProc)) {
    for (const e of readdirSync(preProc, { withFileTypes: true })) {
      if (e.isSymbolicLink()) {
        // 断链（目标不存在）不构成挂载面：既无内容，也不该成为跳过理由
        if (existsSync(join(preProc, e.name))) symlinkMounts.push(e.name);
      } else if (e.isDirectory()) {
        realDirs.push(e.name);
      }
    }
  }
  return {
    tracked: nsSourcesTracked(root),
    symlinkMounts: symlinkMounts.sort(),
    realDirs: realDirs.sort(),
  };
}

/**
 * 「本仓不持有 ns 面」的统一跳过提示（**显式跳过**，替代 0 单元的假绿）。
 *
 * 归属：ns 面判定归 ns 仓档位（平台仓在门禁表按 `nsOwnedContent` 跳过）；本函数只把
 * 「为什么跳过」打成一行可审计输出 —— 不静默绿，也不与「判定通过」同形。
 *
 * @param rule 门禁/检查规则名（如 `check-block-json`）
 * @param root 仓库根（默认当前目录）
 */
export function nsFaceSkipNotice(rule: string, root = "."): void {
  const { symlinkMounts } = nsFaceStatus(root);
  const reason =
    symlinkMounts.length > 0
      ? `Pre-Proc/{${symlinkMounts.join(", ")}} 为符号链接挂载，内容归 ns 仓`
      : "无 Pre-Proc/*（本仓不跟踪 ns 源）";
  console.log(
    `⏭ ${rule}: 本仓不持有 ns 面（${reason}）—— 跳过判定（判定归 ns 仓档位）`,
  );
}

/** 本仓是否跟踪该路径（`git ls-files` 非空）——「持有面」的判据（非磁盘存在性）。 */
function trackedInRepo(root: string, rel: string): boolean {
  const r = spawnSync("git", ["-C", root, "ls-files", "-z", "--", rel], {
    encoding: "utf8",
  });
  return (r.stdout ?? "").split("\u0000").some((f) => f !== "");
}

/**
 * 平台面判据根「本仓是否持有」。{@link nsSourcesTracked} 的**镜像**（ns 档位消费平台面）。
 *
 * 判据 = 仓角色为 ns **且** 该判据根不入本仓（`git ls-files <rel>` 空）：ns 仓经 `.alioth/platform.conf`
 * 的 `LINKS` 符号链接消费平台共享面（根 `Cargo.toml` / 根 `Cargo.lock` / `Backup/` 均不入 ns 仓），
 * 磁盘上可能因挂载而可见 ⇒ MUST 视作「不持有」（否则判定面是**别的仓的树**，跨机判定翻转）。
 * 平台仓（无 `.alioth/platform.conf`）**恒为 true** ⇒ 平台档位照常真判：判据根缺失即 fail-closed
 * （不因「未跟踪」而静默跳过）。
 *
 * @param root 仓库根
 * @param rel 平台面判据根（相对仓库根，如 `Cargo.lock`、`Backup/latest`）
 */
export function platformFaceHeld(root: string, rel: string): boolean {
  if (repoRoleOf(root) !== "ns") return true;
  return trackedInRepo(root, rel);
}

/** 「ns 档位不持有平台面」的统一跳过提示（与 {@link nsFaceSkipNotice} 镜像；显式跳过，非静默绿）。 */
export function platformFaceSkipNotice(
  rule: string,
  root: string,
  rel: string,
): void {
  console.log(
    `⏭ ${rule}: ns 档位不持有平台判据根 ${rel}（不入 ns 仓；平台面经符号链接消费）—— 跳过判定（判定归平台档位）`,
  );
}
