//! str-order.ts — 生成物排序的唯一比较器（码元序 / code-unit）
//!
//! 为什么单独成模块：生成物（`fk_index.rs`、`context_meta.rs` 等）的顺序 MUST 与
//! **运行环境无关**。两类环境依赖必须排除：
//!   1. SQL `ORDER BY` 的比较语义由 DB collation 决定（dev 库 `zh_CN.UTF-8` 下 `-`
//!      为可忽略字符：`'a-b' < 'ab'`，与 JS 码元序相反）；
//!   2. `String.prototype.localeCompare` 走 ICU 默认 locale（随运行时/版本变化）。
//!
//! 故生成器与校验门禁共用本比较器：`a < b` / `a > b` 即 UTF-16 码元序，纯函数、零依赖。

/** 码元序比较器（与 `Array.prototype.sort` 契约一致：负 / 0 / 正）。 */
export function byteCmp(a: string, b: string): number {
  return a < b ? -1 : a > b ? 1 : 0;
}

/** 按多个键依次做码元序比较（首个非 0 结果即结论）。 */
export function byteCmpKeys(a: readonly string[], b: readonly string[]): number {
  for (let i = 0; i < Math.min(a.length, b.length); i += 1) {
    const c = byteCmp(a[i]!, b[i]!);
    if (c !== 0) return c;
  }
  return a.length - b.length;
}
