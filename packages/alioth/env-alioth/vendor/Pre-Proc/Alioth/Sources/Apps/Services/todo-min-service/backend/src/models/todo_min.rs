//! todo-min 实体模型 — AliothStudio 脚手架生成（crud v2）

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use sqlx::FromRow;

use crud::{AliothDbEntity, Identifiable};

/// todo-min — 映射 `zc_id_leve-task`（PG 继承自 zc_id_level：读侧列透明，类型判别由子表自身承担——生成契约）
#[derive(Debug, Clone, Serialize, Deserialize, FromRow)]
#[serde(rename_all = "camelCase")]
pub struct TodoMin {
    #[serde(with = "common::serde_zuid")]
    pub id: i64,
    pub comment: Option<String>,

    pub created_at: Option<DateTime<Utc>>,
    pub updated_at: Option<DateTime<Utc>>,
    pub deleted_at: Option<DateTime<Utc>>,
}

impl Identifiable for TodoMin {
    fn id(&self) -> i64 {
        self.id
    }
}

impl AliothDbEntity for TodoMin {
    fn table_name() -> &'static str {
        r#""zc_id_leve-task""#
    }
    const SELECT_FIELDS: &'static str =
        r#"id, "notice" AS "comment", created_at, updated_at, deleted_at"#;
    const ENTITY_NAME: &'static str = "todo-min";
    const SOFT_DELETE: bool = true;
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CreateTodoMinRequest {
    /// comment
    pub comment: Option<String>,
}

/// 部分更新：全部字段可选，未提供的字段保持原值（COALESCE 语义）。
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UpdateTodoMinRequest {
    pub comment: Option<String>,
}
