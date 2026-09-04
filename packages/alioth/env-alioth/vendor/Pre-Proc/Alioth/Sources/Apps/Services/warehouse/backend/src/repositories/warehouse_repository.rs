//! Warehouse Repository — isahl."zc_id_cate-warehouse" CRUD（crud 模式：GenericRepository 委托读侧/删除）。
use async_trait::async_trait;
use common::AliothError as ApiError;
use crud::{AliothRepository, GenericRepository, ListQuery, PaginatedResponse};
use sqlx::PgPool;

use crate::models::{CreateWarehouseRequest, UpdateWarehouseRequest, Warehouse};

#[derive(Clone)]
pub struct WarehouseRepository {
    generic: GenericRepository<Warehouse>,
    pool: PgPool,
}

impl WarehouseRepository {
    pub fn new(pool: PgPool) -> Self {
        Self {
            generic: GenericRepository::new(pool.clone()),
            pool,
        }
    }

    /// 按 ID 读取（含 _refs 解析）
    pub async fn get_refs(&self, id: i64) -> Result<Option<Warehouse>, ApiError> {
        self.generic.get_refs(id, None).await
    }
}

#[async_trait]
impl AliothRepository<Warehouse, CreateWarehouseRequest, UpdateWarehouseRequest, ApiError>
    for WarehouseRepository
{
    async fn list(&self, query: &ListQuery) -> Result<PaginatedResponse<Warehouse>, ApiError> {
        self.generic.list_refs(query).await
    }

    async fn get(&self, id: i64) -> Result<Option<Warehouse>, ApiError> {
        self.generic.get_refs(id, None).await
    }

    async fn create(
        &self,
        req: CreateWarehouseRequest,
        user_id: i64,
    ) -> Result<Warehouse, ApiError> {
        let id: i64 = sqlx::query_scalar(
            r#"INSERT INTO isahl."zc_id_cate-warehouse"
               (notice, code, comments, enable, c_sort_, created_by_id)
               VALUES ($1, $2, $3, $4, $5, $6)
               RETURNING id"#,
        )
        .bind(&req.notice)
        .bind(&req.code)
        .bind(&req.comments)
        .bind(req.enable)
        .bind(req.c_sort_)
        .bind(user_id)
        .fetch_one(&self.pool)
        .await
        .map_err(ApiError::from)?;

        self.get_refs(id)
            .await?
            .ok_or_else(|| ApiError::NotFound(format!("warehouse {} not found", id)))
    }

    async fn update(
        &self,
        id: i64,
        req: UpdateWarehouseRequest,
        user_id: i64,
    ) -> Result<Option<Warehouse>, ApiError> {
        if self.get_refs(id).await?.is_none() {
            return Ok(None);
        }

        let rows = sqlx::query(
            r#"UPDATE isahl."zc_id_cate-warehouse"
               SET                    notice = COALESCE($1, notice),
                   code = COALESCE($2, code),
                   comments = COALESCE($3, comments),
                   enable = COALESCE($4, enable),
                   c_sort_ = COALESCE($5, c_sort_),
                   updated_by_id = $6
               WHERE id = $7 AND deleted_at IS NULL"#,
        )
        .bind(&req.notice)
        .bind(&req.code)
        .bind(&req.comments)
        .bind(req.enable)
        .bind(req.c_sort_)
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
