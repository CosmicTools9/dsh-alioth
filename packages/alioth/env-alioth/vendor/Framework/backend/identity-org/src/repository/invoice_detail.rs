// InvoiceDetailRepository（split 自 repository.rs 单体，④ 候选）
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

use crate::models::{CreateInvoiceDetailRequest, InvoiceDetail, UpdateInvoiceDetailRequest};

use super::ontology_binding;

// ═══════════════════════════════════════════════
// InvoiceDetail — "isahl"."zc_id_deta-invoice"
// ═══════════════════════════════════════════════

#[derive(Clone)]
pub struct InvoiceDetailRepository {
    generic: GenericRepository<InvoiceDetail>,
    pool: PgPool,
}

impl InvoiceDetailRepository {
    pub fn new(pool: PgPool) -> Self {
        Self {
            generic: GenericRepository::new(pool.clone()),
            pool,
        }
    }
}

impl From<PgPool> for InvoiceDetailRepository {
    fn from(pool: PgPool) -> Self {
        Self::new(pool.clone())
    }
}

#[async_trait]
impl
    AliothRepository<
        InvoiceDetail,
        CreateInvoiceDetailRequest,
        UpdateInvoiceDetailRequest,
        ApiError,
    > for InvoiceDetailRepository
{
    async fn list(&self, query: &ListQuery) -> Result<PaginatedResponse<InvoiceDetail>, ApiError> {
        self.generic.list_refs(query).await
    }

    async fn get(&self, id: i64) -> Result<Option<InvoiceDetail>, ApiError> {
        self.generic.get_refs(id, None).await
    }

    async fn create(
        &self,
        req: CreateInvoiceDetailRequest,
        user_id: i64,
    ) -> Result<InvoiceDetail, ApiError> {
        let (dk_scene, dk_factor, dk_function) =
            ontology_binding::resolve(&self.pool, "InvoiceDetail").await?;
        sqlx::query_as::<_, InvoiceDetail>(
            r#"INSERT INTO "isahl"."zc_id_deta-invoice" (code, notice, comments, fk_list, fk_subject, ck_category, qk_qty, qk_price, qk_amount, qk_tax_amount, qk_tax_ratio, created_by_id, dk_scene, dk_factor, dk_function)
               VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, $13, $14, $15)
               RETURNING id, code, notice, comments, fk_list, fk_subject, ck_category, qk_qty, qk_price, qk_amount, qk_tax_amount, qk_tax_ratio, created_at, updated_at, deleted_at"#,
        )
        .bind(&req.code).bind(&req.notice).bind(&req.comments)
        .bind(req.fk_list).bind(req.fk_subject)
        .bind(req.ck_category)
        .bind(req.qk_qty).bind(req.qk_price).bind(req.qk_amount)
        .bind(req.qk_tax_amount).bind(req.qk_tax_ratio)
        .bind(user_id)
        .bind(dk_scene).bind(dk_factor).bind(dk_function)
        .fetch_one(&self.pool)
        .await
        .map_err(ApiError::from)
    }

    async fn update(
        &self,
        id: i64,
        req: UpdateInvoiceDetailRequest,
        user_id: i64,
    ) -> Result<Option<InvoiceDetail>, ApiError> {
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
        if req.fk_list.is_some() {
            idx += 1;
            sets.push(format!("fk_list = ${}", idx));
        }
        if req.fk_subject.is_some() {
            idx += 1;
            sets.push(format!("fk_subject = ${}", idx));
        }
        if req.ck_category.is_some() {
            idx += 1;
            sets.push(format!("ck_category = ${}", idx));
        }
        if req.qk_qty.is_some() {
            idx += 1;
            sets.push(format!("qk_qty = ${}", idx));
        }
        if req.qk_price.is_some() {
            idx += 1;
            sets.push(format!("qk_price = ${}", idx));
        }
        if req.qk_amount.is_some() {
            idx += 1;
            sets.push(format!("qk_amount = ${}", idx));
        }
        if req.qk_tax_amount.is_some() {
            idx += 1;
            sets.push(format!("qk_tax_amount = ${}", idx));
        }
        if req.qk_tax_ratio.is_some() {
            idx += 1;
            sets.push(format!("qk_tax_ratio = ${}", idx));
        }

        if sets.is_empty() {
            return self.get(id).await;
        }

        sets.push("updated_at = NOW()".into());
        idx += 1;
        sets.push(format!("updated_by_id = ${}", idx));
        let id_param = idx + 1;

        let sql = format!(
            r#"UPDATE "isahl"."zc_id_deta-invoice" SET {} WHERE id = ${} AND deleted_at IS NULL
               RETURNING id, code, notice, comments, fk_list, fk_subject, ck_category, qk_qty, qk_price, qk_amount, qk_tax_amount, qk_tax_ratio, created_at, updated_at, deleted_at"#,
            sets.join(", "),
            id_param
        );

        let mut q = sqlx::query_as::<_, InvoiceDetail>(AssertSqlSafe(sql.as_str()));
        if let Some(ref v) = req.code {
            q = q.bind(v);
        }
        if let Some(ref v) = req.notice {
            q = q.bind(v);
        }
        if let Some(ref v) = req.comments {
            q = q.bind(v);
        }
        if let Some(v) = req.fk_list {
            q = q.bind(v);
        }
        if let Some(v) = req.fk_subject {
            q = q.bind(v);
        }
        if let Some(ref v) = req.ck_category {
            q = q.bind(v);
        }
        if let Some(ref v) = req.qk_qty {
            q = q.bind(v);
        }
        if let Some(ref v) = req.qk_price {
            q = q.bind(v);
        }
        if let Some(ref v) = req.qk_amount {
            q = q.bind(v);
        }
        if let Some(ref v) = req.qk_tax_amount {
            q = q.bind(v);
        }
        if let Some(ref v) = req.qk_tax_ratio {
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
