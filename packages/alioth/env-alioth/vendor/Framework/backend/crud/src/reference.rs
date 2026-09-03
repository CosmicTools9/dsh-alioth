//! Reference Resolver — 自动关联引用字段的 JOIN/子查询框架
//!
//! 根据实体声明的引用关系（`HasReferenceJoins` trait）生成 SQL 片段，
//! 将 FK 值解析为目标表 `notice` 等显示字段，聚合为 JSONB 嵌入查询结果。
//!
//! # 支持的 7 种关联形式
//!
//! | interface | type            | 基数   | SQL 模式                                                |
//! |-----------|-----------------|--------|--------------------------------------------------------|
//! | m2o       | belongsTo       | ToOne  | 标量子查询 `(SELECT ... FROM target WHERE id = s.fk)`  |
//! | obo       | belongsTo (1:1) | ToOne  | 同上                                                   |
//! | oho       | hasOne          | ToOne  | 标量子查询 `(SELECT ... FROM target WHERE fk = s.id)`  |
//! | o2m       | hasMany         | ToMany | 聚合子查询 `(SELECT jsonb_agg(...) FROM target WHERE)` |
//! | m2m       | belongsToMany   | ToMany | 穿透 junction 表后聚合                                   |
//! | mbm       | belongsToArray  | ToMany | 数组 FK: `ANY(s.array_fk)`                              |
//! | order-m2m | belongsToMany   | ToOne  | 穿透 junction 表 + DESC/NULLS LAST 排序                 |
//!
//! # 设计原则
//!
//! - 所有关联通过**标量/聚合子查询**实现，不产生行倍增 → pagination 不受影响
//! - 结果聚合到单个 `_refs` JSONB 列，实体只需一个 `#[sqlx(default)]` 字段
//! - Forward 关系也使用子查询（而非 LEFT JOIN），保持统一代码路径

use crate::entity::{AliothDbEntity, SensitiveColumn};

// ═══════════════════════════════════════════════════════════════════════════════
// 类型定义
// ═══════════════════════════════════════════════════════════════════════════════

/// 关联基数——决定 JSON 输出类型和 SQL 聚合方式
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Card {
    /// 0..1 个目标 → 标量子查询，输出单个 JSON 对象或 null
    ToOne,
    /// 0..N 个目标 → 聚合子查询，输出 JSON 数组
    ToMany,
}

/// Junction 表字段映射：源列名 → JSON 输出 key
///
/// 用于 `OrderedJunction` 的 `junction_display_fields`。Render 输出
/// `'<alias>', jtN.<column>` 格式，将 junction 表字段嵌入 _refs JSON。
#[derive(Debug, Clone, Copy)]
pub struct JunctionField {
    /// Junction 表列名（如 `"default_info"`），renderer 自动加双引号
    pub column: &'static str,
    /// JSON 输出 key（如 `"is_default"`），需为合法 JSON key
    pub alias: &'static str,
}

/// SQL 关联模式——决定子查询的 WHERE 条件
#[derive(Debug, Clone)]
pub enum JoinKind {
    /// m2o / obo: source.fk → target.id
    Forward {
        local_fk: &'static str,
        target_key: &'static str,
    },
    /// oho / o2m: target.fk → source.id
    Reverse {
        target_fk: &'static str,
        source_key: &'static str,
    },
    /// m2m: source → junction → target
    Junction {
        junction_table: &'static str,
        source_fk: &'static str,
        target_fk: &'static str,
        /// 关联表排序字段（如 `sequence`），生成 `jsonb_agg(... ORDER BY jt.field)`
        order_by: Option<&'static str>,
    },
    /// m2m with sort direction: source → junction → target
    ///
    /// 与 Junction 相同但支持 DESC/NULLS LAST 排序。
    /// 用于 `default_info` 等布尔排序字段需要 `DESC NULLS LAST` 语义的场景。
    OrderedJunction {
        junction_table: &'static str,
        source_fk: &'static str,
        target_fk: &'static str,
        /// 关联表排序字段（如 `"default_info"`）
        order_by: Option<&'static str>,
        /// 降序排序（true 时追加 DESC）
        order_desc: bool,
        /// NULL 置后（true 时追加 NULLS LAST）
        nulls_last: bool,
        /// 关联表额外暴露字段（如 `[JunctionField { column: "default_info", alias: "is_default" }]`）
        junction_display_fields: &'static [JunctionField],
    },
    /// mbm: ANY(source.array_fk) = target.id
    ArrayFk {
        array_fk: &'static str,
        target_key: &'static str,
    },
}

