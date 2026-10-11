//! Requirement 实体模型 — 映射 `isahl.zc_id_event`（继承 `zc_id_lifecycle`）。
//!
//! ## 承载映射（BIDDING §3.6 / design D1 修正）
//! - `code`（需求编号，用户输入）/ `notice`（标题 → DTO `name`）/ `comments` /
//!   `fk_place`（→ DTO `place`）
//! - 类目：`zc_id_event` **无 `ck_category` 物理列**（2026-08-11 DB 实测，D1 修正）——
//!   经 `isahl.zc_id_lifecycle_r_category` 关联表承载（belongsToMany 先例见
//!   `Services/isahl-db/service.json`）。本服务采用**单值类目约定**：写入时先软删旧关联再插入
//!   新关联（ref_left=实体 id / ref_right=category id），读取经同表子查询取最新一行，
//!   `_refs.category` 经 `JoinKind::Junction`（ToOne + ORDER BY jt.id LIMIT 1）解析类目名称。
//! - `dk_scene/dk_factor/dk_function`：DTO 不暴露（BACKEND_FRAMEWORK §7.3），写入 NULL
//!   （与既有 `zc_id_event` 数据一致；坐标注入机制未接线）。

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use sqlx::FromRow;

use crud::{AliothDbEntity, Card, HasReferenceJoins, Identifiable, JoinKind, ReferenceJoin};

/// 需求条目读模型（RequirementItem，契约字段名，camelCase 输出）
#[derive(Debug, Clone, Serialize, Deserialize, FromRow)]
#[serde(rename_all = "camelCase")]
pub struct Requirement {
    #[serde(with = "common::serde_zuid")]
    pub id: i64,
    /// 需求编号（用户输入，非系统 o_number）
    pub code: Option<String>,
    /// 需求标题（物理列 notice → L2 命名 name）
    pub name: String,
    /// 需求详细描述
    pub comments: Option<String>,
    /// 类目 ID（经 zc_id_lifecycle_r_category 关联承载）
    #[serde(with = "common::serde_zuid::opt", default)]
    pub category: Option<i64>,
    /// 场所 ID（fk_place → zc_id_lifecycle）
    #[serde(with = "common::serde_zuid::opt", default)]
    pub place: Option<i64>,
    pub created_at: DateTime<Utc>,
    pub updated_at: Option<DateTime<Utc>>,
    /// 关联引用嵌入数据（_refs.category / _refs.place 名称解析，禁 raw ID）
    #[sqlx(default)]
    #[serde(rename = "_refs")]
    pub _refs: Option<serde_json::Value>,
}

impl Identifiable for Requirement {
    fn id(&self) -> i64 {
        self.id
    }
}

impl AliothDbEntity for Requirement {
    fn table_name() -> &'static str {
        r#"isahl."zc_id_event""#
    }

    /// `category` 为关联子查询（取 zc_id_lifecycle_r_category 最新未删行，单值语义；
    /// 子查询经外层别名 `e` 关联——本实体所有查询路径（list_refs/get_refs/delete 内部 get）
    /// 均以 `e` 为表别名，禁在未别名路径使用）。
    const SELECT_FIELDS: &'static str =
        "id, code, notice AS name, comments, \
         (SELECT jt0.ref_right FROM isahl.zc_id_lifecycle_r_category jt0 \
          WHERE jt0.ref_left = e.id AND jt0.deleted_at IS NULL ORDER BY jt0.id DESC LIMIT 1) AS category, \
         fk_place AS place, \
         created_at, updated_at";
    const ENTITY_NAME: &'static str = "requirement";
    const SOFT_DELETE: bool = true;
    const HAS_AUDIT: bool = false;
}

impl HasReferenceJoins for Requirement {
    fn reference_joins() -> Vec<ReferenceJoin> {
        vec![
            // 类目：穿透 zc_id_lifecycle_r_category 桥接（单值约定；写入替换时旧行软删，
            // 取最新一行 → ORDER BY jt.id DESC LIMIT 1，与 SELECT_FIELDS 子查询一致）
            ReferenceJoin {
                name: "category",
                card: Card::ToOne,
                kind: JoinKind::OrderedJunction {
                    junction_table: "isahl.zc_id_lifecycle_r_category",
                    source_fk: "ref_left",
                    target_fk: "ref_right",
                    order_by: Some("id"),
                    order_desc: true,
                    nulls_last: false,
                    junction_display_fields: &[],
                },
                target_table: "isahl.zc_id_category",
                display_fields: &["notice"],
            },
            // 场所：fk_place → zc_id_lifecycle（m2o）
            ReferenceJoin {
                name: "place",
                card: Card::ToOne,
                kind: JoinKind::Forward {
                    local_fk: "fk_place",
                    target_key: "id",
                },
                target_table: "isahl.zc_id_lifecycle",
                display_fields: &["notice"],
            },
        ]
    }
}

/// 创建需求输入（DTO_DESIGN_SPEC §6.1 用户可写集，L2 业务命名）
///
/// 禁止字段：`dk_*` / `o_number` / `id` / `created_at` / `updated_at` /
/// `deleted_at` / `created_by_id` / `updated_by_id`（系统与维度派生）。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CreateRequirementRequest {
    /// 需求编号（必填，用户输入）
    pub code: String,
    /// 需求标题（必填）
    pub name: String,
    pub comments: Option<String>,
    /// 类目 ID（单值，经关联表承载）
    #[serde(with = "common::serde_zuid::opt", default)]
    pub category: Option<i64>,
    /// 场所 ID
    #[serde(with = "common::serde_zuid::opt", default)]
    pub place: Option<i64>,
}

/// 更新需求输入（全 Optional，同可写集）
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UpdateRequirementRequest {
    pub code: Option<String>,
    pub name: Option<String>,
    pub comments: Option<String>,
    #[serde(with = "common::serde_zuid::opt", default)]
    pub category: Option<i64>,
    #[serde(with = "common::serde_zuid::opt", default)]
    pub place: Option<i64>,
}

/// 维度选择器选项（{id, name}）
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DimensionOption {
    #[serde(with = "common::serde_zuid")]
    pub id: i64,
    pub name: String,
}

/// 维度选择器数据源响应（Contract：GET /service/demand/requirements/dimensions）
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DimensionsResponse {
    pub categories: Vec<DimensionOption>,
    pub places: Vec<DimensionOption>,
}
