//! 知识域表登记册 —— `isahl.zc_id_law-*` / `zc_id_stan-*` 全族唯一权威声明
//!
//! 唯一事实源（本文件）：知识域物理表清单 + 列位形态 + 体系/叶关系 + 入库 scope +
//! **通用 CRUD 静态 SQL**（`concat!` 编译期固化，运行期无拼串、无 `AssertSqlSafe`）。
//!
//! 铁律：
//! 1. 知识域物理表名只在本文件声明；消费方（Meta 知识端点 / namespace 知识 Service /
//!    Gateway 检索端点 / 知识图谱投影）经 [`by_code`] / [`all`] / 域访问器取事实，
//!    MUST NOT 在本地维护第二份清单（`scripts/check/check-knowledge-domains.ts` 强制）。
//! 2. 列位形态由**形态关键字**表达（`law` / `stan_leaf` / `stan_body`）——由模型实测得出，
//!    运行期 MUST NOT 用 `information_schema` 探测列位；
//!    形态 ↔ 模型一致性由 `Meta/backend/bin/tests/knowledge_domains_catalog.rs` 守住。
//! 3. 新增知识域表 = 在 [`knowledge_domains!`] 调用中加一行；消费方代码无需改动。
//!
//! 形态 ↔ 列位（DB 实测，`meta_fields` 为模型真相源）：
//!
//! | 形态 | `fk_parent` | `fk_previous` | `fk_jurisdiction` | `_t_` | `qk_effective` |
//! |---|---|---|---|---|---|
//! | `law`（法域全族） | ✓ | ✓ | ✓ | ✓ | ✓ |
//! | `stan_leaf`（标准体系行与条文叶） | ✓ | ✓ | ✗ | ✓ | ✗ |
//! | `stan_body`（标准基表 air/fin/operation/prod_quality） | ✗ | ✓ | ✗ | ✓ | ✗ |

use crate::AliothError;

/// 下钻语义（树形端点用）
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Drill {
    /// 不可下钻
    None,
    /// 结构下钻（下级 = 中间层表，如 法典 → 编/章）
    Structural,
    /// 叶下钻（下级 = `entry_table` 的 `fk_parent` 子行）
    Leaf,
}

/// 知识域表规格（全部字段为编译期常量）
#[derive(Debug, Clone, Copy)]
pub struct KnowledgeTable {
    /// 物理表名（= API `table_code`，二者同值）
    pub table: &'static str,
    /// 限定名 `isahl."<table>"`
    pub qualified: &'static str,
    /// 域分组（`legal-civil` / `legal-common` / `legal-intl` / `legal-mixed` / `legal-religious`
    /// / `aviation` / `finance` / `clause` / `operation` / `quality`）
    pub group: &'static str,
    /// Meta 入库 scope（`None` = 不可经入库端点写入，仅树/投影/只读语义使用）
    pub scope: Option<&'static str>,
    /// 该表下钻得到的条目叶表（`None` = 无叶语义）
    pub entry_table: Option<&'static str>,
    /// 下钻语义
    pub drill: Drill,
    /// 有 `fk_parent` 列（层级）
    pub has_parent: bool,
    /// 有 `fk_previous` 列（版本链）
    pub has_previous: bool,
    /// 有 `fk_jurisdiction` 列（法域）
    pub has_jurisdiction: bool,
    /// 有 `_t_` 列（层级类型）
    pub has_level: bool,
    /// 有 `qk_effective` 列（生效日，标量引用）
    pub has_effective: bool,
    /// `SELECT id … WHERE code = $1 AND deleted_at IS NULL LIMIT 1`
    pub sql_by_code: &'static str,
    /// `SELECT count(*) … WHERE deleted_at IS NULL`
    pub sql_count: &'static str,
    /// 计数（限 `fk_parent = $1`）
    pub sql_count_children: &'static str,
    /// `SELECT id, code, notice, comments … ORDER BY id DESC LIMIT $1 OFFSET $2`
    pub sql_list: &'static str,
    /// 列表（限 `fk_parent = $3`；分页占位符 $1/$2 在前）
    pub sql_list_children: &'static str,
    /// `SELECT id, code, notice … ORDER BY id`
    pub sql_rows: &'static str,
    /// 行集（限 `fk_parent = $1`）
    pub sql_rows_children: &'static str,
    /// `SELECT code, notice … ORDER BY id LIMIT 1`
    pub sql_head: &'static str,
    /// 幂等入库（列集按形态固化）
    pub sql_insert: &'static str,
    /// 软删（`deleted_at` 置位）
    pub sql_soft_delete: &'static str,
}

