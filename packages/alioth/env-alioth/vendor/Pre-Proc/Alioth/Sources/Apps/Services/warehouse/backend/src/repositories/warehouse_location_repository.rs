//! WarehouseLocation Repository — isahl."zc_id_stor-plc-warehouse" CRUD（crud 模式：GenericRepository 委托读侧/删除）。
use async_trait::async_trait;
use common::AliothError as ApiError;
use crud::{AliothRepository, GenericRepository, ListQuery, PaginatedResponse};
use sqlx::PgPool;

use crate::models::{
    CreateWarehouseLocationRequest, UpdateWarehouseLocationRequest, WarehouseLocation,
};

#[derive(Clone)]
pub struct WarehouseLocationRepository {
    generic: GenericRepository<WarehouseLocation>,
    pool: PgPool,
}

impl WarehouseLocationRepository {
    pub fn new(pool: PgPool) -> Self {
        Self {
            generic: GenericRepository::new(pool.clone()),
            pool,
        }
    }

    /// 按 ID 读取（含 _refs 解析）
    pub async fn get_refs(&self, id: i64) -> Result<Option<WarehouseLocation>, ApiError> {
        self.generic.get_refs(id, None).await
    }
}

#[async_trait]
impl
    AliothRepository<
        WarehouseLocation,
        CreateWarehouseLocationRequest,
        UpdateWarehouseLocationRequest,
        ApiError,
    > for WarehouseLocationRepository
{
    async fn list(
        &self,
        query: &ListQuery,
    ) -> Result<PaginatedResponse<WarehouseLocation>, ApiError> {
        self.generic.list_refs(query).await
    }

    async fn get(&self, id: i64) -> Result<Option<WarehouseLocation>, ApiError> {
        self.generic.get_refs(id, None).await
    }

    async fn create(
        &self,
        req: CreateWarehouseLocationRequest,
        user_id: i64,
    ) -> Result<WarehouseLocation, ApiError> {
        let id: i64 = sqlx::query_scalar(
            r#"INSERT INTO isahl."zc_id_stor-plc-warehouse"
               (notice, code, comments, ak_source, fk_address, sk_unit, fk_trustee, qk_capacity, fk_parent, qk_fence, created_by_id)
               VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11)
               RETURNING id"#,
        )
        .bind(&req.notice)
        .bind(&req.code)
        .bind(&req.comments)
        .bind(&req.ak_source)
        .bind(req.fk_address)
        .bind(req.sk_unit)
        .bind(req.fk_trustee)
        .bind(req.qk_capacity)
        .bind(req.fk_parent)
        .bind(req.qk_fence)
        .bind(user_id)
        .fetch_one(&self.pool)
        .await
        .map_err(ApiError::from)?;

        self.get_refs(id)
            .await?
            .ok_or_else(|| ApiError::NotFound(format!("warehouse_location {} not found", id)))
    }

    async fn update(
        &self,
        id: i64,
        req: UpdateWarehouseLocationRequest,
        user_id: i64,
    ) -> Result<Option<WarehouseLocation>, ApiError> {
        if self.get_refs(id).await?.is_none() {
            return Ok(None);
        }

        let rows = sqlx::query(
            r#"UPDATE isahl."zc_id_stor-plc-warehouse"
               SET                    notice = COALESCE($1, notice),
                   code = COALESCE($2, code),
                   comments = COALESCE($3, comments),
                   ak_source = COALESCE($4, ak_source),
                   fk_address = COALESCE($5, fk_address),
                   sk_unit = COALESCE($6, sk_unit),
                   fk_trustee = COALESCE($7, fk_trustee),
                   qk_capacity = COALESCE($8, qk_capacity),
                   fk_parent = COALESCE($9, fk_parent),
                   qk_fence = COALESCE($10, qk_fence),
                   updated_by_id = $11
               WHERE id = $12 AND deleted_at IS NULL"#,
        )
        .bind(&req.notice)
        .bind(&req.code)
        .bind(&req.comments)
        .bind(&req.ak_source)
        .bind(req.fk_address)
        .bind(req.sk_unit)
        .bind(req.fk_trustee)
        .bind(req.qk_capacity)
        .bind(req.fk_parent)
        .bind(req.qk_fence)
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
