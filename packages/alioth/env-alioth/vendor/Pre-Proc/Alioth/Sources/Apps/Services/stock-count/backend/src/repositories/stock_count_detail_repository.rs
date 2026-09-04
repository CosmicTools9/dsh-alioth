//! StockCountDetail Repository — isahl."zc_id_deta-counting" CRUD（crud 模式：GenericRepository 委托读侧/删除）。
use async_trait::async_trait;
use common::AliothError as ApiError;
use crud::{AliothRepository, GenericRepository, ListQuery, PaginatedResponse};
use sqlx::PgPool;

use crate::models::{
    CreateStockCountDetailRequest, StockCountDetail, UpdateStockCountDetailRequest,
};

#[derive(Clone)]
pub struct StockCountDetailRepository {
    generic: GenericRepository<StockCountDetail>,
    pool: PgPool,
}

impl StockCountDetailRepository {
    pub fn new(pool: PgPool) -> Self {
        Self {
            generic: GenericRepository::new(pool.clone()),
            pool,
        }
    }

    /// 按 ID 读取（含 _refs 解析）
    pub async fn get_refs(&self, id: i64) -> Result<Option<StockCountDetail>, ApiError> {
        self.generic.get_refs(id, None).await
    }
}

#[async_trait]
impl
    AliothRepository<
        StockCountDetail,
        CreateStockCountDetailRequest,
        UpdateStockCountDetailRequest,
        ApiError,
    > for StockCountDetailRepository
{
    async fn list(
        &self,
        query: &ListQuery,
    ) -> Result<PaginatedResponse<StockCountDetail>, ApiError> {
        self.generic.list_refs(query).await
    }

    async fn get(&self, id: i64) -> Result<Option<StockCountDetail>, ApiError> {
        self.generic.get_refs(id, None).await
    }

    async fn create(
        &self,
        req: CreateStockCountDetailRequest,
        user_id: i64,
    ) -> Result<StockCountDetail, ApiError> {
        let id: i64 = sqlx::query_scalar(
            r#"INSERT INTO isahl."zc_id_deta-counting"
               (notice, code, comments, ak_source, qk_date, ck_category, fk_list, fk_biller, fk_production, fk_storage, qk_qty, qk_w_qty, qk_v_qty, fk_voucher, created_by_id)
               VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, $13, $14, $15)
               RETURNING id"#,
        )
        .bind(&req.notice)
        .bind(&req.code)
        .bind(&req.comments)
        .bind(&req.ak_source)
        .bind(req.qk_date)
        .bind(req.ck_category)
        .bind(req.fk_list)
        .bind(req.fk_biller)
        .bind(req.fk_production)
        .bind(req.fk_storage)
        .bind(req.qk_qty)
        .bind(req.qk_w_qty)
        .bind(req.qk_v_qty)
        .bind(req.fk_voucher)
        .bind(user_id)
        .fetch_one(&self.pool)
        .await
        .map_err(ApiError::from)?;

        self.get_refs(id)
            .await?
            .ok_or_else(|| ApiError::NotFound(format!("stock_count_detail {} not found", id)))
    }

    async fn update(
        &self,
        id: i64,
        req: UpdateStockCountDetailRequest,
        user_id: i64,
    ) -> Result<Option<StockCountDetail>, ApiError> {
        if self.get_refs(id).await?.is_none() {
            return Ok(None);
        }

        let rows = sqlx::query(
            r#"UPDATE isahl."zc_id_deta-counting"
               SET                    notice = COALESCE($1, notice),
                   code = COALESCE($2, code),
                   comments = COALESCE($3, comments),
                   ak_source = COALESCE($4, ak_source),
                   qk_date = COALESCE($5, qk_date),
                   ck_category = COALESCE($6, ck_category),
                   fk_list = COALESCE($7, fk_list),
                   fk_biller = COALESCE($8, fk_biller),
                   fk_production = COALESCE($9, fk_production),
                   fk_storage = COALESCE($10, fk_storage),
                   qk_qty = COALESCE($11, qk_qty),
                   qk_w_qty = COALESCE($12, qk_w_qty),
                   qk_v_qty = COALESCE($13, qk_v_qty),
                   fk_voucher = COALESCE($14, fk_voucher),
                   updated_by_id = $15
               WHERE id = $16 AND deleted_at IS NULL"#,
        )
        .bind(&req.notice)
        .bind(&req.code)
        .bind(&req.comments)
        .bind(&req.ak_source)
        .bind(req.qk_date)
        .bind(req.ck_category)
        .bind(req.fk_list)
        .bind(req.fk_biller)
        .bind(req.fk_production)
        .bind(req.fk_storage)
        .bind(req.qk_qty)
        .bind(req.qk_w_qty)
        .bind(req.qk_v_qty)
        .bind(req.fk_voucher)
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
