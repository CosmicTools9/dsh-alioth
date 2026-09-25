//! todo 实体模型 — AliothStudio 脚手架生成（crud v2）

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use sqlx::FromRow;

use crud::{AliothDbEntity, Identifiable};

/// todo — 映射 `zc_id_stus-task`（PG 继承自 zc_id_status：读侧列透明，类型判别由子表自身承担——生成契约）
#[derive(Debug, Clone, Serialize, Deserialize, FromRow)]
#[serde(rename_all = "camelCase")]
pub struct Todo {
    #[serde(with = "common::serde_zuid")]
    pub id: i64,

    /// 状态名称（L1 `notice`）
    pub notice: Option<String>,
    /// 状态编码（L1 `code`）
    pub code: Option<String>,
    /// 状态阶段（L1 `flag`，PG 枚举 `status_flag`：start | doing | end；经 `flag::text` 读回）
    pub flag: Option<String>,
    /// 启用（L1 `enable`）
    pub enable: Option<bool>,
    /// 备注（L1 `comments`）
    pub comments: Option<String>,

    pub created_at: Option<DateTime<Utc>>,
    pub updated_at: Option<DateTime<Utc>>,
    pub deleted_at: Option<DateTime<Utc>>,
}

impl Identifiable for Todo {
    fn id(&self) -> i64 {
        self.id
    }
}

impl AliothDbEntity for Todo {
    fn table_name() -> &'static str {
        r#""zc_id_stus-task""#
    }
    const SELECT_FIELDS: &'static str =
        r#"id, notice, code, flag::text, enable, comments, created_at, updated_at, deleted_at"#;
    const ENTITY_NAME: &'static str = "todo";
    const SOFT_DELETE: bool = true;
}

/// 创建请求。
///
/// `flag` 为 **必填**（模型侧 `zc_id_stus-task.flag` 为 PG 枚举 `status_flag`、NOT NULL 且无默认值：
/// 缺列必违约；见 2026-09-22 写径回归）；其余列可空。
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CreateTodoRequest {
    pub notice: Option<String>,
    pub code: Option<String>,
    /// 状态阶段（`start` | `doing` | `end`）
    pub flag: String,
    pub enable: Option<bool>,
    pub comments: Option<String>,
}

/// 部分更新：全部字段可选，未提供的字段保持原值（COALESCE 语义）。
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UpdateTodoRequest {
    pub notice: Option<String>,
    pub code: Option<String>,
    /// 状态阶段（`start` | `doing` | `end`）
    pub flag: Option<String>,
    pub enable: Option<bool>,
    pub comments: Option<String>,
}
