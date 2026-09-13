// SettlementBankRepository（split 自 repository.rs 单体，④ 候选）
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

use crate::models::{CreateSettlementBankRequest, SettlementBank, UpdateSettlementBankRequest};

use super::ontology_binding;

// ═══════════════════════════════════════════════
// SettlementBank — "isahl"."zc_id_stat-smt-bank"
// ═══════════════════════════════════════════════

#[derive(Clone)]
pub struct SettlementBankRepository {
    generic: GenericRepository<SettlementBank>,
    pool: PgPool,
}

impl SettlementBankRepository {
    pub fn new(pool: PgPool) -> Self {
        Self {
            generic: GenericRepository::new(pool.clone()),
            pool,
        }
    }
}

impl From<PgPool> for SettlementBankRepository {
    fn from(pool: PgPool) -> Self {
        Self::new(pool.clone())
    }
}

#[async_trait]
impl
    AliothRepository<
        SettlementBank,
        CreateSettlementBankRequest,
        UpdateSettlementBankRequest,
        ApiError,
    > for SettlementBankRepository
{
    async fn list(&self, query: &ListQuery) -> Result<PaginatedResponse<SettlementBank>, ApiError> {
        self.generic.list_refs(query).await
    }

    async fn get(&self, id: i64) -> Result<Option<SettlementBank>, ApiError> {
        self.generic.get_refs(id, None).await
    }

    async fn create(
        &self,
        req: CreateSettlementBankRequest,
        user_id: i64,
    ) -> Result<SettlementBank, ApiError> {
        let (dk_scene, dk_factor, dk_function) =
            ontology_binding::resolve(&self.pool, "SettlementBank").await?;
        sqlx::query_as::<_, SettlementBank>(
            r#"INSERT INTO "isahl"."zc_id_stat-smt-bank" (qk_date, qk_income, qk_outgo, qk_balance, "qk_exchange-rate", created_by_id, dk_scene, dk_factor, dk_function)
               VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9)
               RETURNING id, qk_date, qk_income, qk_outgo, qk_balance, "qk_exchange-rate", created_at, updated_at, deleted_at"#,
        )
        .bind(req.qk_date).bind(req.qk_income).bind(req.qk_outgo)
        .bind(req.qk_balance).bind(req.qk_exchange_rate)
        .bind(user_id)
        .bind(dk_scene).bind(dk_factor).bind(dk_function)
        .fetch_one(&self.pool)
        .await
        .map_err(ApiError::from)
    }

    async fn update(
        &self,
        id: i64,
        req: UpdateSettlementBankRequest,
        user_id: i64,
    ) -> Result<Option<SettlementBank>, ApiError> {
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
        if req.qk_balance.is_some() {
            idx += 1;
            sets.push(format!("qk_balance = ${}", idx));
        }
        if req.qk_exchange_rate.is_some() {
            idx += 1;
            sets.push(format!("qk_exchange-rate = ${}", idx));
        }

        if sets.is_empty() {
            return self.get(id).await;
        }

        sets.push("updated_at = NOW()".into());
        idx += 1;
        sets.push(format!("updated_by_id = ${}", idx));
        let id_param = idx + 1;

        let sql = format!(
            r#"UPDATE "isahl"."zc_id_stat-smt-bank" SET {} WHERE id = ${} AND deleted_at IS NULL
               RETURNING id, qk_date, qk_income, qk_outgo, qk_balance, "qk_exchange-rate", created_at, updated_at, deleted_at"#,
            sets.join(", "),
            id_param
        );

        let mut q = sqlx::query_as::<_, SettlementBank>(AssertSqlSafe(sql.as_str()));
        if let Some(ref v) = req.qk_date {
            q = q.bind(v);
        }
        if let Some(ref v) = req.qk_income {
            q = q.bind(v);
        }
        if let Some(ref v) = req.qk_outgo {
            q = q.bind(v);
        }
        if let Some(ref v) = req.qk_balance {
            q = q.bind(v);
        }
        if let Some(ref v) = req.qk_exchange_rate {
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
