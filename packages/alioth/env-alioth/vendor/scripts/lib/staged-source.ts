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

/** 索引内容缓冲上限（最大快照 ~93MB；超限 fail-closed 而非静默放行） */
const INDEX_BLOB_MAX_BYTES = 256 * 1024 * 1024;

/**
 * 读取暂存索引中的文件内容（与工作树版本无关）。
 *
 * 返回值语义（2026-09-23「不接受豁免」加固）：`null` **仅**表示「该路径不在索引中」这一正常情形；
 * 其余读取失败（超缓冲上限 / IO / 截断）一律 **throw（fail-closed）**——判定输入不可得 MUST NOT
 * 伪装成「无需判定」。原实现无 `maxBuffer`（Node 默认 1 MB）⇒ 任何 >1MB 的暂存 `.sql`
 * （实测 `Backup/{ts}/ddl/schema.sql` 1.2MB、元数据面快照 92MB）读取失败被静默当「不在索引」跳过，
 * 形成**按文件大小**的静默豁免面。
 */
export function readIndexBlob(path: string): string | null {
  const inIndex = spawnSync('git', ['cat-file', '-e', `:${path}`], { encoding: 'utf8' });
  if (inIndex.status !== 0) return null; // 不在索引：正常路径（rename 旧路径 / 未暂存）
  const r = spawnSync('git', ['show', `:${path}`], { encoding: 'utf8', maxBuffer: INDEX_BLOB_MAX_BYTES });
  if (r.status !== 0 || r.error !== undefined) {
    const code = (r.error as { code?: string } | undefined)?.code;
    const detail = code ?? (r.stderr ?? '').trim().split('\n')[0] ?? `exit ${r.status}`;
    throw new Error(`暂存内容不可读（判定输入不可得，fail-closed）: ${path} — ${detail}`);
  }
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
