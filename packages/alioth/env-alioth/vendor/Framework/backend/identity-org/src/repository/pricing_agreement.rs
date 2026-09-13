// PricingAgreementRepository（split 自 repository.rs 单体，④ 候选）
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
    CreatePricingAgreementRequest, PricingAgreement, UpdatePricingAgreementRequest,
};

use super::ontology_binding;

// ═══════════════════════════════════════════════
// PricingAgreement Repository — "isahl"."zc_id_agre-pricing"
// ═══════════════════════════════════════════════

#[derive(Clone)]
pub struct PricingAgreementRepository {
    generic: GenericRepository<PricingAgreement>,
    pool: PgPool,
}

impl PricingAgreementRepository {
    pub fn new(pool: PgPool) -> Self {
        Self {
            generic: GenericRepository::new(pool.clone()),
            pool,
        }
    }
}

impl From<PgPool> for PricingAgreementRepository {
    fn from(pool: PgPool) -> Self {
        Self::new(pool.clone())
    }
}

#[async_trait]
impl
    AliothRepository<
        PricingAgreement,
        CreatePricingAgreementRequest,
        UpdatePricingAgreementRequest,
        ApiError,
    > for PricingAgreementRepository
{
    async fn list(
        &self,
        query: &ListQuery,
    ) -> Result<PaginatedResponse<PricingAgreement>, ApiError> {
        self.generic.list_refs(query).await
    }

    async fn get(&self, id: i64) -> Result<Option<PricingAgreement>, ApiError> {
        self.generic.get_refs(id, None).await
    }

    async fn create(
        &self,
        req: CreatePricingAgreementRequest,
        user_id: i64,
    ) -> Result<PricingAgreement, ApiError> {
        let (dk_scene, dk_factor, dk_function) =
            ontology_binding::resolve(&self.pool, "PricingAgreement").await?;
        sqlx::query_as::<_, PricingAgreement>(
            r#"INSERT INTO "isahl"."zc_id_agre-pricing" (code, notice, comments, created_by_id, dk_scene, dk_factor, dk_function)
               VALUES ($1, $2, $3, $4, $5, $6, $7)
               RETURNING id, code, notice, comments, t_color_, tpl_id, created_at, updated_at, deleted_at"#,
        )
        .bind(&req.code)
        .bind(&req.notice)
        .bind(&req.comments)
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
        req: UpdatePricingAgreementRequest,
        _user_id: i64,
    ) -> Result<Option<PricingAgreement>, ApiError> {
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
        if sets.is_empty() {
            return self.get(id).await;
        }
        sets.push("updated_at = NOW()".into());
        idx += 1;
        sets.push(format!("updated_by_id = ${}", idx));
        let id_param = idx + 1;
        let sql = format!(
            r#"UPDATE "isahl"."zc_id_agre-pricing" SET {} WHERE id = ${} AND deleted_at IS NULL
               RETURNING id, code, notice, comments, t_color_, tpl_id, created_at, updated_at, deleted_at"#,
            sets.join(", "),
            id_param
        );
        let mut q = sqlx::query_as::<_, PricingAgreement>(AssertSqlSafe(sql.as_str()));
        if let Some(ref v) = req.code {
            q = q.bind(v);
        }
        if let Some(ref v) = req.notice {
            q = q.bind(v);
        }
        if let Some(ref v) = req.comments {
            q = q.bind(v);
        }
        q = q.bind(_user_id);
        q = q.bind(id);
        q.fetch_optional(&self.pool).await.map_err(ApiError::from)
    }

    async fn delete(&self, id: i64, user_id: i64) -> Result<(), ApiError> {
        self.generic.delete(id, user_id).await
    }
}
