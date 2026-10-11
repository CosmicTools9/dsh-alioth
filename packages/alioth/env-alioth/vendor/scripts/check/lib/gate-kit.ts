#!/usr/bin/env bun
/**
 * gate-kit.ts — 检查门禁的共享**纯工具面**（不含 argv / IO / import 期副作用）。
 *
 * 本模块只承载跨检查复用的**纯判据与纯函数**：
 *   · `EXCLUDE_DIRS` / `isExcludedDirName` / `isExcludedPath`（「哪些目录不被判定」的唯一属主）
 *   · `Violation`（违规条目类型）
 *   · `nsScopes` / `portsTableRoot`（ns 面双档位的单一实现）
 *   · `isCommentOnlyLine` / `slotLiteralShapes`（行判据与端口字面量形态的单一实现）
 *
 * **仓库根 / 文件域 / 遍历 / 行读取 / 报告 / 基线 IO 一律经检查基座**——`scripts/check/lib/check-interface.ts`
 * 的 `ctx`（`ctx.root` / `ctx.files` / `ctx.fileDomainMode` / `ctx.walk` / `ctx.readText`）与报告缝，以及
 * `scripts/check/lib/baseline-store.ts`。本模块 MUST NOT 再导出 argv 派生值或 IO 助手：import 期读
 * `process.argv` 会让凡导入它的检查都无法在不伪造 argv 的情况下被完整测试。
 * 判据 = `openspec/specs/precommit-execution` :: 检查的 IO 与根 MUST 经唯一基座。
 */
import { existsSync, readdirSync, realpathSync } from 'node:fs';
import { join } from 'node:path';

/**
 * 顶层排除目录（与 check-sqlx-test-forbidden.ts 口径一致 + 部署/产物面）。
 *
 * **单一属主**：「哪些目录不被判定」只在此声明；检查脚本 MUST NOT 在本地再声明一份
 * （`check-exclude-dirs-single-source.ts` 以 AST 判据阻断副本）。各门禁**特有**的附加过滤
 * （如 `vendor/build/Prototypes/.planning/openspec`）以 `{ ...EXCLUDE_DIRS, <extra>: true }` 组合，
 * 不从本集合里增删。判据消费入口 = `isExcludedDirName` / `isExcludedPath` / 基座 `ctx.walk`。
 *
 * `.parallel/` = parallel 战役的 agent 基础设施（worktree 与平台克隆；`.git/info/exclude` 覆盖、不入库）。
 * 不排除会把克隆体当仓库内容扫（实测 2026-09-25：cors-single-source 在 ns 仓扫到
 * `.parallel/worktrees/{AliothMeta,wt-1..4}/Framework/.../cors.rs` 5 处伪违规 ⇒ push 档被阻断）。
 */
export const EXCLUDE_DIRS: Record<string, true> = {
  node_modules: true,
  target: true,
  dist: true,
  '.git': true,
  '.codegraph': true,
  Deploy: true,
  Release: true,
  Backup: true,
  '.parallel': true,
};

/**
 * 目录名判据（**单一实现**）：`excludeDirs` 成员或名称含 `node_modules` 的变体（如 `.pnpm` 内的嵌套名）。
 * 缺省集合 = `EXCLUDE_DIRS`；各门禁的特有附加过滤经 `{ ...EXCLUDE_DIRS, ... }` 组合后传入。
 */
export function isExcludedDirName(name: string, excludeDirs: Record<string, true> = EXCLUDE_DIRS): boolean {
  return excludeDirs[name] === true || name.includes('node_modules');
}

/**
 * 仓库相对路径判据（**单一实现**，posix 分隔）：任一路径段命中 `isExcludedDirName` 即排除。
 * 供 `include` 过滤器 / git 清单过滤 / 行级扫描器等「按相对路径跳过」的调用方使用。
 */
export function isExcludedPath(rel: string, excludeDirs: Record<string, true> = EXCLUDE_DIRS): boolean {
  return rel.split('/').some((seg) => isExcludedDirName(seg, excludeDirs));
}

export interface Violation {
  file: string; // 仓库相对路径
  line: number;
  text: string; // 命中行摘要（≤120 字符）
}

/**
 * 当前仓可见的 ns 面（ns 名 + ns 仓根）——**双档位单一实现**。
 *
 * 判据 = `<root>/Pre-Proc/*` 目录/符号链接的 realpath 形态为 `<nsRepo>/Pre-Proc/<ns>`：
 *   · 平台仓档位 ⇒ 全部挂载的 ns；
 *   · ns 仓档位 ⇒ 本仓唯一 ns。
 * 形态不符（`Pre-Proc` 不在 realpath 倒数第二段）⇒ 跳过（非 ns 仓布局）；无 `Pre-Proc/` ⇒ 空。
 */
export function nsScopes(root: string): Array<{ ns: string; repoRoot: string }> {
  const dir = join(root, 'Pre-Proc');
  if (!existsSync(dir)) return [];
  const out: Array<{ ns: string; repoRoot: string }> = [];
  for (const e of readdirSync(dir, { withFileTypes: true })) {
    if (e.name.startsWith('.') || !(e.isDirectory() || e.isSymbolicLink())) continue;
    let real: string;
    try {
      real = realpathSync(join(dir, e.name));
    } catch {
      continue;
    }
    const parts = real.split('/');
    if (parts[parts.length - 2] !== 'Pre-Proc') continue;
    out.push({ ns: parts[parts.length - 1]!, repoRoot: parts.slice(0, -2).join('/') });
  }
  return out.sort((a, b) => a.ns.localeCompare(b.ns));
}

/**
 * `Deploy/ports.conf` 所在根（双档位）——**单一实现转出**：实现落 `scripts/lib/ns-mounts.ts`
 * （库面），检查脚本经此处复用（避免 check 面再写一份解析）。
 */
export { portsTableRoot } from '../../lib/ns-mounts';

/**
 * 注释行判据（自然语言面）——**单一实现**：以 `#` / `//` / `*` / `--` / `<!--` 起头即视为注释。
 * 用于「注释可点名他 ns 槽位（防误用告警），代码行不可内联」的共同判定。
 */
export function isCommentOnlyLine(line: string): boolean {
  const t = line.trimStart();
  return t.startsWith('#') || t.startsWith('//') || t.startsWith('*') || t.startsWith('--') || t.startsWith('<!--');
}

/**
 * 端口字面量的「赋值 / URL / shell 缺省」形态——**单一实现**（`check-ns-slot-port-isolation`
 * 与 `check-ns-deploy-frontend-drift` 共用）：`:port`（host:port / URL）、`=port`（KEY=port）、
 * `:-port`（shell `${VAR:-port}` 缺省）。纯数字业务 id（如 JSON `"id": "9009"`）不落此形态。
 */
export function slotLiteralShapes(port: number): readonly RegExp[] {
  return [new RegExp(`:${port}\\b`), new RegExp(`=${port}\\b`), new RegExp(`:-${port}\\b`)];
}
