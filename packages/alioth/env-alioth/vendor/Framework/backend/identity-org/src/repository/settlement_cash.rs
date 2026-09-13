// SettlementCashRepository（split 自 repository.rs 单体，④ 候选）
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

use crate::models::{CreateSettlementCashRequest, SettlementCash, UpdateSettlementCashRequest};

use super::ontology_binding;

// ═══════════════════════════════════════════════
// SettlementCash — "isahl"."zc_id_stat-smt-cash"
// ═══════════════════════════════════════════════

#[derive(Clone)]
pub struct SettlementCashRepository {
    generic: GenericRepository<SettlementCash>,
    pool: PgPool,
}

impl SettlementCashRepository {
    pub fn new(pool: PgPool) -> Self {
        Self {
            generic: GenericRepository::new(pool.clone()),
            pool,
        }
    }
}

impl From<PgPool> for SettlementCashRepository {
    fn from(pool: PgPool) -> Self {
        Self::new(pool.clone())
    }
}

#[async_trait]
impl
    AliothRepository<
        SettlementCash,
        CreateSettlementCashRequest,
        UpdateSettlementCashRequest,
        ApiError,
    > for SettlementCashRepository
{
    async fn list(&self, query: &ListQuery) -> Result<PaginatedResponse<SettlementCash>, ApiError> {
        self.generic.list_refs(query).await
    }

    async fn get(&self, id: i64) -> Result<Option<SettlementCash>, ApiError> {
        self.generic.get_refs(id, None).await
    }

    async fn create(
        &self,
        req: CreateSettlementCashRequest,
        user_id: i64,
    ) -> Result<SettlementCash, ApiError> {
        let (dk_scene, dk_factor, dk_function) =
            ontology_binding::resolve(&self.pool, "SettlementCash").await?;
        sqlx::query_as::<_, SettlementCash>(
            r#"INSERT INTO "isahl"."zc_id_stat-smt-cash" (qk_date, qk_income, qk_outgo, qk_amount, created_by_id, dk_scene, dk_factor, dk_function)
               VALUES ($1, $2, $3, $4, $5, $6, $7, $8)
               RETURNING id, qk_date, qk_income, qk_outgo, qk_amount, created_at, updated_at, deleted_at"#,
        )
        .bind(req.qk_date).bind(req.qk_income).bind(req.qk_outgo)
        .bind(req.qk_amount)
        .bind(user_id)
        .bind(dk_scene).bind(dk_factor).bind(dk_function)
        .fetch_one(&self.pool)
        .await
        .map_err(ApiError::from)
    }

    async fn update(
        &self,
        id: i64,
        req: UpdateSettlementCashRequest,
        user_id: i64,
    ) -> Result<Option<SettlementCash>, ApiError> {
        let mut sets = Vec::new();
        let mut idx: usize = 0;

        if req.qk_date.is_some() {
            idx += 1;
            sets.push(format!("qk_date = ${}", idx));
        }
        if req.qk_income.is_some() {
            idx += 1;
            sets.push(format!("qk_income = ${}", idx));
        }
        if req.qk_outgo.is_some() {
            idx += 1;
            sets.push(format!("qk_outgo = ${}", idx));
        }

        if sets.is_empty() {
            return self.get(id).await;
        }

        sets.push("updated_at = NOW()".into());
        idx += 1;
        sets.push(format!("updated_by_id = ${}", idx));
        let id_param = idx + 1;

        let sql = format!(
            r#"UPDATE "isahl"."zc_id_stat-smt-cash" SET {} WHERE id = ${} AND deleted_at IS NULL
               RETURNING id, qk_date, qk_income, qk_outgo, qk_amount, created_at, updated_at, deleted_at"#,
            sets.join(", "),
            id_param
        );

        let mut q = sqlx::query_as::<_, SettlementCash>(AssertSqlSafe(sql.as_str()));
        if let Some(ref v) = req.qk_date {
            q = q.bind(v);
        }
        if let Some(ref v) = req.qk_income {
            q = q.bind(v);
        }
        if let Some(ref v) = req.qk_outgo {
            q = q.bind(v);
        }
        if let Some(ref v) = req.qk_amount {
            q = q.bind(v);
        }
        q = q.bind(user_id);
        q = q.bind(id);

        q.fetch_optional(&self.pool).await.map_err(ApiError::from)
    }

    async fn delete(&self, id: i64, user_id: i64) -> Result<(), ApiError> {
        self.generic.delete(id, user_id).await
    }
}
