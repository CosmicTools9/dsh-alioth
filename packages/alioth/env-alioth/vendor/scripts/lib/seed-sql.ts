/**
 * seed-sql.ts — 种子 SQL 载体判据（单一真相源）
 *
 * 自 `scripts/check/check-seed-sql.ts` 抽出（fix-seed-comments-json-writes）：种子载体定义
 * 被两条门禁共同消费（良构/ID 同类性 + comments JSON 字面量），复制两份必然漂移。
 */

/** 种子 SQL 载体判定（仓库相对路径）——所有消费者共用单一判据 */
export function isSeedSqlRel(rel: string): boolean {
  if (!rel.endsWith('.sql')) return false;
  if (rel.startsWith('Framework/seed/') || rel.startsWith('scripts/seed/')) return true;
  if (rel.startsWith('scripts/db/seed-')) return true;
  const segs = rel.split('/');
  return segs.length === 4 && segs[0] === 'Pre-Proc' && (segs[2] === 'seed' || segs[2] === 'test-data');
}

/** 种子目录（相对仓库根）——两门禁的 walk 面一致 */
export const SEED_DIRS = ['Framework/seed', 'scripts/seed', 'scripts/db'] as const;
