//! SalesOrder Repository — isahl."zc_id_oper-sales_order" CRUD（crud 模式：GenericRepository 委托读侧/删除）。
use async_trait::async_trait;
use common::AliothError as ApiError;
use crud::{AliothRepository, GenericRepository, ListQuery, PaginatedResponse};
use sqlx::PgPool;

use crate::models::{CreateSalesOrderRequest, SalesOrder, UpdateSalesOrderRequest};

#[derive(Clone)]
pub struct SalesOrderRepository {
    generic: GenericRepository<SalesOrder>,
    pool: PgPool,
}

impl SalesOrderRepository {
    pub fn new(pool: PgPool) -> Self {
        Self {
            generic: GenericRepository::new(pool.clone()),
            pool,
        }
    }

    /// 按 ID 读取（含 _refs 解析）
    pub async fn get_refs(&self, id: i64) -> Result<Option<SalesOrder>, ApiError> {
        self.generic.get_refs(id, None).await
    }
}

#[async_trait]
impl AliothRepository<SalesOrder, CreateSalesOrderRequest, UpdateSalesOrderRequest, ApiError>
    for SalesOrderRepository
{
    async fn list(&self, query: &ListQuery) -> Result<PaginatedResponse<SalesOrder>, ApiError> {
        self.generic.list_refs(query).await
    }

    async fn get(&self, id: i64) -> Result<Option<SalesOrder>, ApiError> {
        self.generic.get_refs(id, None).await
    }

    async fn create(
        &self,
        req: CreateSalesOrderRequest,
        user_id: i64,
    ) -> Result<SalesOrder, ApiError> {
        // fix-fk-approve-residual-consumers：fk_approve 物理列已移除——
        // 创建后写 rr_event 桥行承载审批事件关联
        let id: i64 = sqlx::query_scalar(
            r#"INSERT INTO isahl."zc_id_oper-sales_order"
               (notice, code, comments, ak_source, tk_version, tk_batch_no, fk_previous, ck_branch, qk_work_duration, fk_operator, fk_subject, qk_period, "ck_cate-wh", "sk_unit-working", qk_arrived, "ck_cate-biz", qk_sla, lk_urgent, "ck_cate-proc_op", created_by_id)
               VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, $13, $14, $15, $16, $17, $18, $19, $20)
               RETURNING id"#,
        )
        .bind(&req.notice)
        .bind(&req.code)
        .bind(&req.comments)
        .bind(&req.ak_source)
        .bind(req.tk_version)
        .bind(req.tk_batch_no)
        .bind(req.fk_previous)
        .bind(req.ck_branch)
        .bind(req.qk_work_duration)
        .bind(req.fk_operator)
        .bind(req.fk_subject)
        .bind(req.qk_period)
        .bind(req.ck_cate_wh)
        .bind(req.sk_unit_working)
        .bind(req.qk_arrived)
        .bind(req.ck_cate_biz)
        .bind(req.qk_sla)
        .bind(req.lk_urgent)
        .bind(req.ck_cate_proc_op)
        .bind(user_id)
        .fetch_one(&self.pool)
        .await
        .map_err(ApiError::from)?;

        if let Some(ev) = req.fk_approve {
            sqlx::query(
                r#"INSERT INTO isahl.zc_id_operation_rr_event (id, ref_left, ref_right, created_by_id)
                   SELECT isahl.gen_next_zuid(), $1, $2, $3
                   WHERE NOT EXISTS (
                       SELECT 1 FROM isahl.zc_id_operation_rr_event rr
                       WHERE rr.ref_left = $1 AND rr.ref_right = $2 AND rr.deleted_at IS NULL
                   )"#,
            )
            .bind(id)
            .bind(ev)
            .bind(user_id)
            .execute(&self.pool)
            .await
            .map_err(ApiError::from)?;
        }

        self.get_refs(id)
            .await?
            .ok_or_else(|| ApiError::NotFound(format!("sales_order {} not found", id)))
    }

    async fn update(
        &self,
        id: i64,
        req: UpdateSalesOrderRequest,
        user_id: i64,
    ) -> Result<Option<SalesOrder>, ApiError> {
        if self.get_refs(id).await?.is_none() {
            return Ok(None);
        }

        let rows = sqlx::query(
            r#"UPDATE isahl."zc_id_oper-sales_order"
               SET                    notice = COALESCE($1, notice),
                   code = COALESCE($2, code),
                   comments = COALESCE($3, comments),
                   ak_source = COALESCE($4, ak_source),
                   tk_version = COALESCE($5, tk_version),
                   tk_batch_no = COALESCE($6, tk_batch_no),
                   fk_previous = COALESCE($7, fk_previous),
                   ck_branch = COALESCE($8, ck_branch),
                   qk_work_duration = COALESCE($9, qk_work_duration),
                   fk_operator = COALESCE($10, fk_operator),
                   fk_subject = COALESCE($11, fk_subject),
                   qk_period = COALESCE($12, qk_period),
                   "ck_cate-wh" = COALESCE($13, "ck_cate-wh"),
                   "sk_unit-working" = COALESCE($14, "sk_unit-working"),
                   qk_arrived = COALESCE($15, qk_arrived),
                   "ck_cate-biz" = COALESCE($16, "ck_cate-biz"),
                   qk_sla = COALESCE($17, qk_sla),
                   lk_urgent = COALESCE($18, lk_urgent),
                   "ck_cate-proc_op" = COALESCE($19, "ck_cate-proc_op"),
                   updated_by_id = $20
               WHERE id = $21 AND deleted_at IS NULL"#,
        )
        .bind(&req.notice)
        .bind(&req.code)
        .bind(&req.comments)
        .bind(&req.ak_source)
        .bind(req.tk_version)
        .bind(req.tk_batch_no)
        .bind(req.fk_previous)
        .bind(req.ck_branch)
        .bind(req.qk_work_duration)
        .bind(req.fk_operator)
        .bind(req.fk_subject)
        .bind(req.qk_period)
        .bind(req.ck_cate_wh)
        .bind(req.sk_unit_working)
        .bind(req.qk_arrived)
        .bind(req.ck_cate_biz)
        .bind(req.qk_sla)
        .bind(req.lk_urgent)
        .bind(req.ck_cate_proc_op)
        .bind(user_id)
        .bind(id)
        .execute(&self.pool)
        .await
        .map_err(ApiError::from)?;
        if rows.rows_affected() == 0 {
            return Ok(None);
        }

        // fix-fk-approve-residual-consumers：审批事件关联改 rr_event 桥——
        // update 带 fk_approve 即重绑：软删旧桥行后重建
        if let Some(ev) = req.fk_approve {
            sqlx::query(
                r#"UPDATE isahl.zc_id_operation_rr_event SET deleted_at = NOW()
                   WHERE ref_left = $1 AND deleted_at IS NULL"#,
            )
            .bind(id)
            .execute(&self.pool)
            .await
            .map_err(ApiError::from)?;
            sqlx::query(
                r#"INSERT INTO isahl.zc_id_operation_rr_event (id, ref_left, ref_right, created_by_id)
                   VALUES (isahl.gen_next_zuid(), $1, $2, $3)"#,
            )
            .bind(id)
            .bind(ev)
            .bind(user_id)
            .execute(&self.pool)
            .await
            .map_err(ApiError::from)?;
        }

        self.get_refs(id).await
    }

    async fn delete(&self, id: i64, user_id: i64) -> Result<(), ApiError> {
        self.generic.delete(id, user_id).await
    }
}
