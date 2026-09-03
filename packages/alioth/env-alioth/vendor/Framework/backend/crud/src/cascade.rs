//! 级联软删除模块
//!
//! 主实体软删除（`soft_delete` / `batch_soft_delete`）时，按 `fk_index` 注册拓扑
//! （[`crate::fk_index::lookup_reverse_fk`]）推导级联目标，在同一数据库事务内批量
//! 标记关联数据的 `deleted_at`（REQ-NFR-005 验收(1)：「主需求软删除后，通过 JOIN
//! 可查询到的关联数据 deleted_at 均不为 NULL」）。
//!
//! # 级联分类与默认策略（保守默认）
//!
//! | 类别 | 判定 | 默认 |
//! | --- | --- | --- |
//! | 关系表 | 表名含 `_r_`/`_rr_`（桥接），或 FK 列为 `ref_left`/`ref_right` | 级联 |
//! | 子实体 | FK 列为 `fk_parent`/`fk_previous`（层级子行） | 级联（递归，含孙行） |
//! | 业务引用 | 其他跨实体 `fk_*`/`sk_*`/`qk_*` 等引用 | **不**级联，`CascadeConfig::business_refs` 显式开启 |
//!
//! 级联目标**必须**由 fk_index 注册推导（registry-driven），禁止手写映射表
//! （REUSE_FIRST / REFERENCE_RESOLVER 语义：与 `_refs` 解析同一编译期源）。
//!
//! # 配置承载
//!
//! 实体级可选配置，对应 service.json 实体声明中的 `softDeleteCascade`（缺失 →
//! 保守默认 [`CascadeConfig::default`]）：
//!
//! ```json
//! { "softDeleteCascade": { "relationTables": true, "childEntities": true, "businessRefs": false } }
//! ```
//!
//! 框架侧扩展点已就绪：`QueryBuilder::soft_delete_with_cascade` /
//! `batch_soft_delete_with_cascade` 接受 [`CascadeConfig`]（缺省走保守默认）。
//! service.json `softDeleteCascade` 键的读取与接线随 Service codegen 落地（后续增量）；
//! 当前调用路径（`soft_delete` / `batch_soft_delete` / `GenericRepository`）恒使用默认配置。
//!
//! # 边界（Non-goal）
//!
//! - 仅标记 `deleted_at`，不传播审计字段（`deleted_by_id`、`updated_at` 等）
//! - 未注册进 `fk_index` 的手写 SQL 桥接不参与级联（以注册拓扑为准）
//! - 数组/桥接 `_` 列（注册条目 local_key 为空或 `"id"`）无法形成标量 FK 谓词，跳过

use std::collections::VecDeque;

use sqlx::{AssertSqlSafe, Postgres, Row, Transaction};

use crate::fk_index::lookup_reverse_fk;

/// 递归级联深度上限（防自引用数据环导致无限递归；正常层级远低于此）。
pub const MAX_CASCADE_DEPTH: usize = 16;

/// 级联目标种类
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CascadeKind {
    /// 关系桥接表（`r_*`/`rr_*`，`ref_left`/`ref_right` 指向主实体）
    RelationTable,
    /// 子实体（`fk_parent`/`fk_previous` 指向主实体的行）
    ChildEntity,
    /// 业务跨实体引用
    BusinessRef,
}

/// 级联范围配置——对应 service.json 实体声明 `softDeleteCascade`。
///
/// 默认值保守：关系表 + 子实体级联，业务引用不级联（D1）。
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct CascadeConfig {
    /// 关系表（`r_*`/`rr_*` 桥接）随主删除级联
    pub relation_tables: bool,
    /// 子实体（`fk_parent`/`fk_previous`）随主删除级联（递归）
    pub child_entities: bool,
    /// 业务 `fk_*` 跨实体引用级联（显式开启，跨实体生命周期需调用方确认）
    pub business_refs: bool,
    /// 明细（detail 族 `zc_id_deta-*`）业务引用默认级联（2026-08-27 用户裁决：
    /// 「所有明细就是这么设计的」——明细随其所属单据/主实体软删级联）
    pub detail_business_refs: bool,
}

impl Default for CascadeConfig {
    fn default() -> Self {
        Self {
            relation_tables: true,
            child_entities: true,
            business_refs: false,
            detail_business_refs: true,
        }
    }
}

/// 明细（detail 族）表判定：zc_id_detail 全部 13 张子表均 `zc_id_deta-*` 前缀
/// （实测，2026-08-27），静态前缀匹配即可（级联推导为纯函数，无 DB 访问）。
pub fn is_detail_table(table: &str) -> bool {
    table.starts_with("zc_id_deta-")
}

