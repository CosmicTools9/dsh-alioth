// BillCheckRepository（split 自 repository.rs 单体，④ 候选）
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

use crate::models::{BillCheck, CreateBillCheckRequest, UpdateBillCheckRequest};

use super::ontology_binding;

// ═══════════════════════════════════════════════
// BillCheck — "isahl"."zc_id_bill-check"
// ═══════════════════════════════════════════════

#[derive(Clone)]
pub struct BillCheckRepository {
    generic: GenericRepository<BillCheck>,
    pool: PgPool,
}

impl BillCheckRepository {
    pub fn new(pool: PgPool) -> Self {
        Self {
            generic: GenericRepository::new(pool.clone()),
            pool,
        }
    }
}

impl From<PgPool> for BillCheckRepository {
    fn from(pool: PgPool) -> Self {
        Self::new(pool.clone())
    }
}

#[async_trait]
impl AliothRepository<BillCheck, CreateBillCheckRequest, UpdateBillCheckRequest, ApiError>
    for BillCheckRepository
{
    async fn list(&self, query: &ListQuery) -> Result<PaginatedResponse<BillCheck>, ApiError> {
        self.generic.list_refs(query).await
    }

    async fn get(&self, id: i64) -> Result<Option<BillCheck>, ApiError> {
        self.generic.get_refs(id, None).await
    }

    async fn create(
        &self,
        req: CreateBillCheckRequest,
        user_id: i64,
    ) -> Result<BillCheck, ApiError> {
        let (dk_scene, dk_factor, dk_function) =
            ontology_binding::resolve(&self.pool, "BillCheck").await?;
        sqlx::query_as::<_, BillCheck>(
            r#"INSERT INTO "isahl"."zc_id_bill-check" (code, notice, comments, fk_settle, fk_account, qk_amount, "qk_write-off", qk_tax, sk_currency, created_by_id, dk_scene, dk_factor, dk_function)
               VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, $13)
               RETURNING id, code, notice, comments, fk_settle, fk_account, qk_amount, "qk_write-off" AS qk_write_off, qk_tax, sk_currency, created_at, updated_at, deleted_at"#,
        )
        .bind(&req.code).bind(&req.notice).bind(&req.comments)
        .bind(req.fk_settle).bind(req.fk_account)
        .bind(req.qk_amount).bind(req.qk_write_off).bind(req.qk_tax)
        .bind(req.sk_currency)
        .bind(user_id)
        .bind(dk_scene).bind(dk_factor).bind(dk_function)
        .fetch_one(&self.pool)
        .await
        .map_err(ApiError::from)
    }

    async fn update(
        &self,
        id: i64,
        req: UpdateBillCheckRequest,
        user_id: i64,
    ) -> Result<Option<BillCheck>, ApiError> {
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
        if req.fk_settle.is_some() {
            idx += 1;
            sets.push(format!("fk_settle = ${}", idx));
        }
        if req.fk_account.is_some() {
            idx += 1;
            sets.push(format!("fk_account = ${}", idx));
        }
        if req.qk_amount.is_some() {
            idx += 1;
            sets.push(format!("qk_amount = ${}", idx));
        }
        if req.qk_write_off.is_some() {
            idx += 1;
            sets.push(format!("\"qk_write-off\" = ${}", idx));
        }
        if req.qk_tax.is_some() {
            idx += 1;
            sets.push(format!("qk_tax = ${}", idx));
        }
        if req.sk_currency.is_some() {
            idx += 1;
            sets.push(format!("sk_currency = ${}", idx));
        }

        if sets.is_empty() {
            return self.get(id).await;
        }

        sets.push("updated_at = NOW()".into());
        idx += 1;
        sets.push(format!("updated_by_id = ${}", idx));
        let id_param = idx + 1;

        let sql = format!(
            r#"UPDATE "isahl"."zc_id_bill-check" SET {} WHERE id = ${} AND deleted_at IS NULL
               RETURNING id, code, notice, comments, fk_settle, fk_account, qk_amount, "qk_write-off" AS qk_write_off, qk_tax, sk_currency, created_at, updated_at, deleted_at"#,
            sets.join(", "),
            id_param
        );

        let mut q = sqlx::query_as::<_, BillCheck>(AssertSqlSafe(sql.as_str()));
        if let Some(ref v) = req.code {
            q = q.bind(v);
        }
        if let Some(ref v) = req.notice {
            q = q.bind(v);
        }
        if let Some(ref v) = req.comments {
            q = q.bind(v);
        }
        if let Some(v) = req.fk_settle {
            q = q.bind(v);
        }
        if let Some(v) = req.fk_account {
            q = q.bind(v);
        }
        if let Some(ref v) = req.qk_amount {
            q = q.bind(v);
        }
        if let Some(ref v) = req.qk_write_off {
            q = q.bind(v);
        }
        if let Some(ref v) = req.qk_tax {
            q = q.bind(v);
        }
        if let Some(v) = req.sk_currency {
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
