//! 叶表作用域检索单源模块（change `consolidate-scoped-search-module`）。
//!
//! ── 职责 ──
//! - 注册表 [`ScopeTable`]：scope 名 → 物理叶表 + 检索列 + 可选分级列；
//! - 执行器 [`search_scoped`]：逐叶表 ILIKE 检索 → 三档相关性排序 → 真分页（真 `total` + `has_more`）；
//! - 检索内核 [`tokenize`] / [`trigram_similarity`] / [`fuzzy_score`] / [`MatchPattern`]：
//!   自 `Meta/backend/src/search/service.rs` 上提（语义逐字不变），Meta 与消费端点共用。
//!
//! ── 硬约束 ──
//! - 表名 / 列名 MUST 为注册表携带的编译期字面量（执行器只做单态化拼接，无运行期来源）；
//! - 检索模式 MUST 经 [`crate::search::like_pattern`] / [`crate::search::escape_like`]（转义单源）；
//! - 关键词 / limit / level 一律绑定参数。
//!
//! 门禁注记：本文件 `format!` 内的 SQL 插值站点属 `generic-type` 类（编译期常量集单态化，
//! 非运行期来源），登记于 `scripts/check/baselines/dynamic-table-name.baseline.json`。

use std::cmp::Ordering;
use std::collections::BTreeSet;

use sqlx::PgPool;

use crate::search::{escape_like, like_pattern, strip_regex_wrapper};

// ─────────────────────────────── 注册表 / 请求 / 结果 ───────────────────────────────

/// 作用域 → 叶表注册表条目（表名与列名均为编译期字面量）。
#[derive(Debug, Clone, Copy)]
pub struct ScopeTable {
    /// 对外 scope 名（如 `"civil"` / `"air-caac"`）
    pub scope: &'static str,
    /// 物理叶表（不含 schema；执行器以 `isahl."…"` 引用）
    pub table: &'static str,
    /// 检索列（`columns[0]` 同时为 rank 主列）
    pub columns: &'static [&'static str],
    /// 可选附加列（分级列）：参与投影（映射为 [`ScopedHit::level`]）与 `level` 过滤
    pub level_column: Option<&'static str>,
}

/// 检索请求（`keywords` 互为 OR 并集；空 `scopes` = 全部登记项）。
#[derive(Debug)]
pub struct ScopedRequest<'a> {
    pub keywords: &'a [String],
    pub scopes: &'a [String],
    /// 分级过滤（仅当命中 scope 声明了 `level_column` 时生效）
    pub level: Option<&'a str>,
    pub limit: i64,
    pub offset: i64,
}

/// 命中行（三档 rank；`source_table` 为物理表名）。
#[derive(Debug, Clone)]
pub struct ScopedHit {
    pub id: i64,
    pub code: Option<String>,
    pub title: Option<String>,
    pub body: Option<String>,
    pub source_table: &'static str,
    /// 分级列值（scope 声明 `level_column` 时；否则为 `None`）
    pub level: Option<String>,
    pub rank: f64,
}

/// 分页结果（`total` = 真实命中总数；`has_more = offset + hits.len() < total`）。
#[derive(Debug, Clone)]
pub struct ScopedPage {
    pub hits: Vec<ScopedHit>,
    pub total: i64,
    pub has_more: bool,
}

/// scope 列表 → 注册表子集（空 = 全部登记项；未知 scope 忽略；同表去重且保序）。
pub fn resolve_scopes<'a>(registry: &'a [ScopeTable], scopes: &[String]) -> Vec<&'a ScopeTable> {
    let mut out: Vec<&'a ScopeTable> = Vec::new();
    let push = |t: &'a ScopeTable, out: &mut Vec<&'a ScopeTable>| {
        if !out.iter().any(|prev| prev.table == t.table) {
            out.push(t);
        }
    };
    if scopes.is_empty() {
        for t in registry {
            push(t, &mut out);
        }
        return out;
    }
    for scope in scopes {
        if let Some(t) = registry.iter().find(|t| t.scope == scope) {
            push(t, &mut out);
        }
    }
    out
}

/// 命中排序：rank 降序 → 来源表 → id（跨表同分不抖动；Meta `sort_results` 的作用域同形）。
pub fn sort_results(hits: &mut [ScopedHit]) {
    hits.sort_by(|a, b| {
        b.rank
            .partial_cmp(&a.rank)
            .unwrap_or(Ordering::Equal)
            .then_with(|| a.source_table.cmp(b.source_table))
            .then_with(|| a.id.cmp(&b.id))
    });
}

