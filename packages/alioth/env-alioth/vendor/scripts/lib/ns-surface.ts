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
import { spawnSync } from 'node:child_process';

/**
 * 本仓是否跟踪 ns 源（`Pre-Proc/<ns>/Sources/…`）。
 *
 * @param root 仓库根（默认当前目录；脚本通常传自身解析出的 ROOT）
 */
export function nsSourcesTracked(root = '.'): boolean {
  const r = spawnSync('git', ['-C', root, 'ls-files', '-z', 'Pre-Proc/*/Sources/*'], {
    encoding: 'utf8',
  });
  const stdout = r.stdout ?? '';
  return stdout.split('\u0000').some((f) => f !== '');
}
