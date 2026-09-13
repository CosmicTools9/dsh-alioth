// ConsignmentRepository（split 自 repository.rs 单体，④ 候选）
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

use crate::models::{Consignment, CreateConsignmentRequest, UpdateConsignmentRequest};

use super::ontology_binding;

// ═══════════════════════════════════════════════
// Consignment — "isahl"."zc_id_orde-land"
// ═══════════════════════════════════════════════

#[derive(Clone)]
pub struct ConsignmentRepository {
    generic: GenericRepository<Consignment>,
    pool: PgPool,
}

impl ConsignmentRepository {
    pub fn new(pool: PgPool) -> Self {
        Self {
            generic: GenericRepository::new(pool.clone()),
            pool,
        }
    }

    /// traffic_line_id 此前寄生于产品 comments JSON，已随 comments 文本化失效
    /// （模型无承载列）——绑定能力停用，显式拒绝而非静默丢数据。
    async fn apply_traffic_line(
        &self,
        _consignment_id: i64,
        traffic_line_id: Option<i64>,
    ) -> Result<(), ApiError> {
        if traffic_line_id.unwrap_or(0) > 0 {
            return Err(ApiError::Validation {
                field: "traffic_line_id".into(),
                message: "运输线路绑定已停用（comments 已文本化，模型无承载列）".into(),
            });
        }
        Ok(())
    }
}

impl From<PgPool> for ConsignmentRepository {
    fn from(pool: PgPool) -> Self {
        Self::new(pool.clone())
    }
}