// ─────────────────────────────────── 执行器 ───────────────────────────────────

/// 叶表作用域检索：逐叶表 ILIKE → 三档 rank → 全局排序 → 真分页。
///
/// 每表取 `limit + offset` 行（全局排序后切页不缺行），`total` 为该查询下逐叶表
/// `COUNT(*)` 之和（真实命中总数，MUST NOT 以截断条数冒充）。
pub async fn search_scoped(
    pool: &PgPool,
    registry: &[ScopeTable],
    req: &ScopedRequest<'_>,
) -> Result<ScopedPage, sqlx::Error> {
    let tables = resolve_scopes(registry, req.scopes);

    // 转义单源：contains 走 like_pattern；exact / prefix 在主列的小写形态上构造。
    let contains: Vec<String> = req.keywords.iter().map(|k| like_pattern(k)).collect();
    let exacts: Vec<String> = req
        .keywords
        .iter()
        .map(|k| strip_regex_wrapper(k).to_lowercase())
        .collect();
    let prefixes: Vec<String> = req
        .keywords
        .iter()
        .map(|k| format!("{}%", escape_like(&strip_regex_wrapper(k).to_lowercase())))
        .collect();

    let limit = req.limit.max(0);
    let offset = req.offset.max(0);
    let fetch = limit + offset;

    let mut hits: Vec<ScopedHit> = Vec::new();
    let mut total: i64 = 0;

    for scope in &tables {
        let table = scope.table;
        let main = scope.columns[0];
        let search_pred = scope
            .columns
            .iter()
            .map(|c| format!("{c} ILIKE ANY($1::text[])"))
            .collect::<Vec<_>>()
            .join(" OR ");
        let level_value = req.level.filter(|_| scope.level_column.is_some());
        let (level_proj, level_pred, limit_pos) = match (level_value, scope.level_column) {
            (Some(_), Some(lc)) => (
                format!(", {lc}::text AS level"),
                format!(" AND {lc}::text = $4"),
                5,
            ),
            (None, Some(lc)) => (format!(", {lc}::text AS level"), String::new(), 4),
            (_, None) => (", NULL::text AS level".to_string(), String::new(), 4),
        };
        // 表名 / 列名来自注册表编译期字面量（单态化常量集，无注入面）；模式与值一律绑定参数。
        let sql = format!(
            "SELECT id, code, notice, comments{level_proj}, \
             CASE WHEN lower({main}) = ANY($2::text[]) THEN 1.0 \
                  WHEN lower({main}) LIKE ANY($3::text[]) THEN 0.9 \
                  WHEN {main} ILIKE ANY($1::text[]) THEN 0.7 \
                  ELSE 0.4 END::float8 AS rank \
             FROM isahl.\"{table}\" \
             WHERE deleted_at IS NULL AND ({search_pred}){level_pred} \
             ORDER BY rank DESC, id ASC LIMIT ${limit_pos}"
        );
        let mut query = sqlx::query_as::<
            _,
            (
                i64,
                Option<String>,
                Option<String>,
                Option<String>,
                Option<String>,
                f64,
            ),
        >(sqlx::AssertSqlSafe(sql))
        .bind(&contains)
        .bind(&exacts)
        .bind(&prefixes);
        if let Some(level) = level_value {
            query = query.bind(level);
        }
        let rows = query.bind(fetch).fetch_all(pool).await?;
        for (id, code, notice, comments, level, rank) in rows {
            hits.push(ScopedHit {
                id,
                code,
                title: notice,
                body: comments,
                source_table: table,
                level,
                rank,
            });
        }

        let count_sql = format!(
            "SELECT COUNT(*) FROM isahl.\"{table}\" \
             WHERE deleted_at IS NULL AND ({search_pred}){count_level_pred}",
            count_level_pred = match (level_value, scope.level_column) {
                (Some(_), Some(lc)) => format!(" AND {lc}::text = $2"),
                _ => String::new(),
            }
        );
        let mut count_query =
            sqlx::query_scalar::<_, i64>(sqlx::AssertSqlSafe(count_sql)).bind(&contains);
        if let Some(level) = level_value {
            count_query = count_query.bind(level);
        }
        total += count_query.fetch_one(pool).await?;
    }

    sort_results(&mut hits);

    let page: Vec<ScopedHit> = hits
        .into_iter()
        .skip(offset as usize)
        .take(limit as usize)
        .collect();
    let has_more = offset + (page.len() as i64) < total;
    Ok(ScopedPage {
        hits: page,
        total,
        has_more,
    })
}

