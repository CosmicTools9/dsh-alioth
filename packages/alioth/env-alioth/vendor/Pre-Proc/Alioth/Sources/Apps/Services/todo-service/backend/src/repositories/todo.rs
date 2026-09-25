//! todo Repository — AliothStudio 脚手架生成
//!
//! list/get/delete 委托 crud::GenericRepository（含 RLS 变体）；
//! create/update 经运行时触发器通道（crud::trigger），系统管理列与 dk 由本层注入 record。

use crate::models::todo::{CreateTodoRequest, Todo, UpdateTodoRequest};
use async_trait::async_trait;
use common::data::{ListQuery, PaginatedResponse};
use common::error::AliothError;
use crud::trigger;
use crud::{AliothRepository, GenericRepository};
use sqlx::PgPool;

/// DTO 字段名（L2）→ 物理列名（L1）映射：record 构造与触发器读旧值的唯一来源。
/// UPDATE/INSERT 的 record 键必须是物理列名——`crud::trigger` 的 core 直接取 `record.keys()`
/// 当列名，不做列集校验；聚合请求的 `items` 等非列字段经此映射天然被剔除。
const _WRITE_COLUMN_MAP: &[(&str, &str)] = &[
    ("notice", "notice"),
    ("code", "code"),
    ("flag", "flag"),
    ("enable", "enable"),
    ("comments", "comments"),
];

/// 回读映射：触发器通道 RETURNING（物理列名键）→ 实体 DTO 字段名键。
const _READ_COLUMN_MAP: &[(&str, &str)] = &[
    ("id", "id"),
    ("notice", "notice"),
    ("code", "code"),
    ("flag", "flag"),
    ("enable", "enable"),
    ("comments", "comments"),
    ("created_at", "created_at"),
    ("updated_at", "updated_at"),
    ("deleted_at", "deleted_at"),
];

#[derive(Clone)]
pub struct TodoRepository {
    generic: GenericRepository<Todo>,
}

impl TodoRepository {
    pub fn new(pool: PgPool) -> Self {
        Self {
            generic: GenericRepository::new(pool),
        }
    }
}

impl From<PgPool> for TodoRepository {
    fn from(pool: PgPool) -> Self {
        Self::new(pool)
    }
}

#[async_trait]
impl AliothRepository<Todo, CreateTodoRequest, UpdateTodoRequest, AliothError> for TodoRepository {
    async fn list(&self, query: &ListQuery) -> Result<PaginatedResponse<Todo>, AliothError> {
        self.generic.list(query).await
    }

    async fn list_with_rls(
        &self,
        query: &ListQuery,
        visible_ids: Option<&[i64]>,
        authorized_columns: Option<&[String]>,
    ) -> Result<PaginatedResponse<Todo>, AliothError> {
        self.generic
            .list_with_rls(query, visible_ids, authorized_columns)
            .await
    }

    async fn get(&self, id: i64) -> Result<Option<Todo>, AliothError> {
        self.generic.get(id).await
    }

    async fn create(&self, req: CreateTodoRequest, user_id: i64) -> Result<Todo, AliothError> {
        let mut record = trigger::record_from_fields(&req, _WRITE_COLUMN_MAP)
            .map_err(AliothError::Serialization)?;
        record.insert("created_by_id".to_string(), serde_json::json!(user_id));
        record.insert("updated_by_id".to_string(), serde_json::json!(user_id));
        let row = trigger::insert_with_triggers(
            self.generic.pool(),
            "zc_id_stus-task",
            record,
            Some(user_id),
        )
        .await
        .map_err(|e| AliothError::Database(e.to_string()))?;
        trigger::from_record_mapped(&row, _READ_COLUMN_MAP).map_err(AliothError::Serialization)
    }

    async fn update(
        &self,
        id: i64,
        req: UpdateTodoRequest,
        user_id: i64,
    ) -> Result<Option<Todo>, AliothError> {
        let Some(old_record) = trigger::get_record(self.generic.pool(), "zc_id_stus-task", id)
            .await
            .map_err(|e| AliothError::Database(e.to_string()))?
        else {
            return Ok(None);
        };
        let mut record = trigger::record_from_fields(&req, _WRITE_COLUMN_MAP)
            .map_err(AliothError::Serialization)?;
        record.retain(|_, v| !v.is_null());
        record.insert("updated_by_id".to_string(), serde_json::json!(user_id));
        let row = trigger::update_with_triggers(
            self.generic.pool(),
            "zc_id_stus-task",
            id,
            record,
            &old_record,
            Some(user_id),
        )
        .await
        .map_err(|e| AliothError::Database(e.to_string()))?;
        trigger::from_record_mapped(&row, _READ_COLUMN_MAP)
            .map(Some)
            .map_err(AliothError::Serialization)
    }

    async fn delete(&self, id: i64, user_id: i64) -> Result<(), AliothError> {
        self.generic.delete(id, user_id).await
    }
}
