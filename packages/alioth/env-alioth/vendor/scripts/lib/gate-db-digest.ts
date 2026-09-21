//! gate-db-digest.ts — 「DB 派生门禁」的缓存键（DB 摘要 + 目标文件 + 生成器）
//!
//! 适用对象（两者同形：从 DB 元数据生成整文件 → 与已提交文件比对 → 漂移则改写并阻断）：
//!   · gate `context-fields-sync` → scripts/check/check-context-fields.ts（目标
//!     `Framework/backend/approval/src/context_meta.rs`）
//!   · gate `fk-index`（push）→ scripts/check/check-fk-index.ts（目标
//!     `Framework/backend/crud/src/fk_index.rs`）
//!
//! 为什么共享：两个生成器读的是**同一批元数据**（`isahl_meta.meta_fields` / `meta_collections`
//! 与 isahl 物理面）。键面若各写一份，日后任一生成器增读一列就会出现「只更新了一处」的漏失效
//! （= 假通过，最坏那类 bug）。故键面只在此定义一次：**两个生成器读取面的并集**。
//! 通用缓存原语（键读写 / 逃生开关 / 原子写）在 `scripts/lib/gate-cache.ts`，此处 re-export
//! 以便既有调用方（两个 check 与 lib 测试）无需改导入路径。
//!
//! 依赖：`psql`（只读 SELECT；缺失或库不可达 ⇒ 缓存降级为全量，不影响判定）与 `git`。
//!
//! DB 摘要面（穷举自两个生成器的 SELECT 列表；**过失效优于漏失效**，但不含未读列，
//! 否则他 session 改 `updated_at` 之类就会白跑全量）：
//!   meta_collections: table_name,name
//!   meta_fields:      fk_collection,name,title,category,data_type,
//!                     config->reference_config->>{local_key,target_table,junction_table,
//!                     junction_left_key,junction_right_key}（只取任一生成器会读到的行）
//!   pg_catalog（nspname='isahl'）：relkind/relname + attname/atttypid + 被引用类型的 typname/typtype
//!     **直读 catalog 而非 information_schema**：后者是视图、按行做权限判定（实测 4.5s vs catalog
//!     百毫秒级），且 catalog 面是其超集（过失效方向）。
//!   **成本现实**：摘要是一次查询，实测 0.2–0.5s（库空闲）到数秒（他 session 压库时）——方差由共享
//!   dev 库负载主导，非 SQL 形态；缓存的价值恰在库忙时最大（它免掉的是**生成器**：fk-index
//!   6.5–17s、context-fields 1–82s，且同样要付 DB 延迟）。

import { spawnSync } from 'node:child_process';
import { readFile } from 'node:fs/promises';

import { cacheDisabled, sha256Short } from './gate-cache';

export { cacheDisabled, cacheFile, cacheHit, cacheWrite } from './gate-cache';