// ────────────────────────────── 检索内核（自 Meta 上提） ──────────────────────────────

/// 模糊搜索每类候选上限（打分前的粗筛规模）
pub const FUZZY_CANDIDATE_CAP: i64 = 400;
/// 模糊候选粗筛使用的三元组 LIKE 模式上限
pub const FUZZY_TRIGRAM_PATTERNS: usize = 12;
/// 模糊搜索候选长度窗口上浮（长度 ≤ query 长度 + 该值）
pub const FUZZY_LEN_SPAN: i64 = 6;
/// 模糊搜索候选长度窗口下沉（长度 ≥ query 长度 - 该值）
pub const FUZZY_LEN_SLACK: i64 = 4;

/// 匹配模式三档（大小写不敏感；正则包裹 `/pattern/` 只取内层文本，与前端 `matchesQuery` 一致）。
///
/// 构造单源：`contains` 经 [`like_pattern`]（转义），`prefix` 经 [`escape_like`] + `%`，
/// `exact` 为小写原文（等值比较无通配面）。
pub struct MatchPattern {
    /// `%q%`：名称/描述包含
    pub contains: String,
    /// `q%`：名称前缀（小写）
    pub prefix: String,
    /// `q`：名称全等（小写）
    pub exact: String,
}

impl MatchPattern {
    pub fn new(query: &str) -> Self {
        let raw = strip_regex_wrapper(query);
        let lower = raw.to_lowercase();
        Self {
            contains: like_pattern(raw),
            prefix: format!("{}%", escape_like(&lower)),
            exact: lower,
        }
    }
}

/// 查询切分：非字母数字（`_`/`-`/空白/标点）与 camelCase 边界
pub fn tokenize(query: &str) -> Vec<String> {
    let mut tokens: Vec<String> = Vec::new();
    let mut current = String::new();
    let mut prev_lower = false;

    for ch in query.chars() {
        if ch.is_alphanumeric() {
            if ch.is_uppercase() && prev_lower && !current.is_empty() {
                tokens.push(std::mem::take(&mut current));
            }
            prev_lower = !ch.is_uppercase();
            current.extend(ch.to_lowercase());
        } else {
            if !current.is_empty() {
                tokens.push(std::mem::take(&mut current));
            }
            prev_lower = false;
        }
    }
    if !current.is_empty() {
        tokens.push(current);
    }
    tokens
}

/// 字符三元组集合（首尾补空格，短串亦可比较）
fn trigrams(s: &str) -> BTreeSet<String> {
    let chars: Vec<char> = s.chars().collect();
    let mut padded: Vec<char> = Vec::with_capacity(chars.len() + 3);
    padded.push(' ');
    padded.push(' ');
    padded.extend(chars.iter().copied());
    padded.push(' ');
    padded
        .windows(3)
        .map(|w| w.iter().collect::<String>())
        .collect()
}

/// trigram Jaccard 相似度（`0.0..=1.0`）
pub fn trigram_similarity(a: &str, b: &str) -> f64 {
    let ta = trigrams(a);
    let tb = trigrams(b);
    if ta.is_empty() || tb.is_empty() {
        return 0.0;
    }
    let inter = ta.intersection(&tb).count() as f64;
    let union = (ta.len() + tb.len()) as f64 - inter;
    if union <= 0.0 {
        1.0
    } else {
        inter / union
    }
}

/// 近似度打分：全等 1.0 > 完整包含 0.95 > max(trigram Jaccard, 词元覆盖率 × 0.9)
pub fn fuzzy_score(query: &str, name: &str) -> f64 {
    let q = query.to_lowercase();
    let n = name.to_lowercase();
    if q == n {
        return 1.0;
    }
    if n.contains(&q) {
        return 0.95;
    }
    let tokens = tokenize(query);
    let coverage = if tokens.is_empty() {
        0.0
    } else {
        tokens.iter().filter(|t| n.contains(t.as_str())).count() as f64 / tokens.len() as f64
    };
    trigram_similarity(&q, &n).max(coverage * 0.9)
}

#[cfg(test)]
mod tests {
    use super::*;

    const TEST_REGISTRY: &[ScopeTable] = &[
        ScopeTable {
            scope: "civil",
            table: "zc_id_law-civil-article",
            columns: &["notice", "comments"],
            level_column: Some("fk_jurisdiction"),
        },
        ScopeTable {
            scope: "intl",
            table: "zc_id_law-intl-article",
            columns: &["notice", "comments"],
            level_column: Some("fk_jurisdiction"),
        },
    ];

