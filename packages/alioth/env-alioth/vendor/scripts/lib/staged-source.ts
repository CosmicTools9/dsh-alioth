/**
 * staged-source.ts — git 暂存面读取（门禁共用）
 *
 * 用途：commit 阶段门禁以**索引内容**为判定输入（工作树可能与暂存集不同）。
 * 调用方：`scripts/check/check-isahl-ddl-boundary.ts`、`scripts/check/check-code-inline-sql.ts`。
 * 实现用 `node:child_process`（scripts/lib 作用域无 bun 类型，与 gate-db-digest.ts 同约定）。
 */
import { spawnSync } from 'node:child_process';

export interface StagedEntry {
  file: string;
  /** A=新增 / M=修改 / C=复制 / R=重命名（`--diff-filter=ACMR`） */
  status: string;
}

/** 读取暂存索引中的文件内容（与工作树版本无关）；文件不在索引中 → null */
export function readIndexBlob(path: string): string | null {
  const r = spawnSync('git', ['show', `:${path}`], { encoding: 'utf8' });
  if (r.status !== 0) return null;
  return r.stdout;
}

/** 暂存条目（file + 变更状态）：按给定 pathspec 过滤 */
export function stagedEntries(pathspecs: string[]): StagedEntry[] {
  const r = spawnSync('git', ['diff', '--cached', '--name-status', '--diff-filter=ACMR', '--', ...pathspecs], {
    encoding: 'utf8',
  });
  return (r.stdout ?? '')
    .split('\n')
    .filter(Boolean)
    .map((line: string) => {
      const tab = line.indexOf('\t');
      return { status: line.slice(0, tab), file: line.slice(tab + 1) };
    });
}