/** 单一 SQL：覆盖两个生成器读取面的并集，返回 md5 摘要。 */
const DIGEST_SQL = `SELECT md5(
    coalesce((SELECT string_agg(t.table_name || ':' || t.name, '|' ORDER BY t.table_name)
              FROM isahl_meta.meta_collections t), '') || '::' ||
    coalesce((SELECT string_agg(
                f.fk_collection || ':' || f.name || ':' || coalesce(f.title, '') || ':' ||
                f.category::text || ':' || f.data_type::text || ':' ||
                coalesce(f.config->'reference_config'->>'local_key', '') || ':' ||
                coalesce(f.config->'reference_config'->>'target_table', '') || ':' ||
                coalesce(f.config->'reference_config'->>'junction_table', '') || ':' ||
                coalesce(f.config->'reference_config'->>'junction_left_key', '') || ':' ||
                coalesce(f.config->'reference_config'->>'junction_right_key', ''),
                '|' ORDER BY f.fk_collection, f.name)
              FROM isahl_meta.meta_fields f
              -- 只取**任一生成器会读到**的行（auto 且无 reference_config 的行两个生成器都过滤掉）：
              -- 行集仍是读面超集（category 翻转 auto↔非 auto 会改变筛选结果 ⇒ 摘要随之变），但文本量大幅下降
              WHERE f.category::text <> 'auto' OR f.config ? 'reference_config'), '') || '::' ||
    coalesce((SELECT string_agg(n.nspname || '.' || c.relname || ':' || i.inhparent::text, ',' ORDER BY 1)
              FROM pg_inherits i JOIN pg_class c ON c.oid = i.inhrelid
              JOIN pg_namespace n ON n.oid = c.relnamespace WHERE n.nspname = 'isahl'), '') || '::' ||
    -- 表/列/类型三面直读 pg_catalog（不用 information_schema：后者是视图，按行做权限判定，
    -- 实测 4.5s vs catalog 直读百毫秒级；且 catalog 面是 information_schema 的超集 ⇒ 过失效方向）
    coalesce((SELECT string_agg(c.relname, ',' ORDER BY 1)
              FROM pg_class c JOIN pg_namespace n ON n.oid = c.relnamespace
              WHERE n.nspname = 'isahl' AND c.relkind IN ('r','p','v','m','f')), '') || '::' ||
    -- 用 atttypid（OID）而非 format_type(...)：便宜（不做逐行 catalog 格式化）且对生成器输出等价——
    -- 两者读的是 information_schema 的 data_type/udt_name，其中不含 typmod（varchar 长度），
    -- 而类型身份（含 enum 重建换 OID）由 atttypid 捕获 ✓
    coalesce((SELECT string_agg(c.relname || '.' || a.attname || ':' || a.atttypid::text, ',' ORDER BY 1)
              FROM pg_attribute a JOIN pg_class c ON c.oid = a.attrelid
              JOIN pg_namespace n ON n.oid = c.relnamespace
              WHERE n.nspname = 'isahl' AND a.attnum > 0 AND NOT a.attisdropped), '') || '::' ||
    coalesce((SELECT string_agg(t.typname || ':' || t.typtype::text, ',' ORDER BY 1)
              FROM pg_type t
              WHERE t.oid IN (SELECT DISTINCT a.atttypid FROM pg_attribute a
                              JOIN pg_class c ON c.oid = a.attrelid JOIN pg_namespace n ON n.oid = c.relnamespace
                              WHERE n.nspname = 'isahl' AND a.attnum > 0 AND NOT a.attisdropped)), '')
  ) AS digest`;

/**
 * 读 DB 摘要；不可得（psql 缺失/库不可达/查询失败）⇒ null（调用方降级为全量），并**打印原因**不静默。
 */
export async function dbDigest(): Promise<string | null> {
  const url =
    process.env.DATABASE_URL ?? 'postgres://alioth_readonly@localhost:5432/aliothstudio_dev';
  // 用 psql（仓库只读审计既有模式，见 check-actor-identity-integrity.ts / lib/db-ready.sh）：
  // 免 Bun.SQL 依赖（scripts/lib 作用域无 bun 类型）、进程与连接开销更小；SQL 经 -c 传入，
  // 结果取 -tA 单行。
  const res = spawnSync('psql', [url, '-tA', '-c', DIGEST_SQL], { encoding: 'utf8', timeout: 60_000 });
  if (res.error !== undefined || res.status !== 0) {
    console.warn(
      `⚠️ gate-db-digest: DB 摘要不可得（psql rc=${String(res.status)}），降级全量判定。${String(
        res.stderr ?? '',
      )
        .trim()
        .slice(0, 160)}`,
    );
    return null;
  }
  const digest = (res.stdout ?? '').trim();
  return digest === '' ? null : digest;
}

/**
 * 计算缓存键：**任一侧不可得或缓存被禁用 ⇒ null**（调用方据此走全量）。
 * `target` / `generator` 用**内容**（非 mtime/路径）。
 */
export async function digestCacheKey(
  targetPath: string,
  generatorPath: string,
): Promise<string | null> {
  if (cacheDisabled()) return null;
  const digest = await dbDigest();
  if (digest === null) return null;
  const target = await readFile(targetPath, 'utf8').catch(() => null);
  const generator = await readFile(generatorPath, 'utf8').catch(() => null);
  if (target === null || generator === null) return null;
  return sha256Short(`${digest}|${sha256Short(target)}|${sha256Short(generator)}`);
}
