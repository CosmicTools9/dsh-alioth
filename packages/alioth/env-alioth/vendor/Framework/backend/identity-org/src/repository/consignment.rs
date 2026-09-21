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
        // 货量/金额（volume/amount）已并入结构写路单一实现：
        // `consignment_writer::update_consignment_structures_tx`（本函数下方 apply_structured_update 调用）。
        // MUST NOT 在本处再写 `zc_id_scal-weight` / `zc_id_scal-amount`（旁路 + 货量 ROUND 缺陷）。

        // ── 结构化写路（change: migrate-consignment-fields-to-structures T10）──────────────
        // 读侧已切结构（明细/停靠/时段/标量），`comments` 摘要不再被解析——业务字段若只并进
        // comments，编辑保存即"写进无人读的列"（功能性回归）。写件单一实现在
        // `consignment-writer::update_consignment_structures_tx`（白名单写件），本处仅薄调用。
        super::consignment_structures::apply_structured_update(&self.pool, id, user_id, &req)
            .await?;

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