/// `none` → `None`；`"x"` → `Some("x")`（宏声明语法糖）
macro_rules! opt_literal {
    (none) => {
        None
    };
    ($x:literal) => {
        Some($x)
    };
}

macro_rules! drill_kind {
    (none) => {
        Drill::None
    };
    (structural) => {
        Drill::Structural
    };
    (leaf) => {
        Drill::Leaf
    };
}

macro_rules! has_parent {
    (law) => {
        true
    };
    (stan_leaf) => {
        true
    };
    (stan_body) => {
        false
    };
}

macro_rules! has_previous {
    (law) => {
        true
    };
    (stan_leaf) => {
        true
    };
    (stan_body) => {
        true
    };
}

macro_rules! has_jurisdiction {
    (law) => {
        true
    };
    ($_:ident) => {
        false
    };
}

macro_rules! has_level {
    (law) => {
        true
    };
    (stan_leaf) => {
        true
    };
    (stan_body) => {
        true
    };
}

macro_rules! has_effective {
    (law) => {
        true
    };
    ($_:ident) => {
        false
    };
}

macro_rules! sql_insert {
    (law $t:literal) => {
        concat!(
            "INSERT INTO isahl.\"",
            $t,
            "\" (notice, code, comments, fk_parent, fk_jurisdiction) VALUES ($1, $2, $3, $4, NULL)"
        )
    };
    (stan_leaf $t:literal) => {
        concat!(
            "INSERT INTO isahl.\"",
            $t,
            "\" (notice, code, comments, fk_parent, _t_) VALUES ($1, $2, $3, $4, $5)"
        )
    };
    (stan_body $t:literal) => {
        concat!(
            "INSERT INTO isahl.\"",
            $t,
            "\" (notice, code, comments, _t_) VALUES ($1, $2, $3, $4)"
        )
    };
}

/// 单表规格展开（形态关键字 → 列位 flag + 静态 SQL）
macro_rules! kt {
    ($shape:ident $t:literal, $group:literal, $scope:tt, $entry:tt, $drill:tt) => {
        KnowledgeTable {
            table: $t,
            qualified: concat!("isahl.\"", $t, "\""),
            group: $group,
            scope: opt_literal!($scope),
            entry_table: opt_literal!($entry),
            drill: drill_kind!($drill),
            has_parent: has_parent!($shape),
            has_previous: has_previous!($shape),
            has_jurisdiction: has_jurisdiction!($shape),
            has_level: has_level!($shape),
            has_effective: has_effective!($shape),
            sql_by_code: concat!(
                "SELECT id FROM isahl.\"",
                $t,
                "\" WHERE code = $1 AND deleted_at IS NULL LIMIT 1"
            ),
            sql_count: concat!(
                "SELECT count(*) FROM isahl.\"",
                $t,
                "\" WHERE deleted_at IS NULL"
            ),
            sql_count_children: concat!(
                "SELECT count(*) FROM isahl.\"",
                $t,
                "\" WHERE deleted_at IS NULL AND fk_parent = $1"
            ),
            sql_list: concat!(
                "SELECT id, code, notice, comments FROM isahl.\"",
                $t,
                "\" WHERE deleted_at IS NULL ORDER BY id DESC LIMIT $1 OFFSET $2"
            ),
            sql_list_children: concat!(
                "SELECT id, code, notice, comments FROM isahl.\"",
                $t,
                "\" WHERE deleted_at IS NULL AND fk_parent = $3 ORDER BY id DESC LIMIT $1 OFFSET $2"
            ),
            sql_rows: concat!(
                "SELECT id, code, notice FROM isahl.\"",
                $t,
                "\" WHERE deleted_at IS NULL ORDER BY id"
            ),
            sql_rows_children: concat!(
                "SELECT id, code, notice FROM isahl.\"",
                $t,
                "\" WHERE deleted_at IS NULL AND fk_parent = $1 ORDER BY id"
            ),
            sql_head: concat!(
                "SELECT code, notice FROM isahl.\"",
                $t,
                "\" WHERE deleted_at IS NULL ORDER BY id LIMIT 1"
            ),
            sql_insert: sql_insert!($shape $t),
            sql_soft_delete: concat!(
                "UPDATE isahl.\"",
                $t,
                "\" SET deleted_at = NOW() WHERE id = $1 AND deleted_at IS NULL"
            ),
        }
    };
}

