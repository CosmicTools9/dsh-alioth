//! InboundOrder Repository — isahl."zc_id_plan-inbound" CRUD（crud 模式：GenericRepository 委托读侧/删除）。
use async_trait::async_trait;
use common::AliothError as ApiError;
use crud::{AliothRepository, GenericRepository, ListQuery, PaginatedResponse};
use sqlx::PgPool;

use crate::models::{CreateInboundOrderRequest, InboundOrder, UpdateInboundOrderRequest};

#[derive(Clone)]
pub struct InboundOrderRepository {
    generic: GenericRepository<InboundOrder>,
    pool: PgPool,
}

impl InboundOrderRepository {
    pub fn new(pool: PgPool) -> Self {
        Self {
            generic: GenericRepository::new(pool.clone()),
            pool,
        }
    }

    /// 按 ID 读取（含 _refs 解析）
    pub async fn get_refs(&self, id: i64) -> Result<Option<InboundOrder>, ApiError> {
        self.generic.get_refs(id, None).await
    }
}

#[async_trait]
impl AliothRepository<InboundOrder, CreateInboundOrderRequest, UpdateInboundOrderRequest, ApiError>
    for InboundOrderRepository
{
    async fn list(&self, query: &ListQuery) -> Result<PaginatedResponse<InboundOrder>, ApiError> {
        self.generic.list_refs(query).await
    }

    async fn get(&self, id: i64) -> Result<Option<InboundOrder>, ApiError> {
        self.generic.get_refs(id, None).await
    }

    async fn create(
        &self,
        req: CreateInboundOrderRequest,
        user_id: i64,
    ) -> Result<InboundOrder, ApiError> {
        let id: i64 = sqlx::query_scalar(
            r#"INSERT INTO isahl."zc_id_plan-inbound"
               (notice, code, comments, ak_source, cron, sort, "qk_date-segm", "qk_time-segm", qk_progress, progress_pct, schedule_pct, lk_health, created_by_id)
               VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, $13)
               RETURNING id"#,
        )
        .bind(&req.notice)
        .bind(&req.code)
        .bind(&req.comments)
        .bind(&req.ak_source)
        .bind(&req.cron)
        .bind(req.sort)
        .bind(req.qk_date_segm)
        .bind(req.qk_time_segm)
        .bind(req.qk_progress)
        .bind(req.progress_pct)
        .bind(req.schedule_pct)
        .bind(req.lk_health)
        .bind(user_id)
        .fetch_one(&self.pool)
        .await
        .map_err(ApiError::from)?;

        self.get_refs(id)
            .await?
            .ok_or_else(|| ApiError::NotFound(format!("inbound_order {} not found", id)))
    }

    async fn update(
        &self,
        id: i64,
        req: UpdateInboundOrderRequest,
        user_id: i64,
    ) -> Result<Option<InboundOrder>, ApiError> {
        if self.get_refs(id).await?.is_none() {
            return Ok(None);
        }

        let rows = sqlx::query(
            r#"UPDATE isahl."zc_id_plan-inbound"
               SET                    notice = COALESCE($1, notice),
                   code = COALESCE($2, code),
                   comments = COALESCE($3, comments),
                   ak_source = COALESCE($4, ak_source),
                   cron = COALESCE($5, cron),
                   sort = COALESCE($6, sort),
                   "qk_date-segm" = COALESCE($7, "qk_date-segm"),
                   "qk_time-segm" = COALESCE($8, "qk_time-segm"),
                   qk_progress = COALESCE($9, qk_progress),
                   progress_pct = COALESCE($10, progress_pct),
                   schedule_pct = COALESCE($11, schedule_pct),
                   lk_health = COALESCE($12, lk_health),
                   updated_by_id = $13
               WHERE id = $14 AND deleted_at IS NULL"#,
        )
        .bind(&req.notice)
        .bind(&req.code)
        .bind(&req.comments)
        .bind(&req.ak_source)
        .bind(&req.cron)
        .bind(req.sort)
        .bind(req.qk_date_segm)
        .bind(req.qk_time_segm)
        .bind(req.qk_progress)
        .bind(req.progress_pct)
        .bind(req.schedule_pct)
        .bind(req.lk_health)
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
