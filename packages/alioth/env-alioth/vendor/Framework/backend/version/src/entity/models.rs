//! 实体版本模型（entity 面）——`isahl.zc_id_version` 版本链
//!
//! 1:1 提取自 WZ/Alioth version 生产实现（consolidate-version-services）。

use chrono::{DateTime, Utc};
use crud::entity::{AliothDbEntity, Identifiable};
use serde::{Deserialize, Serialize};
use sqlx::FromRow;

/// 版本记录 — 映射 `isahl.zc_id_version`
///
/// 字段并集吸收 WZ/Alioth 两 ns 契约：
/// - WZ：notice/code/comments/tk_version/tk_batch_no/fk_previous/ck_branch（链语义）
/// - Alioth：tpl_id（实体锚点）/tk_version→version_number/reversion→revision/fk_previous→previous_id
#[derive(Debug, Clone, Serialize, Deserialize, FromRow)]
pub struct VersionRecord {
    #[serde(with = "common::serde_zuid")]
    pub id: i64,
    /// 实体锚点（Alioth 语义；WZ 不消费）
    #[serde(with = "common::serde_zuid::opt", default)]
    pub tpl_id: Option<i64>,
    pub notice: Option<String>,
    pub code: Option<String>,
    pub comments: Option<String>,
    #[serde(with = "common::serde_zuid::opt", default)]
    pub tk_version: Option<i64>,
    #[serde(with = "common::serde_zuid::opt", default)]
    pub tk_batch_no: Option<i64>,
    /// 修订号（Alioth revision 语义；WZ 不消费）
    #[serde(with = "common::serde_zuid::opt", default)]
    pub reversion: Option<i64>,
    #[serde(with = "common::serde_zuid::opt", default)]
    pub fk_previous: Option<i64>,
    #[serde(with = "common::serde_zuid::opt", default)]
    pub ck_branch: Option<i64>,
    pub created_at: DateTime<Utc>,
    pub updated_at: Option<DateTime<Utc>>,
    pub deleted_at: Option<DateTime<Utc>>,
}

impl Identifiable for VersionRecord {
    fn id(&self) -> i64 {
        self.id
    }
}

impl AliothDbEntity for VersionRecord {
    fn table_name() -> &'static str {
        "isahl.zc_id_version"
    }
    const SELECT_FIELDS: &'static str =
        "id, tpl_id, notice, code, comments, tk_version, tk_batch_no, reversion, fk_previous, ck_branch, created_at, updated_at, deleted_at";
    const ENTITY_NAME: &'static str = "version";
    const SOFT_DELETE: bool = true;
    const HAS_AUDIT: bool = false;
}

/// 创建版本请求
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CreateVersionRequest {
    pub notice: Option<String>,
    pub code: Option<String>,
    pub comments: Option<String>,
    #[serde(with = "common::serde_zuid::opt", default)]
    pub tk_version: Option<i64>,
    #[serde(with = "common::serde_zuid::opt", default)]
    pub tk_batch_no: Option<i64>,
    #[serde(with = "common::serde_zuid::opt", default)]
    pub reversion: Option<i64>,
    #[serde(with = "common::serde_zuid::opt", default)]
    pub fk_previous: Option<i64>,
    #[serde(with = "common::serde_zuid::opt", default)]
    pub ck_branch: Option<i64>,
    /// 实体锚点（Alioth 语义）
    #[serde(with = "common::serde_zuid::opt", default)]
    pub tpl_id: Option<i64>,
}

/// 更新版本请求（PATCH 语义）
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UpdateVersionRequest {
    pub notice: Option<String>,
    pub code: Option<String>,
    pub comments: Option<String>,
    #[serde(with = "common::serde_zuid::opt", default)]
    pub tk_version: Option<i64>,
    #[serde(with = "common::serde_zuid::opt", default)]
    pub tk_batch_no: Option<i64>,
    #[serde(with = "common::serde_zuid::opt", default)]
    pub reversion: Option<i64>,
    #[serde(with = "common::serde_zuid::opt", default)]
    pub fk_previous: Option<i64>,
    #[serde(with = "common::serde_zuid::opt", default)]
    pub ck_branch: Option<i64>,
    #[serde(with = "common::serde_zuid::opt", default)]
    pub tpl_id: Option<i64>,
}