/// 知识域表登记册 —— 唯一权威声明（每行一表；表名字面量全仓仅此处出现）
///
/// 行格式：`<形态> <表名>, <域分组>, <入库 scope|none>, <叶表|none>, <下钻|none|structural|leaf>;`
macro_rules! knowledge_domains {
    ($( $shape:ident $t:literal, $group:literal, $scope:tt, $entry:tt, $drill:tt ; )*) => {
        /// 全族登记册（声明序 = 分组内稳定序；消费方按域取切片，MUST NOT 依赖跨域顺序）
        pub const KNOWLEDGE_TABLES: &[KnowledgeTable] = &[
            $( kt!($shape $t, $group, $scope, $entry, $drill), )*
        ];
    };
}

knowledge_domains![
    // ── 法域 · 民法典体系（law-civil）
    law "zc_id_law-civil", "legal-civil", none, none, none;
    law "zc_id_law-civil-code", "legal-civil", none, "zc_id_law-civil-article", structural;
    law "zc_id_law-civil-book", "legal-civil", none, "zc_id_law-civil-article", leaf;
    law "zc_id_law-civil-chapter", "legal-civil", none, none, none;
    law "zc_id_law-civil-section", "legal-civil", none, none, none;
    law "zc_id_law-civil-article", "legal-civil", "legal", none, none;
    // ── 法域 · 通用法规（law-common）
    law "zc_id_law-common", "legal-common", none, none, none;
    law "zc_id_law-common-statute", "legal-common", none, "zc_id_law-common-section", leaf;
    law "zc_id_law-common-title", "legal-common", none, none, none;
    law "zc_id_law-common-chapter", "legal-common", none, none, none;
    law "zc_id_law-common-section", "legal-common", "legal", none, none;
    law "zc_id_law-common-case", "legal-common", none, "zc_id_law-common-holding", leaf;
    law "zc_id_law-common-holding", "legal-common", "legal", none, none;
    // ── 法域 · 国际条约（law-intl）
    law "zc_id_law-intl", "legal-intl", none, none, none;
    law "zc_id_law-intl-treaty", "legal-intl", none, "zc_id_law-intl-article", leaf;
    law "zc_id_law-intl-part", "legal-intl", none, none, none;
    law "zc_id_law-intl-chapter", "legal-intl", none, none, none;
    law "zc_id_law-intl-custom", "legal-intl", none, none, none;
    law "zc_id_law-intl-article", "legal-intl", "legal", none, none;
    // ── 法域 · 其他法系
    law "zc_id_law-mixed", "legal-mixed", none, none, none;
    law "zc_id_law-religious", "legal-religious", none, none, none;
    // ── 航空标准（stan-air）
    stan_body "zc_id_stan-air", "aviation", none, none, none;
    stan_leaf "zc_id_stan-air-caac", "aviation", none, none, none;
    stan_leaf "zc_id_stan-air-caac-article", "aviation", "aviation", none, none;
    stan_leaf "zc_id_stan-air-easa", "aviation", none, none, none;
    stan_leaf "zc_id_stan-air-easa-article", "aviation", "aviation", none, none;
    stan_leaf "zc_id_stan-air-faa", "aviation", none, none, none;
    stan_leaf "zc_id_stan-air-faa-article", "aviation", "aviation", none, none;
    stan_leaf "zc_id_stan-air-icao", "aviation", none, none, none;
    stan_leaf "zc_id_stan-air-icao-article", "aviation", "aviation", none, none;
    // ── 标准条款（stan-clause）
    stan_leaf "zc_id_stan-clause", "clause", none, none, none;
    // ── 财务标准（stan-fin）
    stan_body "zc_id_stan-fin", "finance", none, none, none;
    stan_leaf "zc_id_stan-fin-cas", "finance", none, "zc_id_stan-fin-cas-article", leaf;
    stan_leaf "zc_id_stan-fin-cas-article", "finance", "finance", none, none;
    stan_leaf "zc_id_stan-fin-ifrs", "finance", none, "zc_id_stan-fin-ifrs-article", leaf;
    stan_leaf "zc_id_stan-fin-ifrs-article", "finance", "finance", none, none;
    stan_leaf "zc_id_stan-fin-gaap", "finance", none, "zc_id_stan-fin-gaap-article", leaf;
    stan_leaf "zc_id_stan-fin-gaap-article", "finance", "finance", none, none;
    // ── 运营与质量
    stan_body "zc_id_stan-operation", "operation", "operation", none, none;
    stan_body "zc_id_stan-prod_quality", "quality", "quality", none, none;
];

