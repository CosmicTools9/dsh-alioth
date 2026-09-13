// TransportTrackingRepository（split 自 repository.rs 单体，④ 候选）
//! 身份实体 Repository — 标准 CRUD 实现
//!
//! Identity 使用自定义 Repository，其余实体组合 GenericRepository，
//! 仅自定义 create/update 的 INSERT/UPDATE SQL。

use async_trait::async_trait;
use common::data::{ListQuery, PaginatedResponse};
use common::AliothError as ApiError;
use crud::repository::AliothRepository;
use crud::GenericRepository;
use sqlx::{AssertSqlSafe, PgPool};

use crate::models::{
    CreateTransportTrackingRequest, TransportTracking, UpdateTransportTrackingRequest,
};

use super::ontology_binding;

// ═══════════════════════════════════════════════
// TransportTracking — "isahl"."zc_id_oper-transport_tracking"
// ═══════════════════════════════════════════════

#[derive(Clone)]
pub struct TransportTrackingRepository {
    generic: GenericRepository<TransportTracking>,
    pool: PgPool,
}

impl TransportTrackingRepository {
    pub fn new(pool: PgPool) -> Self {
        Self {
            generic: GenericRepository::new(pool.clone()),
            pool,
        }
    }
}

impl From<PgPool> for TransportTrackingRepository {
    fn from(pool: PgPool) -> Self {
        Self::new(pool.clone())
    }
}

#[async_trait]
impl
    AliothRepository<
        TransportTracking,
        CreateTransportTrackingRequest,
        UpdateTransportTrackingRequest,
        ApiError,
    > for TransportTrackingRepository
{
    async fn list(
        &self,
        query: &ListQuery,
    ) -> Result<PaginatedResponse<TransportTracking>, ApiError> {
        self.generic.list_refs(query).await
    }

    async fn get(&self, id: i64) -> Result<Option<TransportTracking>, ApiError> {
        self.generic.get_refs(id, None).await
    }

    async fn create(
        &self,
        req: CreateTransportTrackingRequest,
        user_id: i64,
    ) -> Result<TransportTracking, ApiError> {
        let (dk_scene, dk_factor, dk_function) =
            ontology_binding::resolve(&self.pool, "TransportTracking").await?;
        // fix-fk-approve-residual-consumers：fk_approve 物理列已移除——
        // 创建后写 rr_event 桥行承载审批事件关联，RETURNING 用基表查询补派生值
        let created: TransportTracking = sqlx::query_as::<_, TransportTracking>(
            r#"INSERT INTO "isahl"."zc_id_oper-transport_tracking" (code, notice, comments, fk_operator, fk_subject, qk_arrived, qk_work_duration, "ck_cate-wh", "ck_cate-biz", created_by_id, dk_scene, dk_factor, dk_function)
               VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, $13)
               RETURNING id, code, notice, comments, fk_operator, fk_subject, NULL::bigint AS fk_approve, qk_arrived, qk_work_duration, "ck_cate-wh", "ck_cate-biz", created_at, updated_at, deleted_at"#,
        )
        .bind(&req.code).bind(&req.notice).bind(&req.comments)
        .bind(req.fk_operator).bind(req.fk_subject)
        .bind(req.qk_arrived).bind(req.qk_work_duration)
        .bind(req.ck_cate_wh).bind(req.ck_cate_biz)
        .bind(user_id)
        .bind(dk_scene).bind(dk_factor).bind(dk_function)
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
            .bind(created.id)
            .bind(ev)
            .bind(user_id)
            .execute(&self.pool)
            .await
            .map_err(ApiError::from)?;
        }
        // fk_approve 为桥派生值——桥行已落库，回显请求值即可（与 get/get_refs 派生同口径）
        if req.fk_approve.is_some() {
            return Ok(TransportTracking {
                fk_approve: req.fk_approve,
                ..created
            });
        }
        Ok(created)
    }

    async fn update(
        &self,
        id: i64,
        req: UpdateTransportTrackingRequest,
        user_id: i64,
    ) -> Result<Option<TransportTracking>, ApiError> {
        let mut sets = Vec::new();
        let mut idx: usize = 0;

        if req.code.is_some() {
            idx += 1;
            sets.push(format!("code = ${}", idx));
        }
        if req.notice.is_some() {
            idx += 1;
            sets.push(format!("notice = ${}", idx));
        }
        if req.comments.is_some() {
            idx += 1;
            sets.push(format!("comments = ${}", idx));
        }

        if req.fk_operator.is_some() {
            idx += 1;
            sets.push(format!("fk_operator = ${}", idx));
        }
        if req.fk_subject.is_some() {
            idx += 1;
            sets.push(format!("fk_subject = ${}", idx));
        }
        if req.qk_arrived.is_some() {
            idx += 1;
            sets.push(format!("qk_arrived = ${}", idx));
        }
        if req.qk_work_duration.is_some() {
            idx += 1;
            sets.push(format!("qk_work_duration = ${}", idx));
        }
        if req.ck_cate_wh.is_some() {
            idx += 1;
            sets.push(format!(r#""ck_cate-wh" = ${}"#, idx));
        }
        if req.ck_cate_biz.is_some() {
            idx += 1;
            sets.push(format!(r#""ck_cate-biz" = ${}"#, idx));
        }

        if sets.is_empty() {
            return self.get(id).await;
        }

        sets.push("updated_at = NOW()".into());
        idx += 1;
        sets.push(format!("updated_by_id = ${}", idx));
        let id_param = idx + 1;

        let sql = format!(
            r#"UPDATE "isahl"."zc_id_oper-transport_tracking" SET {} WHERE id = ${} AND deleted_at IS NULL
               RETURNING id, code, notice, comments, fk_operator, fk_subject, NULL::bigint AS fk_approve, qk_arrived, qk_work_duration, "ck_cate-wh", "ck_cate-biz", created_at, updated_at, deleted_at"#,
            sets.join(", "),
            id_param
        );

        let mut q = sqlx::query_as::<_, TransportTracking>(AssertSqlSafe(sql.as_str()));
        if let Some(ref v) = req.code {
            q = q.bind(v);
        }
        if let Some(ref v) = req.notice {
            q = q.bind(v);
        }
        if let Some(ref v) = req.comments {
            q = q.bind(v);
        }

        if let Some(v) = req.fk_operator {
            q = q.bind(v);
        }
        if let Some(v) = req.fk_subject {
            q = q.bind(v);
        }
        if let Some(ref v) = req.qk_arrived {
            q = q.bind(v);
        }
        if let Some(ref v) = req.qk_work_duration {
            q = q.bind(v);
        }
        if let Some(ref v) = req.ck_cate_wh {
            q = q.bind(v);
        }
        if let Some(ref v) = req.ck_cate_biz {
            q = q.bind(v);
        }
        q = q.bind(user_id);
        q = q.bind(id);

        let row = q.fetch_optional(&self.pool).await.map_err(ApiError::from)?;
        // fix-fk-approve-residual-consumers：审批事件关联改 rr_event 桥——
        // 有值时软删旧桥行后重建（update 带 fk_approve 即重绑）
        if row.is_some() {
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
                // 返回桥派生后的真实行（fk_approve 经 rr_event 子查询派生）
                return self.get(id).await;
            }
        }
        Ok(row)
    }

    async fn delete(&self, id: i64, user_id: i64) -> Result<(), ApiError> {
        self.generic.delete(id, user_id).await
    }
}