#[async_trait]
impl AliothRepository<Consignment, CreateConsignmentRequest, UpdateConsignmentRequest, ApiError>
    for ConsignmentRepository
{
    async fn list(&self, query: &ListQuery) -> Result<PaginatedResponse<Consignment>, ApiError> {
        self.generic.list_refs(query).await
    }
    async fn get(&self, id: i64) -> Result<Option<Consignment>, ApiError> {
        self.generic.get_refs(id, None).await
    }

    async fn create(
        &self,
        req: CreateConsignmentRequest,
        user_id: i64,
    ) -> Result<Consignment, ApiError> {
        let (dk_scene, dk_factor, dk_function) =
            ontology_binding::resolve(&self.pool, "Consignment").await?;
        sqlx::query_as::<_, Consignment>(
            r#"INSERT INTO "isahl"."zc_id_orde-land" (code, notice, comments, fk_subject, fk_object, fk_contract, qk_date, sk_currency, ck_category, created_by_id, dk_scene, dk_factor, dk_function)
               VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, $13)
               RETURNING id, code, notice, comments, fk_subject, fk_object, fk_contract, qk_date, sk_currency, ck_category, created_at, updated_at, deleted_at"#,
        )
        .bind(&req.code).bind(&req.notice).bind(&req.comments)
        .bind(req.fk_subject).bind(req.fk_object).bind(req.fk_contract)
        .bind(req.qk_date)
        .bind(req.sk_currency).bind(req.ck_category)
        .bind(user_id)
        .bind(dk_scene)
        .bind(dk_factor)
        .bind(dk_function)

        .fetch_one(&self.pool)
        .await
        .map_err(ApiError::from)
    }

    async fn update(
        &self,
        id: i64,
        req: UpdateConsignmentRequest,
        user_id: i64,
    ) -> Result<Option<Consignment>, ApiError> {
        let mut sets = Vec::new();
        let mut idx: usize = 0;
        // 批注 2026-08-21：编辑货量（volume 数值吨）→ 更新 deta-trade_order.qk_w_qty 指向的标量真值。
        // 两段式独立语句（不参与下方 sets 占位符编号）——旧实现把新建标量 id 推入
        // sets 头部却在绑定尾部追加，占位符与绑定序错位（新建标量+其他字段并存必 500）
        if let Some(v) = req.volume {
            let cur_qk: Option<i64> = sqlx::query_scalar(
                r#"SELECT qk_w_qty FROM "isahl"."zc_id_deta-trade_order"
                   WHERE fk_list = $1 AND deleted_at IS NULL LIMIT 1"#,
            )
            .bind(id)
            .fetch_optional(&self.pool)
            .await?
            .flatten();
            match cur_qk {
                Some(scale_id) => {
                    // 批注 2026-08-21：重量/货量存储约束为整数——ROUND 兜底
                    sqlx::query(
                        r#"UPDATE "isahl"."zc_id_scal-weight" SET mark = ROUND($1::numeric), notice = $2 WHERE id = $3"#,
                    )
                    .bind(v)
                    .bind(format!("{}吨", v))
                    .bind(scale_id)
                    .execute(&self.pool)
                    .await?;
                }
                None => {
                    let scale_id: i64 = sqlx::query_scalar(
                        r#"INSERT INTO "isahl"."zc_id_scal-weight" (id, code, notice, mark, created_by_id)
                           VALUES (isahl.gen_next_uid(), $1, $2, ROUND($3::numeric), $4) RETURNING id"#,
                    )
                    .bind(format!("WT-{}", chrono::Utc::now().timestamp()))
                    .bind(format!("{}吨", v))
                    .bind(v)
                    .bind(user_id)
                    .fetch_one(&self.pool)
                    .await?;
                    sqlx::query(
                        r#"UPDATE "isahl"."zc_id_deta-trade_order" SET qk_w_qty = $1, updated_by_id = $2, updated_at = NOW()
                           WHERE fk_list = $3 AND deleted_at IS NULL"#,
                    )
                    .bind(scale_id)
                    .bind(user_id)
                    .bind(id)
                    .execute(&self.pool)
                    .await?;
                }
            }
        }
        // 批注 a2bd97b6：编辑运费（amount 数值）→ 更新 qk_amount 指向的 scal-amount
        // 标量真值（金额保留小数，不 ROUND）；qk_amount 为空则新建标量并回挂
        if let Some(v) = req.amount {
            let cur_qk: Option<i64> = sqlx::query_scalar(
                r#"SELECT qk_amount FROM "isahl"."zc_id_deta-trade_order"
                   WHERE fk_list = $1 AND deleted_at IS NULL LIMIT 1"#,
            )
            .bind(id)
            .fetch_optional(&self.pool)
            .await?
            .flatten();
            match cur_qk {
                Some(scale_id) => {
                    sqlx::query(
                        r#"UPDATE "isahl"."zc_id_scal-amount" SET mark = ROUND($1::numeric, 2), notice = $2 WHERE id = $3"#,
                    )
                    .bind(v)
                    .bind(format!("{}元", v))
                    .bind(scale_id)
                    .execute(&self.pool)
                    .await?;
                }
                None => {
                    let scale_id: i64 = sqlx::query_scalar(
                        r#"INSERT INTO "isahl"."zc_id_scal-amount" (id, code, notice, mark, created_by_id)
                           VALUES (isahl.gen_next_uid(), $1, $2, ROUND($3::numeric, 2), $4) RETURNING id"#,
                    )
                    .bind(format!("AMT-{}", chrono::Utc::now().timestamp()))
                    .bind(format!("{}元", v))
                    .bind(v)
                    .bind(user_id)
                    .fetch_one(&self.pool)
                    .await?;
                    sqlx::query(
                        r#"UPDATE "isahl"."zc_id_deta-trade_order" SET qk_amount = $1, updated_by_id = $2, updated_at = NOW()
                           WHERE fk_list = $3 AND deleted_at IS NULL"#,
                    )
                    .bind(scale_id)
                    .bind(user_id)
                    .bind(id)
                    .execute(&self.pool)
                    .await?;
                }
            }
        }

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
        if req.fk_subject.is_some() {
            idx += 1;
            sets.push(format!("fk_subject = ${}", idx));
        }
        if req.fk_object.is_some() {
            idx += 1;
            sets.push(format!("fk_object = ${}", idx));
        }
        if let Some(fc) = req.fk_contract {
            if fc == 0 {
                // 清除哨兵：0 -> 置 NULL（不占位、不绑定）
                sets.push("fk_contract = NULL".into());
            } else {
                idx += 1;
                sets.push(format!("fk_contract = ${}", idx));
            }
        }

        if req.qk_date.is_some() {
            idx += 1;
            sets.push(format!("qk_date = ${}", idx));
        }
        if req.sk_currency.is_some() {
            idx += 1;
            sets.push(format!("sk_currency = ${}", idx));
        }
        if req.ck_category.is_some() {
            idx += 1;
            sets.push(format!("ck_category = ${}", idx));
        }

        if sets.is_empty() {
            // 仅透传 traffic_line_id（改写创建期产品 comments）时主表无字段可更新
            self.apply_traffic_line(id, req.traffic_line_id).await?;
            return self.get(id).await;
        }

        sets.push("updated_at = NOW()".into());
        idx += 1;
        sets.push(format!("updated_by_id = ${}", idx));
        let id_param = idx + 1;

        let sql = format!(
            r#"UPDATE "isahl"."zc_id_orde-land" SET {} WHERE id = ${} AND deleted_at IS NULL
               RETURNING id, code, notice, comments, fk_subject, fk_object, fk_contract, qk_date, sk_currency, ck_category, created_at, updated_at, deleted_at"#,
            sets.join(", "),
            id_param
        );

        let mut q = sqlx::query_as::<_, Consignment>(AssertSqlSafe(sql.as_str()));
        if let Some(ref v) = req.code {
            q = q.bind(v);
        }
        if let Some(ref v) = req.notice {
            q = q.bind(v);
        }
        if let Some(ref v) = req.comments {
            q = q.bind(v);
        }
        if let Some(v) = req.fk_subject {
            q = q.bind(v);
        }
        if let Some(v) = req.fk_object {
            q = q.bind(v);
        }
        if let Some(v) = req.fk_contract {
            if v != 0 {
                q = q.bind(v);
            }
        }
        if let Some(ref v) = req.qk_date {
            q = q.bind(v);
        }
        if let Some(v) = req.sk_currency {
            q = q.bind(v);
        }
        if let Some(ref v) = req.ck_category {
            q = q.bind(v);
        }

        q = q.bind(user_id);
        q = q.bind(id);

        let updated = q.fetch_optional(&self.pool).await.map_err(ApiError::from)?;
        if updated.is_some() {
            self.apply_traffic_line(id, req.traffic_line_id).await?;
        }
        Ok(updated)
    }

    async fn delete(&self, id: i64, user_id: i64) -> Result<(), ApiError> {
        self.generic.delete(id, user_id).await
    }
}