// ─────────────────────────────────────────────
// 法域结构查询（law-civil 层级：法典 code → 编 book → 条文 article；知识树端点消费）
// ─────────────────────────────────────────────

/// 税域法典根（`code = ANY($1)` 白名单命中；`fk_parent IS NULL`）
pub const SQL_CIVIL_CODE_TAX_ROOTS: &str =
    "SELECT id, code, notice FROM isahl.\"zc_id_law-civil-code\" \
     WHERE deleted_at IS NULL AND fk_parent IS NULL AND code = ANY($1) ORDER BY id";

/// 法域法典根（非税法典）
pub const SQL_CIVIL_CODE_NONTAX_ROOTS: &str =
    "SELECT id, code, notice FROM isahl.\"zc_id_law-civil-code\" \
     WHERE deleted_at IS NULL AND fk_parent IS NULL AND NOT (code = ANY($1)) ORDER BY id";

/// 法典根 → 编（`fk_parent = $1`）
pub const SQL_CIVIL_BOOKS_UNDER_ROOT: &str =
    "SELECT id, code, notice FROM isahl.\"zc_id_law-civil-book\" \
     WHERE deleted_at IS NULL AND fk_parent = $1 ORDER BY id";

/// 法典根子树条文数（编 → 条文两级）
pub const SQL_CIVIL_ARTICLE_COUNT_UNDER_ROOT: &str =
    "SELECT count(*) FROM isahl.\"zc_id_law-civil-article\" \
     WHERE deleted_at IS NULL AND fk_parent IN \
       (SELECT id FROM isahl.\"zc_id_law-civil-book\" WHERE deleted_at IS NULL AND fk_parent = $1)";

/// 财务域种子锚点（CAS 体系行；种子状态探针用）
pub fn finance_seed_anchor() -> Option<&'static KnowledgeTable> {
    by_code("zc_id_stan-fin-cas")
}

// ─────────────────────────────────────────────
// 查找 API（未知表 fail-visible，不回落、不静默）
// ─────────────────────────────────────────────

/// 按物理表名（= API `table_code`）取规格
pub fn by_code(table_code: &str) -> Option<&'static KnowledgeTable> {
    KNOWLEDGE_TABLES.iter().find(|t| t.table == table_code)
}

/// 全族
pub fn all() -> &'static [KnowledgeTable] {
    KNOWLEDGE_TABLES
}

/// 可按入库端点写入的表（`scope = Some`）
pub fn ingestable() -> impl Iterator<Item = &'static KnowledgeTable> {
    KNOWLEDGE_TABLES.iter().filter(|t| t.scope.is_some())
}

/// 指定域分组内**可下钻的体系行表**（声明序稳定；树形端点消费）
pub fn drillable_in_group(group: &'static str) -> impl Iterator<Item = &'static KnowledgeTable> {
    KNOWLEDGE_TABLES
        .iter()
        .filter(move |t| t.group == group && t.drill != Drill::None && t.entry_table.is_some())
}

/// 法域结构访问器（law-civil 层级）
pub fn civil_code() -> Option<&'static KnowledgeTable> {
    by_code("zc_id_law-civil-code")
}

/// 法域结构访问器（法典 → 编）
pub fn civil_book() -> Option<&'static KnowledgeTable> {
    by_code("zc_id_law-civil-book")
}

/// 法域结构访问器（编 → 条文叶）
pub fn civil_article() -> Option<&'static KnowledgeTable> {
    by_code("zc_id_law-civil-article")
}

