// DetaBillCheckRepository（split 自 repository.rs 单体，④ 候选）
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

use crate::models::{CreateDetaBillCheckRequest, DetaBillCheck, UpdateDetaBillCheckRequest};

use super::ontology_binding;

// ═══════════════════════════════════════════════
// DetaBillCheck — "isahl"."zc_id_deta-bill-check"
// ═══════════════════════════════════════════════

#[derive(Clone)]
pub struct DetaBillCheckRepository {
    generic: GenericRepository<DetaBillCheck>,
    pool: PgPool,
}

impl DetaBillCheckRepository {
    pub fn new(pool: PgPool) -> Self {
        Self {
            generic: GenericRepository::new(pool.clone()),
            pool,
        }
    }
}

impl From<PgPool> for DetaBillCheckRepository {
    fn from(pool: PgPool) -> Self {
        Self::new(pool.clone())
    }
}

#[async_trait]
impl
    AliothRepository<
        DetaBillCheck,
        CreateDetaBillCheckRequest,
        UpdateDetaBillCheckRequest,
        ApiError,
    > for DetaBillCheckRepository
{
    async fn list(&self, query: &ListQuery) -> Result<PaginatedResponse<DetaBillCheck>, ApiError> {
        self.generic.list_refs(query).await
    }

    async fn get(&self, id: i64) -> Result<Option<DetaBillCheck>, ApiError> {
        self.generic.get_refs(id, None).await
    }

    async fn create(
        &self,
        req: CreateDetaBillCheckRequest,
        user_id: i64,
    ) -> Result<DetaBillCheck, ApiError> {
        let (dk_scene, dk_factor, dk_function) =
            ontology_binding::resolve(&self.pool, "DetaBillCheck").await?;
        sqlx::query_as::<_, DetaBillCheck>(
            r#"INSERT INTO "isahl"."zc_id_deta-bill-check" (code, notice, comments, fk_list, ck_category, qk_qty, qk_price, qk_amount, created_by_id, dk_scene, dk_factor, dk_function)
               VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12)
               RETURNING id, code, notice, comments, fk_list, ck_category, qk_qty, qk_price, qk_amount, created_at, updated_at, deleted_at"#,
        )
        .bind(&req.code).bind(&req.notice).bind(&req.comments)
        .bind(req.fk_list)
        .bind(req.ck_category)
        .bind(req.qk_qty).bind(req.qk_price).bind(req.qk_amount)
        .bind(user_id)
        .bind(dk_scene).bind(dk_factor).bind(dk_function)
        .fetch_one(&self.pool)
        .await
        .map_err(ApiError::from)
    }

    async fn update(
        &self,
        id: i64,
        req: UpdateDetaBillCheckRequest,
        user_id: i64,
    ) -> Result<Option<DetaBillCheck>, ApiError> {
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

        if sets.is_empty() {
            return self.get(id).await;
        }

        sets.push("updated_at = NOW()".into());
        idx += 1;
        sets.push(format!("updated_by_id = ${}", idx));
        let id_param = idx + 1;

        let sql = format!(
            r#"UPDATE "isahl"."zc_id_deta-bill-check" SET {} WHERE id = ${} AND deleted_at IS NULL
               RETURNING id, code, notice, comments, fk_list, ck_category, qk_qty, qk_price, qk_amount, created_at, updated_at, deleted_at"#,
            sets.join(", "),
            id_param
        );

        let mut q = sqlx::query_as::<_, DetaBillCheck>(AssertSqlSafe(sql.as_str()));
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
        q = q.bind(user_id);
        q = q.bind(id);

        q.fetch_optional(&self.pool).await.map_err(ApiError::from)
    }

    async fn delete(&self, id: i64, user_id: i64) -> Result<(), ApiError> {
        self.generic.delete(id, user_id).await
    }
}