/// 单个关联字段的解析定义
///
/// 模块在实体 struct 上实现 `HasReferenceJoins` 时返回 `Vec<ReferenceJoin>`。
#[derive(Debug, Clone)]
pub struct ReferenceJoin {
    /// `_refs` JSON 中的 key（前端按此取用）
    pub name: &'static str,
    /// 关联基数
    pub card: Card,
    /// SQL 模式
    pub kind: JoinKind,
    /// 目标表完整名称（含 schema），如 `r#"isahl."zc_id_unit-currency""#`
    pub target_table: &'static str,
    /// 目标表显示字段，至少包含 `"notice"`，可扩展如 `["notice", "code"]`
    pub display_fields: &'static [&'static str],
}

/// 实体关联引用声明 trait
///
/// 实现此 trait 的实体会在 `QueryBuilder::with_references()` 中自动
/// 生成关联解析 SQL。
pub trait HasReferenceJoins: AliothDbEntity {
    fn reference_joins() -> Vec<ReferenceJoin>;
}

// ═══════════════════════════════════════════════════════════════════════════════
// 渲染输出
// ═══════════════════════════════════════════════════════════════════════════════

/// `ReferenceJoin` 渲染后的 SQL 片段
pub struct RenderedRef {
    /// 追加到 SELECT 的 JSONB 聚合表达式
    pub select_suffix: String,
}