/// 未知表错误（消费方统一话术）
pub fn unknown_table_err(table_code: &str) -> AliothError {
    AliothError::Internal(format!("未知知识域表: {table_code}（不在登记册）"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashSet;

    #[test]
    fn registry_self_consistent() {
        let mut tables = HashSet::new();
        let mut codes = HashSet::new();
        for t in all() {
            assert!(tables.insert(t.table), "表名重复: {}", t.table);
            assert!(codes.insert(t.table), "code 重复: {}", t.table);
            assert_eq!(t.qualified, format!("isahl.\"{}\"", t.table));
            assert!(t.table.starts_with("zc_id_law-") || t.table.starts_with("zc_id_stan-"));
            // 宏装配不变式：每条 SQL 都引用本表限定名（模板字段错配即失败）
            for sql in [
                t.sql_by_code,
                t.sql_count,
                t.sql_count_children,
                t.sql_list,
                t.sql_list_children,
                t.sql_rows,
                t.sql_rows_children,
                t.sql_head,
                t.sql_insert,
                t.sql_soft_delete,
            ] {
                assert!(
                    sql.contains(t.qualified),
                    "{} 的 SQL 未引用本表限定名 {}: {sql}",
                    t.table,
                    t.qualified
                );
            }
            assert!(
                t.sql_insert.starts_with("INSERT INTO isahl.\""),
                "{}",
                t.table
            );
            assert!(
                t.sql_soft_delete.contains("deleted_at = NOW()"),
                "{}",
                t.table
            );
        }
    }

    #[test]
    fn entry_relations_resolve() {
        for t in all() {
            if let Some(entry) = t.entry_table {
                let leaf = by_code(entry).unwrap_or_else(|| panic!("叶表未登记: {entry}"));
                assert!(
                    leaf.has_parent,
                    "叶表 {} 无 fk_parent，无法承载 fk_parent 下钻",
                    entry
                );
            }
            if t.drill == Drill::Structural {
                assert!(t.entry_table.is_some(), "结构下钻需声明叶表: {}", t.table);
            }
        }
    }

    #[test]
    fn ingest_scopes_match_contract() {
        let got: Vec<(&str, &str)> = ingestable()
            .map(|t| (t.scope.unwrap_or_default(), t.table))
            .collect();
        assert_eq!(got.len(), 13, "入库 scope 表数应为 13: {got:?}");
        for scope in ["legal", "aviation", "finance", "operation", "quality"] {
            assert!(got.iter().any(|(s, _)| *s == scope), "scope 缺失: {scope}");
        }
        assert_eq!(got.iter().filter(|(s, _)| *s == "legal").count(), 4);
        assert_eq!(got.iter().filter(|(s, _)| *s == "aviation").count(), 4);
        assert_eq!(got.iter().filter(|(s, _)| *s == "finance").count(), 3);
    }

    #[test]
    fn shape_flags_follow_family_rule() {
        for t in all() {
            if t.group.starts_with("legal-") {
                assert!(
                    t.has_parent
                        && t.has_previous
                        && t.has_jurisdiction
                        && t.has_level
                        && t.has_effective,
                    "法域全族应为五列齐备: {}",
                    t.table
                );
            } else {
                assert!(
                    !t.has_jurisdiction,
                    "标准族不得有 fk_jurisdiction: {}",
                    t.table
                );
                assert!(!t.has_effective, "标准族不得有 qk_effective: {}", t.table);
                assert!(
                    t.has_previous && t.has_level,
                    "标准族应有 fk_previous/_t_: {}",
                    t.table
                );
            }
        }
    }

    #[test]
    fn group_slices_cover_tree_needs() {
        let finance: Vec<&str> = drillable_in_group("finance").map(|t| t.table).collect();
        assert_eq!(
            finance,
            vec![
                "zc_id_stan-fin-cas",
                "zc_id_stan-fin-ifrs",
                "zc_id_stan-fin-gaap"
            ]
        );
        let common: Vec<&str> = drillable_in_group("legal-common")
            .map(|t| t.table)
            .collect();
        assert_eq!(
            common,
            vec!["zc_id_law-common-statute", "zc_id_law-common-case"]
        );
        let intl: Vec<&str> = drillable_in_group("legal-intl").map(|t| t.table).collect();
        assert_eq!(intl, vec!["zc_id_law-intl-treaty"]);
    }

    #[test]
    fn unknown_table_is_visible() {
        assert!(by_code("zc_id_law-nonexistent").is_none());
        assert!(unknown_table_err("x").to_string().contains("不在登记册"));
    }
}
