//! StockCountStatus Repository — isahl."zc_id_stus-counting" CRUD（crud 模式：GenericRepository 委托读侧/删除）。
use async_trait::async_trait;
use common::AliothError as ApiError;
use crud::{AliothRepository, GenericRepository, ListQuery, PaginatedResponse};
use sqlx::PgPool;

use crate::models::{
    CreateStockCountStatusRequest, StockCountStatus, UpdateStockCountStatusRequest,
};

#[derive(Clone)]
pub struct StockCountStatusRepository {
    generic: GenericRepository<StockCountStatus>,
    pool: PgPool,
}

impl StockCountStatusRepository {
    pub fn new(pool: PgPool) -> Self {
        Self {
            generic: GenericRepository::new(pool.clone()),
            pool,
        }
    }

    /// 按 ID 读取（含 _refs 解析）
    pub async fn get_refs(&self, id: i64) -> Result<Option<StockCountStatus>, ApiError> {
        self.generic.get_refs(id, None).await
    }
}

#[async_trait]
impl
    AliothRepository<
        StockCountStatus,
        CreateStockCountStatusRequest,
        UpdateStockCountStatusRequest,
        ApiError,
    > for StockCountStatusRepository
{
    async fn list(
        &self,
        query: &ListQuery,
    ) -> Result<PaginatedResponse<StockCountStatus>, ApiError> {
        self.generic.list_refs(query).await
    }

    async fn get(&self, id: i64) -> Result<Option<StockCountStatus>, ApiError> {
        self.generic.get_refs(id, None).await
    }

    async fn create(
        &self,
        req: CreateStockCountStatusRequest,
        user_id: i64,
    ) -> Result<StockCountStatus, ApiError> {
        let id: i64 = sqlx::query_scalar(
            r#"INSERT INTO isahl."zc_id_stus-counting"
               (notice, code, comments, enable, created_by_id)
               VALUES ($1, $2, $3, $4, $5)
               RETURNING id"#,
        )
        .bind(&req.notice)
        .bind(&req.code)
        .bind(&req.comments)
        .bind(req.enable)
        .bind(user_id)
        .fetch_one(&self.pool)
        .await
        .map_err(ApiError::from)?;

        self.get_refs(id)
            .await?
            .ok_or_else(|| ApiError::NotFound(format!("stock_count_status {} not found", id)))
    }

    async fn update(
        &self,
        id: i64,
        req: UpdateStockCountStatusRequest,
        user_id: i64,
    ) -> Result<Option<StockCountStatus>, ApiError> {
        if self.get_refs(id).await?.is_none() {
            return Ok(None);
        }

        let rows = sqlx::query(
            r#"UPDATE isahl."zc_id_stus-counting"
               SET                    notice = COALESCE($1, notice),
                   code = COALESCE($2, code),
                   comments = COALESCE($3, comments),
                   enable = COALESCE($4, enable),
                   updated_by_id = $5
               WHERE id = $6 AND deleted_at IS NULL"#,
        )
        .bind(&req.notice)
        .bind(&req.code)
        .bind(&req.comments)
        .bind(req.enable)
        .bind(user_id)
        .bind(id)
        .execute(&self.pool)
        .await
        .map_err(ApiError::from)?;
        if rows.rows_affected() == 0 {
            return Ok(None);
        }

        self.get_refs(id).await
    }

    async fn delete(&self, id: i64, user_id: i64) -> Result<(), ApiError> {
        self.generic.delete(id, user_id).await
    }
}