    #[test]
    fn resolve_scopes_empty_returns_all_and_unknown_ignored() {
        let all = resolve_scopes(TEST_REGISTRY, &[]);
        assert_eq!(all.len(), 2);
        assert_eq!(all[0].table, "zc_id_law-civil-article");

        let known: Vec<String> = vec!["intl".to_string()];
        let hit = resolve_scopes(TEST_REGISTRY, &known);
        assert_eq!(hit.len(), 1);
        assert_eq!(hit[0].scope, "intl");

        let unknown: Vec<String> = vec!["nope".to_string(), "civil".to_string()];
        let mixed = resolve_scopes(TEST_REGISTRY, &unknown);
        assert_eq!(mixed.len(), 1);
        assert_eq!(mixed[0].scope, "civil");

        // 同一 scope 重复（或同表别名）去重，total 不重复计数
        let dup: Vec<String> = vec!["civil".to_string(), "civil".to_string()];
        assert_eq!(resolve_scopes(TEST_REGISTRY, &dup).len(), 1);
    }

    #[test]
    fn sort_results_orders_by_rank_then_table_then_id() {
        let mut hits = vec![
            ScopedHit {
                id: 9,
                code: None,
                title: None,
                body: None,
                source_table: "t_b",
                level: None,
                rank: 0.7,
            },
            ScopedHit {
                id: 3,
                code: None,
                title: None,
                body: None,
                source_table: "t_a",
                level: None,
                rank: 1.0,
            },
            ScopedHit {
                id: 2,
                code: None,
                title: None,
                body: None,
                source_table: "t_a",
                level: None,
                rank: 0.7,
            },
            ScopedHit {
                id: 1,
                code: None,
                title: None,
                body: None,
                source_table: "t_b",
                level: None,
                rank: 0.7,
            },
        ];
        sort_results(&mut hits);
        let order: Vec<(i64, &str, f64)> = hits
            .iter()
            .map(|h| (h.id, h.source_table, h.rank))
            .collect();
        assert_eq!(
            order,
            vec![
                (3, "t_a", 1.0),
                (2, "t_a", 0.7),
                (1, "t_b", 0.7),
                (9, "t_b", 0.7),
            ]
        );
    }

    #[test]
    fn match_pattern_escapes_metacharacters_and_strips_regex_wrapper() {
        let plain = MatchPattern::new("order");
        assert_eq!(plain.contains, "%order%");
        assert_eq!(plain.exact, "order");
        assert_eq!(plain.prefix, "order%");

        let wrapped = MatchPattern::new("/order/");
        assert_eq!(wrapped.contains, "%order%");
        assert_eq!(wrapped.exact, "order");

        // LIKE 元字符按字面量（escape_like 单源）
        let meta = MatchPattern::new("100%_x\\");
        assert_eq!(meta.contains, "%100\\%\\_x\\\\%");
        assert_eq!(meta.exact, "100%_x\\");
        assert_eq!(meta.prefix, "100\\%\\_x\\\\%");
    }

    #[test]
    fn tokenize_splits_separators_and_camel_case() {
        assert_eq!(
            tokenize("salesOrder_detail-2"),
            vec!["sales", "order", "detail", "2"]
        );
        assert_eq!(tokenize("   "), Vec::<String>::new());
        assert_eq!(tokenize("订单明细"), vec!["订单明细"]);
    }

    #[test]
    fn trigram_similarity_bounds() {
        assert!((trigram_similarity("purchase_order", "purchase_order") - 1.0).abs() < 1e-9);
        assert!(trigram_similarity("purchase_order", "invoice_header") < 0.2);
        assert!(
            trigram_similarity("custmer", "customer") > trigram_similarity("custmer", "invoice")
        );
    }

    #[test]
    fn fuzzy_score_ranks_near_miss_above_unrelated() {
        let near = fuzzy_score("purchse_orde", "purchase_order");
        let far = fuzzy_score("purchse_orde", "invoice_header");
        assert!(near >= 0.3, "near miss scored {near}");
        assert!(near > far, "near {near} should beat far {far}");
        assert!((fuzzy_score("purchase_order", "purchase_order") - 1.0).abs() < 1e-9);
        assert!((fuzzy_score("order", "purchase_order") - 0.95).abs() < 1e-9);
    }
}