/// 对 SQL 标识符加双引号，避免连字符/保留字导致语法错误
fn quote_ident(s: &str) -> String {
    format!(r#""{}""#, s)
}

impl ReferenceJoin {
    /// 渲染为 SQL 片段
    ///
    /// 返回的形式：
    /// ```sql
    /// 'name', (SELECT jsonb_build_object('notice', t0.notice) FROM target t0 WHERE t0.id = source.fk)
    /// ```
    /// 或 ToMany 用 `jsonb_agg` 替代 `jsonb_build_object`。
    pub fn render(&self, idx: usize) -> RenderedRef {
        let alias = format!("t{}", idx);
        let fields = self
            .display_fields
            .iter()
            .map(|f| format!("'{}', {}.{}", f, alias, quote_ident(f)))
            .collect::<Vec<_>>()
            .join(", ");

        let subquery = match (&self.kind, self.card) {
            // ── ToOne: 标量子查询 → jsonb_build_object ──
            (
                JoinKind::Forward {
                    local_fk,
                    target_key,
                },
                Card::ToOne,
            ) => format!(
                "(SELECT jsonb_build_object({}) FROM {} {} WHERE {}.{} = e.{})",
                fields,
                self.target_table,
                alias,
                alias,
                quote_ident(target_key),
                quote_ident(local_fk)
            ),
            (
                JoinKind::Reverse {
                    target_fk,
                    source_key,
                },
                Card::ToOne,
            ) => format!(
                "(SELECT jsonb_build_object({}) FROM {} {} WHERE {}.{} = e.{})",
                fields,
                self.target_table,
                alias,
                alias,
                quote_ident(target_fk),
                quote_ident(source_key)
            ),

            // ── ToMany: 聚合子查询 → jsonb_agg ──
            (
                JoinKind::Reverse {
                    target_fk,
                    source_key,
                },
                Card::ToMany,
            ) => format!(
                "(SELECT jsonb_agg(jsonb_build_object({})) FROM {} {} WHERE {}.{} = e.{})",
                fields,
                self.target_table,
                alias,
                alias,
                quote_ident(target_fk),
                quote_ident(source_key)
            ),
            (
                JoinKind::Junction {
                    junction_table,
                    source_fk,
                    target_fk,
                    order_by,
                },
                Card::ToOne,
            ) => {
                let jt = format!("jt{}", idx);
                let order_clause = order_by
                    .map(|ob| format!(" ORDER BY {}.{}", jt, quote_ident(ob)))
                    .unwrap_or_default();
                format!(
                    "(SELECT jsonb_build_object({}) FROM {} AS {} \
                     JOIN {} AS {} ON {}.{} = {}.{} \
                     WHERE {}.{} = e.{}{}{})",
                    fields,
                    self.target_table,
                    alias,
                    junction_table,
                    jt,
                    alias,
                    quote_ident("id"),
                    jt,
                    quote_ident(target_fk),
                    jt,
                    quote_ident(source_fk),
                    quote_ident("id"),
                    order_clause,
                    if order_by.is_some() { " LIMIT 1" } else { "" },
                )
            }
            (
                JoinKind::Junction {
                    junction_table,
                    source_fk,
                    target_fk,
                    order_by,
                },
                Card::ToMany,
            ) => {
                let jt = format!("jt{}", idx);
                let order_clause = order_by
                    .map(|ob| format!(" ORDER BY {}.{}", jt, quote_ident(ob)))
                    .unwrap_or_default();
                format!(
                    "(SELECT jsonb_agg(jsonb_build_object({}){}) FROM {} AS {} \
                     JOIN {} AS {} ON {}.{} = {}.{} \
                     WHERE {}.{} = e.{})",
                    fields,
                    order_clause,
                    self.target_table,
                    alias,
                    junction_table,
                    jt,
                    alias,
                    quote_ident("id"),
                    jt,
                    quote_ident(target_fk),
                    jt,
                    quote_ident(source_fk),
                    quote_ident("id"),
                )
            }
            (
                JoinKind::ArrayFk {
                    array_fk,
                    target_key,
                },
                Card::ToMany,
            ) => format!(
                // array_position 保序：数组外键语义=有序列表（路线途径点方向依赖顺序）
                r#"(SELECT jsonb_agg(jsonb_build_object({})) FROM (
                   SELECT {alias}.*, array_position(e.{}, {alias}.{}) AS __ord FROM {} {alias} WHERE {alias}.{} = ANY(e.{}) ORDER BY __ord
                 ) {alias})"#,
                fields,
                quote_ident(array_fk),
                quote_ident(target_key),
                self.target_table,
                quote_ident(target_key),
                quote_ident(array_fk),
            ),
            (
                JoinKind::OrderedJunction {
                    junction_table,
                    source_fk,
                    target_fk,
                    order_by,
                    order_desc,
                    nulls_last,
                    junction_display_fields,
                },
                Card::ToOne,
            ) => {
                let jt = format!("jt{}", idx);
                let jt_fragments = junction_display_fields
                    .iter()
                    .map(|f| format!("'{}', {}.{}", f.alias, jt, quote_ident(f.column)))
                    .collect::<Vec<_>>();
                let all_fields = if jt_fragments.is_empty() {
                    fields.clone()
                } else {
                    format!("{}, {}", fields, jt_fragments.join(", "))
                };
                let order_clause = order_by
                    .map(|col| {
                        let mut clause = format!("{}.{}", jt, quote_ident(col));
                        if *order_desc {
                            clause.push_str(" DESC");
                        }
                        if *nulls_last {
                            clause.push_str(" NULLS LAST");
                        }
                        format!(" ORDER BY {}", clause)
                    })
                    .unwrap_or_default();
                format!(
                    "(SELECT jsonb_build_object({}) FROM {} AS {} \
                     JOIN {} AS {} ON {}.{} = {}.{} \
                     WHERE {}.{} = e.{}{} LIMIT 1)",
                    all_fields,
                    self.target_table,
                    alias,
                    junction_table,
                    jt,
                    alias,
                    quote_ident("id"),
                    jt,
                    quote_ident(target_fk),
                    jt,
                    quote_ident(source_fk),
                    quote_ident("id"),
                    order_clause,
                )
            }
            (
                JoinKind::OrderedJunction {
                    junction_table,
                    source_fk,
                    target_fk,
                    order_by,
                    order_desc,
                    nulls_last,
                    junction_display_fields,
                },
                Card::ToMany,
            ) => {
                let jt = format!("jt{}", idx);
                let target_fields = self
                    .display_fields
                    .iter()
                    .map(|f| format!("'{}', {}.{}", f, alias, quote_ident(f)))
                    .collect::<Vec<_>>()
                    .join(", ");
                let jt_fragments = junction_display_fields
                    .iter()
                    .map(|f| format!("'{}', {}.{}", f.alias, jt, quote_ident(f.column)))
                    .collect::<Vec<_>>();
                let combined = if jt_fragments.is_empty() {
                    target_fields
                } else {
                    format!("{}, {}", target_fields, jt_fragments.join(", "))
                };
                let order_clause = order_by
                    .map(|col| {
                        let mut clause = format!("{}.{}", jt, quote_ident(col));
                        if *order_desc {
                            clause.push_str(" DESC");
                        }
                        if *nulls_last {
                            clause.push_str(" NULLS LAST");
                        }
                        format!(" ORDER BY {}", clause)
                    })
                    .unwrap_or_default();
                format!(
                    "COALESCE((SELECT jsonb_agg(jsonb_build_object({}){}) FROM {} AS {} \
                     JOIN {} AS {} ON {}.{} = {}.{} \
                     WHERE {}.{} = e.{}), '[]'::jsonb)",
                    combined,
                    order_clause,
                    self.target_table,
                    alias,
                    junction_table,
                    jt,
                    alias,
                    quote_ident("id"),
                    jt,
                    quote_ident(target_fk),
                    jt,
                    quote_ident(source_fk),
                    quote_ident("id"),
                )
            }
            _ => unreachable!(
                "ReferenceJoin: invalid combintation kind={:?} card={:?}",
                self.kind, self.card
            ),
        };

        RenderedRef {
            select_suffix: format!("'{}', {}", self.name, subquery),
        }
    }
}

