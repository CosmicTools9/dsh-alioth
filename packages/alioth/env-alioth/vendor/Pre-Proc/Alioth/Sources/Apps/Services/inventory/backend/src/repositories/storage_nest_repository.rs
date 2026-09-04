//! StorageNest Repository — 储元⇲储元 时空嵌套 CRUD（显式物化版）
//!
//! 行存在即「置入/IN」状态（无 direction，用户定稿）；删除行（软删）即取出。
//! 物化由 Framework 库函数显式调用：`validate_nest`（环检测，写前）+
//! `apply_nest`（置入/取出 rollup，写后）。
//!
//! ⚠️ 本文件为「并行进程停止后落地」的待应用副本。

use async_trait::async_trait;
use common::error::AliothError as ApiError;
use crud::repository::AliothRepository;
use crud::{GenericRepository, ListQuery, PaginatedResponse};
use serde_json::Value;
use sqlx::PgPool;
use std::collections::HashMap;
use trigger_registry::stock_materialization as sm;

use crate::models::{CreateStorageNestRequest, StorageNest, UpdateStorageNestRequest};

fn nest_map(
    id: Option<i64>,
    parent_id: Option<i64>,
    child_id: Option<i64>,
    period_id: Option<i64>,
) -> HashMap<String, Value> {
    let mut m = HashMap::new();
    if let Some(v) = id {
        m.insert("id".to_string(), Value::from(v));
    }
    if let Some(v) = parent_id {
        m.insert("ref_left".to_string(), Value::from(v));
    }
    if let Some(v) = child_id {
        m.insert("ref_right".to_string(), Value::from(v));
    }
    if let Some(v) = period_id {
        m.insert("qk_period".to_string(), Value::from(v));
    }
    m
}

#[derive(Debug, Clone)]
pub struct StorageNestRepository {
    generic: GenericRepository<StorageNest>,
}

impl StorageNestRepository {
    pub fn new(pool: PgPool) -> Self {
        Self {
            generic: GenericRepository::new(pool),
        }
    }
}

impl From<PgPool> for StorageNestRepository {
    fn from(pool: PgPool) -> Self {
        Self::new(pool)
    }
}

#[async_trait]
impl AliothRepository<StorageNest, CreateStorageNestRequest, UpdateStorageNestRequest, ApiError>
    for StorageNestRepository
{
    async fn list(&self, query: &ListQuery) -> Result<PaginatedResponse<StorageNest>, ApiError> {
        self.generic.list(query).await
    }

    async fn get(&self, id: i64) -> Result<Option<StorageNest>, ApiError> {
        self.generic.get(id).await
    }

    async fn create(
        &self,
        req: CreateStorageNestRequest,
        user_id: i64,
    ) -> Result<StorageNest, ApiError> {
        // 环/冗余校验（写前）
        let pending = nest_map(None, req.parent_id, req.child_id, req.period_id);
        sm::validate_nest(self.generic.pool(), Some(&pending))
            .await
            .map_err(ApiError::BadRequest)?;

        let row = sqlx::query_as::<_, StorageNest>(
            r#"INSERT INTO isahl."zc_id_storage_rr_stock-in"
               (ref_left, ref_right, qk_period, created_by_id)
               VALUES ($1, $2, $3, $4)
               RETURNING id, ref_left AS parent_id, ref_right AS child_id,
                         qk_period AS period_id, created_at, updated_at, deleted_at"#,
        )
        .bind(req.parent_id)
        .bind(req.child_id)
        .bind(req.period_id)
        .bind(user_id)
        .fetch_one(self.generic.pool())
        .await
        .map_err(ApiError::from)?;

        // 物化：置入 rollup
        let new_map = nest_map(Some(row.id), row.parent_id, row.child_id, row.period_id);
        sm::apply_nest(self.generic.pool(), None, Some(&new_map))
            .await
            .map_err(ApiError::Database)?;

        Ok(row)
    }

    async fn update(
        &self,
        id: i64,
        req: UpdateStorageNestRequest,
        user_id: i64,
    ) -> Result<Option<StorageNest>, ApiError> {
        let old = match self.generic.get(id).await? {
            Some(e) => e,
            None => return Ok(None),
        };
        let old_map = nest_map(Some(old.id), old.parent_id, old.child_id, old.period_id);

        // 环/冗余校验（写前，对新状态）
        let pending = nest_map(Some(id), req.parent_id, req.child_id, req.period_id);
        sm::validate_nest(self.generic.pool(), Some(&pending))
            .await
            .map_err(ApiError::BadRequest)?;

        let row = sqlx::query_as::<_, StorageNest>(
            r#"UPDATE isahl."zc_id_storage_rr_stock-in" SET
                   ref_left = COALESCE($1, ref_left),
                   ref_right = COALESCE($2, ref_right),
                   qk_period = COALESCE($3, qk_period),
                   updated_at = NOW(),
                   updated_by_id = $4
               WHERE id = $5 AND deleted_at IS NULL
               RETURNING id, ref_left AS parent_id, ref_right AS child_id,
                         qk_period AS period_id, created_at, updated_at, deleted_at"#,
        )
        .bind(req.parent_id)
        .bind(req.child_id)
        .bind(req.period_id)
        .bind(user_id)
        .bind(id)
        .fetch_optional(self.generic.pool())
        .await
        .map_err(ApiError::from)?;

        if let Some(row) = &row {
            let new_map = nest_map(Some(row.id), row.parent_id, row.child_id, row.period_id);
            // 物化：先反后应
            sm::apply_nest(self.generic.pool(), Some(&old_map), Some(&new_map))
                .await
                .map_err(ApiError::Database)?;
        }

        Ok(row)
    }

    async fn delete(&self, id: i64, user_id: i64) -> Result<(), ApiError> {
        let old = self
            .generic
            .get(id)
            .await?
            .ok_or_else(|| ApiError::NotFound(format!("storage_nest {} not found", id)))?;
        let old_map = nest_map(Some(old.id), old.parent_id, old.child_id, old.period_id);

        sqlx::query(
            r#"UPDATE isahl."zc_id_storage_rr_stock-in"
               SET deleted_at = NOW(), updated_by_id = $2
               WHERE id = $1 AND deleted_at IS NULL"#,
        )
        .bind(id)
        .bind(user_id)
        .execute(self.generic.pool())
        .await
        .map_err(ApiError::from)?;

        // 物化：取出 rollback
        sm::apply_nest(self.generic.pool(), Some(&old_map), None)
            .await
            .map_err(ApiError::Database)?;

        Ok(())
    }
}
