/**
 * caller-root.ts — 检查脚本的 ROOT 归属：取**调用方仓**（2026-09-25 判定面归属改造）。
 *
 * 问题：`const ROOT = resolve(import.meta.dirname, "../..")` 经 ns 仓 `scripts` 符号链接
 * realpath 后**恒落平台仓** —— ns 仓调用时判的是平台仓 `Pre-Proc/`（开发机全为符号链接挂载，
 * 枚举恒 0）⇒ 打印「0 个单元」+ ✅ 的**假绿**。见 `.planning/gate-adaptation-assessment.md` §八。
 *
 * 契约：
 * - 调用方 cwd 在 git 仓内，且该仓根含 `Pre-Proc/` ⇒ ROOT = 该仓根
 *   （「内容归谁」= 调用方仓：ns 仓判自己的 ns 面，平台仓判平台面）。
 * - 否则（git 不可用 / 非 git 目录 / 仓根无 `Pre-Proc/`，如 `Meta/` 子仓、`/tmp`）⇒ **回落**
 *   脚本位置派生 `resolve(import.meta.dirname, "../..")`：只**新增**「从 ns 仓调用」的正确归属，
 *   「从仓库根调用」的既有语义逐字不变。
 *
 * 边界：本模块只定**扫描面**。「本仓是否持有该面」由 `ns-surface.ts::nsSourcesTracked` 判定，
 * 基线写入归属由 `check/lib/baseline-store.ts` 的 INV-1 守卫判定（本模块不参与写入拦截）。
 */
import { existsSync } from "node:fs";
import { join, resolve } from "node:path";
import { repoRootOf } from "./repo-role.ts";

/** 脚本位置派生根（= 改造前的既有口径；git 不可用或调用方仓无 `Pre-Proc/` 时的回落值）。 */
const SCRIPT_ROOT = resolve(import.meta.dirname, "../..");

/**
 * 调用方仓根。
 *
 * 判据 = 调用方仓根（`repoRootOf(process.cwd())`，正本 = `scripts/lib/repo-role.ts`）存在且含
 * `Pre-Proc/` 目录；任一不满足 ⇒ 回落 {@link SCRIPT_ROOT}（= 改造前行为，保证平台仓既有调用语义不变）。
 *
 * 本模块 MUST NOT 自持 git 调用（2026-10-08 收敛：原实现内联 `spawnSync git rev-parse`，
 * 仓身份判定因此有第二份 TS 实现 → 门禁 `check-repo-root-single-source.ts`）。
 */
export function callerRepoRoot(): string {
  const top = repoRootOf(process.cwd());
  if (top !== null && existsSync(join(top, "Pre-Proc"))) return top;
  return SCRIPT_ROOT;
}
