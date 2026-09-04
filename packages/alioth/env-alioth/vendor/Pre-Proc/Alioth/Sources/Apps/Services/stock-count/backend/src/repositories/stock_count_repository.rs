//! StockCount Repository — isahl."zc_id_even-counting" CRUD（crud 模式：GenericRepository 委托读侧/删除）。
use async_trait::async_trait;
use common::AliothError as ApiError;
use crud::{AliothRepository, GenericRepository, ListQuery, PaginatedResponse};
use sqlx::PgPool;

use crate::models::{CreateStockCountRequest, StockCount, UpdateStockCountRequest};

#[derive(Clone)]
pub struct StockCountRepository {
    generic: GenericRepository<StockCount>,
    pool: PgPool,
}

impl StockCountRepository {
    pub fn new(pool: PgPool) -> Self {
        Self {
            generic: GenericRepository::new(pool.clone()),
            pool,
        }
    }

    /// 按 ID 读取（含 _refs 解析）
    pub async fn get_refs(&self, id: i64) -> Result<Option<StockCount>, ApiError> {
        self.generic.get_refs(id, None).await
    }
}

#[async_trait]
impl AliothRepository<StockCount, CreateStockCountRequest, UpdateStockCountRequest, ApiError>
    for StockCountRepository
{
    async fn list(&self, query: &ListQuery) -> Result<PaginatedResponse<StockCount>, ApiError> {
        self.generic.list_refs(query).await
    }

    async fn get(&self, id: i64) -> Result<Option<StockCount>, ApiError> {
        self.generic.get_refs(id, None).await
    }

    async fn create(
        &self,
        req: CreateStockCountRequest,
        user_id: i64,
    ) -> Result<StockCount, ApiError> {
        let id: i64 = sqlx::query_scalar(
            r#"INSERT INTO isahl."zc_id_even-counting"
               (notice, code, comments, ak_source, qk_date, fk_place, fk_subject, fk_storage, summary, created_by_id)
               VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10)
               RETURNING id"#,
        )
        .bind(&req.notice)
        .bind(&req.code)
        .bind(&req.comments)
        .bind(&req.ak_source)
        .bind(req.qk_date)
        .bind(req.fk_place)
        .bind(req.fk_subject)
        .bind(req.fk_storage)
        .bind(&req.summary)
        .bind(user_id)
        .fetch_one(&self.pool)
        .await
        .map_err(ApiError::from)?;

        self.get_refs(id)
            .await?
            .ok_or_else(|| ApiError::NotFound(format!("stock_count {} not found", id)))
    }

    async fn update(
        &self,
        id: i64,
        req: UpdateStockCountRequest,
        user_id: i64,
    ) -> Result<Option<StockCount>, ApiError> {
        if self.get_refs(id).await?.is_none() {
            return Ok(None);
        }

        let rows = sqlx::query(
            r#"UPDATE isahl."zc_id_even-counting"
               SET                    notice = COALESCE($1, notice),
                   code = COALESCE($2, code),
                   comments = COALESCE($3, comments),
                   ak_source = COALESCE($4, ak_source),
                   qk_date = COALESCE($5, qk_date),
                   fk_place = COALESCE($6, fk_place),
                   fk_subject = COALESCE($7, fk_subject),
                   fk_storage = COALESCE($8, fk_storage),
                   summary = COALESCE($9, summary),
                   updated_by_id = $10
               WHERE id = $11 AND deleted_at IS NULL"#,
        )
        .bind(&req.notice)
        .bind(&req.code)
        .bind(&req.comments)
        .bind(&req.ak_source)
        .bind(req.qk_date)
        .bind(req.fk_place)
        .bind(req.fk_subject)
        .bind(req.fk_storage)
        .bind(&req.summary)
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
