//! gate-cache.ts — 门禁「通过态缓存」的通用原语（键计算与读写；不含任何判定逻辑）
//!
//! 适用：任何**判定输入可完整表达为「一组文件」**的门禁（整仓扫描类，如
//! `sql-table-quoting` / `code-table-refs`；以及 DB 派生类，见 gate-db-digest.ts）。
//! 语义（与 check-version-alignment.sh 同款模式）：
//!   命中且上次**通过** ⇒ 调用方立即返回并打印 `[cached]`；失败/阻断/跳过一律不写；
//!   `ALIOTH_NO_GATE_CACHE=1` 强制全量。
//! 为什么安全：键覆盖**判定输入全体**——输入一变键即变，故不存在「窗口内漏检」；
//!   这是「跳过」与「时间窗」的本质区别（时间窗会在窗口内漏检真实违规）。
//! 键的空隙（明说）：`scanSurfaceDigest` 用 `mtime(ms)+size` 而非内容哈希（读全部内容
//!   即等于重跑扫描 ⇒ 省不下来）；与仓库既有 version-alignment 缓存同粒度，同毫秒内
//!   写入同尺寸内容属理论情形。
//!
//! 接入下一个整仓扫描门禁的配方（2026-09-14；已在 check-qualified-table-quoting 验证：
//! 冷 18s → 温 2s，注入真违规仍必被检出）：
//!   1. 把「枚举 + 扫描」拆两段：`collectFiles(dir, out)`（只 readdir/stat，不读内容）
//!      + `for (const f of files) scanFile(f)`——保持 DFS 同序，输出顺序不变；
//!   2. `const key = await scanSurfaceDigest(files, new URL(import.meta.url).pathname)`
//!      → `if (await cacheHit(NAME, key)) { print '[cached]'; exit 0 }`；
//!   3. 仅在该门禁**通过**的分支 `await cacheWrite(NAME, key)`；有违规/阻断一律不写；
//!   4. 必测四态：冷 / 温 / `ALIOTH_NO_GATE_CACHE=1` / **注入真违规必须被检出**（证缓存不掩盖）；
//!   5. 剩余整仓扫描门禁（code-table-refs / id-json-precision / code-semantics / rust-serde-attrs /
//!      page-context / ontology-contract）各自扫描面不同，需逐个枚举接入（这些脚本常被他 session
//!      同时编辑 ⇒ 宜小步提交、逐个验证）。

import { spawnSync } from 'node:child_process';
import { createHash } from 'node:crypto';
import { existsSync, statSync } from 'node:fs';
import { mkdir, readFile, rename, writeFile } from 'node:fs/promises';
import { dirname, join } from 'node:path';

/** 缓存总开关：`ALIOTH_NO_GATE_CACHE=1` 强制全量。 */
export const cacheDisabled = (): boolean => process.env.ALIOTH_NO_GATE_CACHE === '1';

/** 缓存文件路径：`<git-common-dir>/../.parallel/cache/<name>`（worktree 与主 checkout 共享一份）。 */
export function cacheFile(name: string): string | null {
  const common = spawnSync('git', ['rev-parse', '--git-common-dir'], { encoding: 'utf8' });
  const raw = (common.stdout ?? '').trim();
  if (raw === '') return null;
  const abs = raw.startsWith('/') ? raw : join(process.cwd(), raw);
  return join(dirname(abs), '.parallel', 'cache', name);
}

/** 命中判定：缓存文件存在且键相同。`key === null`（缓存不可用）⇒ 永不命中。 */
export async function cacheHit(name: string, key: string | null): Promise<boolean> {
  if (key === null) return false;
  const path = cacheFile(name);
  if (path === null || !existsSync(path)) return false;
  const cached = (await readFile(path, 'utf8').catch(() => '')).trim();
  return cached === key;
}

/** 写缓存（**仅在通过时**由调用方调用）；原子替换，失败静默（缓存是优化，不得影响判定）。 */
export async function cacheWrite(name: string, key: string | null): Promise<void> {
  if (key === null) return;
  const path = cacheFile(name);
  if (path === null) return;
  const tmp = `${path}.tmp-${process.pid}`;
  await mkdir(dirname(path), { recursive: true }).catch(() => undefined);
  const ok = await writeFile(tmp, `${key}\n`, 'utf8')
    .then(() => true)
    .catch(() => false);
  if (ok) await rename(tmp, path).catch(() => undefined);
}

export function sha256Short(text: string): string {
  return createHash('sha256').update(text).digest('hex').slice(0, 32);
}

/**
 * 整仓扫描面摘要：对**该门禁自己枚举出的文件列表**取 `relpath:mtimeMs:size` + 脚本自身内容。
 * 调用方须先做枚举（stat 级，便宜），再据此判定是否跳过内容扫描（贵）。
 *
 * 不可 stat 的文件（`git ls-files` 会列出「已删未提交」的路径）MUST 编码为缺位标记而非
 * 放弃缓存：早先版本遇缺失即返回 null ⇒ 只要工作区有一处在途删除，该门禁就**永久不命中**
 * （2026-09-14 实测 code-table-refs 因此从不命中）。缺位标记同时保证「文件出现/消失」改变键。
 */
export async function scanSurfaceDigest(files: readonly string[], selfPath: string): Promise<string | null> {
  if (cacheDisabled()) return null;
  const self = await readFile(selfPath, 'utf8').catch(() => null);
  if (self === null) return null;
  const parts: string[] = [sha256Short(self)];
  for (const file of files) {
    try {
      const st = statSync(file);
      parts.push(`${file}:${Math.floor(st.mtimeMs)}:${st.size}`);
    } catch {
      parts.push(`${file}:<absent>`);
    }
  }
  return sha256Short(parts.join('\n'));
}