/// 单个级联目标：目标表 + 指向主实体的 FK 列
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CascadeTarget {
    /// 目标表裸名（fk_index key，如 `zc_id_lifecycle_r_status`）
    pub table: String,
    /// 目标表中指向主实体的 FK 列（如 `ref_right`、`fk_previous`）
    pub fk_column: String,
    /// 级联种类
    pub kind: CascadeKind,
}

/// 从 `AliothDbEntity::table_name()` 提取裸表名（fk_index 的 key 形态：无 schema、无引号）。
///
/// 支持 `isahl.zc_id_production`、`"isahl"."zc_id_stor-plc-warehouse"`、
/// `zc_id_foo` 等既有实体声明形态。
pub fn bare_table_name(table_name: &str) -> &str {
    let name = table_name
        .strip_prefix(r#""isahl"."#)
        .or_else(|| table_name.strip_prefix("isahl."))
        .or_else(|| table_name.strip_prefix(r#""isahl_auth"."#))
        .or_else(|| table_name.strip_prefix("isahl_auth."))
        .unwrap_or(table_name);
    name.trim_matches('"')
}

/// 按 fk_index 注册拓扑推导级联目标（registry-driven，不手写映射表）。
///
/// - 关系表：表名含 `_r_`/`_rr_`，或 FK 列为 `ref_left`/`ref_right`
/// - 子实体：FK 列为 `fk_parent`/`fk_previous`
/// - 其余为业务引用；三者分别受 `CascadeConfig` 开关控制
pub fn derive_cascade_targets(entity_table: &str, config: &CascadeConfig) -> Vec<CascadeTarget> {
    let mut targets = Vec::new();
    for &(source_table, field_name, local_key) in lookup_reverse_fk(entity_table) {
        // 数组/桥接 `_` 列无法形成标量 FK 谓词，不参与级联（Non-goal 注明）
        if local_key.is_empty() || local_key == "id" {
            continue;
        }
        let kind = classify(source_table, field_name, local_key);
        let enabled = match kind {
            CascadeKind::RelationTable => config.relation_tables,
            CascadeKind::ChildEntity => config.child_entities,
            CascadeKind::BusinessRef => {
                if is_detail_table(source_table) {
                    config.detail_business_refs
                } else {
                    config.business_refs
                }
            }
        };
        if enabled {
            targets.push(CascadeTarget {
                table: source_table.to_string(),
                fk_column: local_key.to_string(),
                kind,
            });
        }
    }
    targets
}

fn classify(source_table: &str, field_name: &str, local_key: &str) -> CascadeKind {
    if is_relation_table(source_table, local_key) {
        CascadeKind::RelationTable
    } else if is_child_reference(field_name, local_key) {
        CascadeKind::ChildEntity
    } else {
        CascadeKind::BusinessRef
    }
}

fn is_relation_table(source_table: &str, local_key: &str) -> bool {
    (source_table.contains("_r_") || source_table.contains("_rr_"))
        || matches!(local_key, "ref_left" | "ref_right")
}

fn is_child_reference(field_name: &str, local_key: &str) -> bool {
    matches!(local_key, "fk_parent" | "fk_previous")
        || matches!(field_name, "fk_parent" | "fk_previous")
}

fn quote_ident(ident: &str) -> String {
    format!(r#""{}""#, ident)
}

fn placeholders(count: usize) -> String {
    (1..=count)
        .map(|i| format!("${}", i))
        .collect::<Vec<_>>()
        .join(", ")
}

/// 在既有事务内执行级联软删除。
///
/// - 关系表/业务引用目标：`UPDATE ... SET deleted_at = NOW() WHERE <fk> IN (ids) AND deleted_at IS NULL`
/// - 子实体目标：`UPDATE ... RETURNING id` 取得已标记子行，将 (子表, 子行 id)
///   压入工作队列继续下探孙行（自引用层级（`fk_previous` 链）逐层展开；
///   `deleted_at IS NULL` 谓词同时天然阻断数据环——回指已删行不会再被选中）
///
/// 任一目标 UPDATE 失败 → 由调用方事务整体回滚（REQ-NFR-005 级联原子性）。
pub async fn cascade_soft_delete(
    tx: &mut Transaction<'_, Postgres>,
    entity_table: &str,
    ids: &[i64],
    config: &CascadeConfig,
) -> Result<u64, sqlx::Error> {
    if ids.is_empty() {
        return Ok(0);
    }

    let mut affected = 0u64;
    // 工作队列：(目标表, 该层 id 集合, 深度)。子实体发现的新行追加到队尾逐层下探。
    let mut queue: VecDeque<(String, Vec<i64>, usize)> = VecDeque::new();
    queue.push_back((entity_table.to_string(), ids.to_vec(), 0));

    while let Some((table, level_ids, depth)) = queue.pop_front() {
        if depth >= MAX_CASCADE_DEPTH {
            continue;
        }
        // schema-drift 守卫：仅级联「表与列在部署库中真实存在」的目标
        // （fk_index 注册可能领先于部署 schema；缺失目标跳过并记入审计观察项）
        let targets = existing_targets(tx, &derive_cascade_targets(&table, config)).await?;
        for target in targets {
            match target.kind {
                CascadeKind::RelationTable | CascadeKind::BusinessRef => {
                    affected += update_by_fk(tx, &target, &level_ids).await?;
                }
                CascadeKind::ChildEntity => {
                    // 标记子行并取回子行 id，用于下探孙行
                    let child_ids = update_by_fk_returning(tx, &target, &level_ids).await?;
                    if !child_ids.is_empty() {
                        affected += child_ids.len() as u64;
                        queue.push_back((target.table.clone(), child_ids, depth + 1));
                    }
                }
            }
        }
    }
    Ok(affected)
}

/// 过滤出部署库中真实存在的级联目标（表存在且含 FK 列与 `deleted_at` 列）。
///
/// fk_index 由生成脚本产出，可能领先于某 namespace 的部署 schema；缺失目标
/// 直接跳过（保留主删除可用性），drift 作为部署观察项记录。
async fn existing_targets(
    tx: &mut Transaction<'_, Postgres>,
    targets: &[CascadeTarget],
) -> Result<Vec<CascadeTarget>, sqlx::Error> {
    if targets.is_empty() {
        return Ok(Vec::new());
    }
    let tables: Vec<&str> = targets.iter().map(|t| t.table.as_str()).collect();
    let rows = sqlx::query(
        "SELECT table_name, column_name FROM information_schema.columns \
         WHERE table_schema = 'isahl' AND table_name = ANY($1)",
    )
    .bind(&tables)
    .fetch_all(&mut **tx)
    .await?;

    let mut cols: std::collections::HashMap<String, std::collections::HashSet<String>> =
        std::collections::HashMap::new();
    for row in &rows {
        cols.entry(row.get::<String, _>("table_name"))
            .or_default()
            .insert(row.get::<String, _>("column_name"));
    }
    Ok(targets
        .iter()
        .filter(|t| {
            cols.get(&t.table)
                .map(|set| set.contains(t.fk_column.as_str()) && set.contains("deleted_at"))
                .unwrap_or(false)
        })
        .cloned()
        .collect())
}

/// 按 FK 列匹配批量标记 `deleted_at`，返回影响行数。
async fn update_by_fk(
    tx: &mut Transaction<'_, Postgres>,
    target: &CascadeTarget,
    ids: &[i64],
) -> Result<u64, sqlx::Error> {
    if ids.is_empty() {
        return Ok(0);
    }
    let sql = format!(
        "UPDATE isahl.{} SET deleted_at = NOW() WHERE {} IN ({}) AND deleted_at IS NULL",
        quote_ident(&target.table),
        quote_ident(&target.fk_column),
        placeholders(ids.len())
    );
    let mut q = sqlx::query(AssertSqlSafe(sql.as_str()));
    for id in ids {
        q = q.bind(*id);
    }
    Ok(q.execute(&mut **tx).await?.rows_affected())
}

/// 按 FK 列匹配批量标记 `deleted_at`，返回本次实际标记的子行 id（RETURNING id）。
async fn update_by_fk_returning(
    tx: &mut Transaction<'_, Postgres>,
    target: &CascadeTarget,
    ids: &[i64],
) -> Result<Vec<i64>, sqlx::Error> {
    if ids.is_empty() {
        return Ok(Vec::new());
    }
    let sql = format!(
        "UPDATE isahl.{} SET deleted_at = NOW() WHERE {} IN ({}) AND deleted_at IS NULL RETURNING id",
        quote_ident(&target.table),
        quote_ident(&target.fk_column),
        placeholders(ids.len())
    );
    let mut q = sqlx::query(AssertSqlSafe(sql.as_str()));
    for id in ids {
        q = q.bind(*id);
    }
    let rows = q.fetch_all(&mut **tx).await?;
    Ok(rows.iter().map(|r| r.get::<i64, _>("id")).collect())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cfg() -> CascadeConfig {
        CascadeConfig::default()
    }

    #[test]
    fn bare_table_name_strips_schema_and_quotes() {
        assert_eq!(
            bare_table_name(r#""isahl"."zc_id_lifecycle""#),
            "zc_id_lifecycle"
        );
        assert_eq!(
            bare_table_name("isahl.zc_id_production"),
            "zc_id_production"
        );
        assert_eq!(bare_table_name("zc_id_foo"), "zc_id_foo");
        assert_eq!(
            bare_table_name(r#""isahl_auth"."meta_permission""#),
            "meta_permission"
        );
    }

    /// 关系表（r_status 桥接，ref_right 指向主实体）→ 默认级联
    #[test]
    fn relation_table_targets_derived_by_default() {
        let targets = derive_cascade_targets("zc_id_status", &cfg());
        let relation = targets
            .iter()
            .find(|t| t.table == "zc_id_lifecycle_r_status")
            .expect("zc_id_lifecycle_r_status 应被推导为级联目标");
        assert_eq!(relation.kind, CascadeKind::RelationTable);
        assert_eq!(relation.fk_column, "ref_right");
    }

    /// 业务引用（zc_id_lifecycle.status → zc_id_status）默认不级联
    #[test]
    fn business_ref_excluded_by_default() {
        // 非明细业务引用（appr 族非 detail）：默认仍不级联
        let targets = derive_cascade_targets("zc_id_subjects", &cfg());
        assert!(
            !targets
                .iter()
                .any(|t| t.table == "zc_id_appr-authorization" && t.fk_column == "fk_subject"),
            "非明细业务引用默认不得级联: {:?}",
            targets
        );
    }

    /// 业务引用显式开启后级联（非明细）
    #[test]
    fn business_ref_included_when_enabled() {
        let config = CascadeConfig {
            business_refs: true,
            ..CascadeConfig::default()
        };
        let targets = derive_cascade_targets("zc_id_subjects", &config);
        let biz = targets
            .iter()
            .find(|t| t.table == "zc_id_appr-authorization" && t.fk_column == "fk_subject")
            .expect("business_refs=true 时应包含 zc_id_appr-authorization.fk_subject");
        assert_eq!(biz.kind, CascadeKind::BusinessRef);
    }

    /// 明细（detail 族）业务引用默认级联（2026-08-27 用户裁决）
    #[test]
    fn detail_business_ref_cascades_by_default() {
        // zc_id_deta-tsp.fk_item → zc_id_lifecycle；fk_list → zc_id_stat-tsp-voucher
        let targets = derive_cascade_targets("zc_id_stat-tsp-voucher", &cfg());
        let biz = targets
            .iter()
            .find(|t| t.table == "zc_id_deta-tsp" && t.fk_column == "fk_list")
            .expect("明细引用默认应级联（detail 族）");
        assert_eq!(biz.kind, CascadeKind::BusinessRef);

        let targets2 = derive_cascade_targets("zc_id_lifecycle", &cfg());
        let biz2 = targets2
            .iter()
            .find(|t| t.table == "zc_id_deta-tsp" && t.fk_column == "fk_item")
            .expect("明细引用默认应级联（detail 族）");
        assert_eq!(biz2.kind, CascadeKind::BusinessRef);
    }

    /// 明细级联可显式关闭（detail_business_refs=false）
    #[test]
    fn detail_business_ref_can_be_disabled() {
        let config = CascadeConfig {
            detail_business_refs: false,
            ..CascadeConfig::default()
        };
        let targets = derive_cascade_targets("zc_id_stat-tsp-voucher", &config);
        assert!(
            !targets
                .iter()
                .any(|t| t.table == "zc_id_deta-tsp" && t.fk_column == "fk_list"),
            "detail_business_refs=false 时明细引用不得级联: {:?}",
            targets
        );
    }

    /// 子实体（fk_previous 自引用链）→ 默认级联
    #[test]
    fn child_entity_targets_derived_by_default() {
        let targets = derive_cascade_targets("zc_id_version", &cfg());
        let child = targets
            .iter()
            .find(|t| t.table == "zc_id_version")
            .expect("fk_previous 自引用应被推导为子实体级联");
        assert_eq!(child.kind, CascadeKind::ChildEntity);
        assert_eq!(child.fk_column, "fk_previous");
    }

    /// 数组/桥接 `_` 列（local_key 为空或 id）不产生级联目标
    #[test]
    fn unsupported_local_keys_skipped() {
        let targets = derive_cascade_targets("zc_ad_scalar", &cfg());
        assert!(
            targets
                .iter()
                .all(|t| !t.fk_column.is_empty() && t.fk_column != "id"),
            "空/id local_key 不得生成目标"
        );
    }

    /// 关系表关闭配置 → 关系目标消失
    #[test]
    fn relation_tables_can_be_disabled() {
        let config = CascadeConfig {
            relation_tables: false,
            ..CascadeConfig::default()
        };
        let targets = derive_cascade_targets("zc_id_status", &config);
        assert!(
            !targets.iter().any(|t| t.kind == CascadeKind::RelationTable),
            "relation_tables=false 时不得包含关系表目标"
        );
    }
}
