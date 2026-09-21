//! todo-min Repository — AliothStudio 脚手架生成
//!
//! list/get/delete 委托 crud::GenericRepository（含 RLS 变体）；
//! create/update 为生成 SQL（系统管理列排除，dk 由 DkContext 注入）。

use crate::models::todo_min::{CreateTodoMinRequest, TodoMin, UpdateTodoMinRequest};
use async_trait::async_trait;
use common::data::{ListQuery, PaginatedResponse};
use common::error::AliothError;
use crud::{AliothRepository, GenericRepository};
use sqlx::{AssertSqlSafe, PgPool};

pub const _SQL_INSERT: &str = r#"INSERT INTO isahl."zc_id_leve-task" ("notice") VALUES ($1) RETURNING id, "notice" AS "comment", created_at, updated_at, deleted_at"#;
pub const _SQL_UPDATE: &str = r#"UPDATE isahl."zc_id_leve-task" SET "notice" = COALESCE($1, "notice") WHERE id = $2 RETURNING id, "notice" AS "comment", created_at, updated_at, deleted_at"#;

#[derive(Clone)]
pub struct TodoMinRepository {
    generic: GenericRepository<TodoMin>,
}

impl TodoMinRepository {
    pub fn new(pool: PgPool) -> Self {
        Self {
            generic: GenericRepository::new(pool),
        }
    }
}

impl From<PgPool> for TodoMinRepository {
    fn from(pool: PgPool) -> Self {
        Self::new(pool)
    }
}

#[async_trait]
impl AliothRepository<TodoMin, CreateTodoMinRequest, UpdateTodoMinRequest, AliothError>
    for TodoMinRepository
{
    async fn list(&self, query: &ListQuery) -> Result<PaginatedResponse<TodoMin>, AliothError> {
        self.generic.list(query).await
    }

    async fn list_with_rls(
        &self,
        query: &ListQuery,
        visible_ids: Option<&[i64]>,
        authorized_columns: Option<&[String]>,
    ) -> Result<PaginatedResponse<TodoMin>, AliothError> {
        self.generic
            .list_with_rls(query, visible_ids, authorized_columns)
            .await
    }

    async fn get(&self, id: i64) -> Result<Option<TodoMin>, AliothError> {
        self.generic.get(id).await
    }

    async fn create(
        &self,
        req: CreateTodoMinRequest,
        user_id: i64,
    ) -> Result<TodoMin, AliothError> {
        let _ = user_id;
        sqlx::query_as::<_, TodoMin>(AssertSqlSafe(_SQL_INSERT))
            .bind(&req.comment)
            .fetch_one(self.generic.pool())
            .await
            .map_err(Into::into)
    }

    async fn update(
        &self,
        id: i64,
        req: UpdateTodoMinRequest,
        user_id: i64,
    ) -> Result<Option<TodoMin>, AliothError> {
        let _ = user_id;
        sqlx::query_as::<_, TodoMin>(AssertSqlSafe(_SQL_UPDATE))
            .bind(&req.comment)
            .bind(id)
            .fetch_optional(self.generic.pool())
            .await
            .map_err(Into::into)
    }

    async fn delete(&self, id: i64, user_id: i64) -> Result<(), AliothError> {
        self.generic.delete(id, user_id).await
    }
}