/// 为实现了 `HasReferenceJoins` 的实体生成 `_refs` SELECT 后缀。
///
/// 返回形如：
/// ```sql
/// , jsonb_strip_nulls(jsonb_build_object('left', (subq), 'right', (subq))) AS _refs
/// ```
pub fn build_refs_select_suffix<E: HasReferenceJoins>() -> String {
    let joins = E::reference_joins();
    if joins.is_empty() {
        // 无引用声明的实体也输出空 `_refs` JSONB 列，保证实体 `_refs` 字段可解码、
        // /refs 路由行为与有引用实体一致。
        return ", '{}'::jsonb AS _refs".to_string();
    }
    let pairs: Vec<String> = joins
        .iter()
        .enumerate()
        .map(|(i, rj)| rj.render(i).select_suffix)
        .collect();
    format!(
        ", jsonb_strip_nulls(jsonb_build_object({})) AS _refs",
        pairs.join(", ")
    )
}

/// 列级安全投影后缀（NGAC 授权列裁剪，B 方案）
///
/// 实体声明 `SENSITIVE_COLUMNS` 后，SELECT 恒带 `_sensitive` jsonb 列：
/// - `SENSITIVE_COLUMNS` 空 → 返回空串（零回归，历史 SELECT 形态不变）
/// - 授权 `Some(cols)`：仅授权列（DTO 名）进入 jsonb_build_object；与 SENSITIVE_COLUMNS 求交
/// - 授权 `Some([])`（无任何列）或授权未覆盖 → `'{}'::jsonb`（fail-closed）
/// - 授权 `None`（未启用列控/无 PDP）→ 全量 SENSITIVE_COLUMNS（与历史行为一致）
///
/// 值形态（对齐 DTO 标量契约，见 common::scalar）：
/// - `scalar_table` 非空（如 `zc_id_scal-price`）→ `(SELECT jsonb_build_object('value', p.mark::text::numeric, 'unit', u.notice)
///   FROM <scalar_table> p LEFT JOIN zc_id_unit u ON u.id = p.sk_unit WHERE p.id = e.<column>)`
/// - `scalar_table` 为 None → 物理列裸值
///
/// `column_prefix`：SELECT 查询传 `"e."`（配合 `FROM ... AS e`）；
/// INSERT/UPDATE 的 RETURNING 无表别名，传 `""`。
pub fn build_sensitive_suffix<E: AliothDbEntity>(
    authorized: Option<&[String]>,
    column_prefix: &str,
) -> String {
    let sensitive = E::SENSITIVE_COLUMNS;
    if sensitive.is_empty() {
        return String::new();
    }
    // 授权为空数组（明确无列授权）或缺失（无 PDP 介入）→ fail-closed 空对象
    let cols: Vec<&SensitiveColumn> = match authorized {
        Some([]) => Vec::new(),
        Some(acl) => sensitive
            .iter()
            .filter(|sc| acl.iter().any(|a| a == sc.dto || a == "*"))
            .collect(),
        None => Vec::new(), // fail-closed：无 X-Authorized-Columns header → 敏感列全裁
    };
    if cols.is_empty() {
        return ", '{}'::jsonb AS _sensitive".to_string();
    }
    let pairs: Vec<String> = cols
        .iter()
        .map(|sc| {
            let key = format!("'{}'", sc.dto);
            match sc.scalar_table {
                Some(table) => format!(
                    "{}, (SELECT jsonb_build_object('value', p.mark::text::numeric, 'unit', u.notice) \
                     FROM isahl.\"{}\" p LEFT JOIN isahl.zc_id_unit u ON u.id = p.sk_unit \
                     WHERE p.id = {}{})",
                    key, table, column_prefix, sc.column
                ),
                None => format!("{}, {}{}", key, column_prefix, sc.column),
            }
        })
        .collect();
    format!(
        ", jsonb_strip_nulls(jsonb_build_object({})) AS _sensitive",
        pairs.join(", ")
    )
}

#[cfg(test)]
mod sensitive_suffix_tests {
    use super::*;
    use crate::entity::{AliothDbEntity, Identifiable, SensitiveColumn};
    use serde::Serialize;

    #[derive(Serialize, sqlx::FromRow)]
    struct FakeSensitiveEntity {
        id: i64,
        qk_price: Option<i64>,
        comments: Option<String>,
    }
    impl Identifiable for FakeSensitiveEntity {
        fn id(&self) -> i64 {
            0
        }
    }
    impl AliothDbEntity for FakeSensitiveEntity {
        fn table_name() -> &'static str {
            "isahl.fake"
        }
        const SELECT_FIELDS: &'static str = "id, qk_price, comments";
        const ENTITY_NAME: &'static str = "fake";
        const SENSITIVE_COLUMNS: &'static [SensitiveColumn] = &[
            SensitiveColumn {
                dto: "price",
                column: "qk_price",
                scalar_table: Some("zc_id_scal-price"),
            },
            SensitiveColumn {
                dto: "notes",
                column: "comments",
                scalar_table: None,
            },
        ];
    }
    #[derive(Serialize, sqlx::FromRow)]
    struct FakePlainEntity {
        id: i64,
    }
    impl Identifiable for FakePlainEntity {
        fn id(&self) -> i64 {
            0
        }
    }
    impl AliothDbEntity for FakePlainEntity {
        fn table_name() -> &'static str {
            "isahl.fake2"
        }
        const SELECT_FIELDS: &'static str = "id";
        const ENTITY_NAME: &'static str = "fake2";
    }

    #[test]
    fn plain_entity_returns_empty_suffix() {
        // SENSITIVE_COLUMNS 空 → 零回归（无 _sensitive 列）
        assert_eq!(build_sensitive_suffix::<FakePlainEntity>(None, "e."), "");
        assert_eq!(
            build_sensitive_suffix::<FakePlainEntity>(Some(&["price".to_string()]), "e."),
            ""
        );
    }

    #[test]
    fn no_authorization_returns_empty_object() {
        // 明确空授权（fail-closed）→ '{}'::jsonb
        let suffix = build_sensitive_suffix::<FakeSensitiveEntity>(Some(&[]), "e.");
        assert_eq!(suffix, ", '{}'::jsonb AS _sensitive");
    }

    #[test]
    fn no_pdp_returns_empty_object() {
        // None（无 X-Authorized-Columns header）→ fail-closed 空对象（敏感列全裁，
        // 与显式空授权同语义——列控安全加固，fix-pep-rls-column-fail-open）
        let suffix = build_sensitive_suffix::<FakeSensitiveEntity>(None, "e.");
        assert_eq!(suffix, ", '{}'::jsonb AS _sensitive");
        assert!(!suffix.contains("'price'"), "无授权时 price 不应出现");
        assert!(!suffix.contains("'notes'"), "无授权时 notes 不应出现");
    }

    #[test]
    fn partial_authorization_filters_columns() {
        // 仅授权 DTO 名 price → 只含该列（标量解析）
        let suffix =
            build_sensitive_suffix::<FakeSensitiveEntity>(Some(&["price".to_string()]), "e.");
        assert!(suffix.contains("'price'"), "price 应保留");
        assert!(!suffix.contains("'notes'"), "notes 应被裁剪");
        assert!(!suffix.contains("e.comments"), "comments 物理列不应出现");
    }

    #[test]
    fn wildcard_authorization_returns_all() {
        // read:* 通配 → 全量
        let suffix = build_sensitive_suffix::<FakeSensitiveEntity>(Some(&["*".to_string()]), "e.");
        assert!(suffix.contains("'price'") && suffix.contains("'notes'"));
    }

    #[test]
    fn scalar_value_shape_matches_dto() {
        // 标量解析产物是 { value: numeric, unit: text }——对齐 DTO ScalarPriceValue 语义
        let suffix =
            build_sensitive_suffix::<FakeSensitiveEntity>(Some(&["price".to_string()]), "e.");
        assert!(
            suffix.contains("jsonb_build_object('value', p.mark::text::numeric, 'unit', u.notice)")
        );
    }
}
