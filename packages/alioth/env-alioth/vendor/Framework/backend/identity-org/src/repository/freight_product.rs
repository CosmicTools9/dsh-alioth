// FreightProductRepository（split 自 repository.rs 单体，④ 候选）
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

use crate::models::{CreateFreightProductRequest, FreightProduct, UpdateFreightProductRequest};

use super::ontology_binding;

// ═══════════════════════════════════════════════
// FreightProduct Repository — zc_id_prod-freight_road-sales
// ═══════════════════════════════════════════════

#[derive(Clone)]
pub struct FreightProductRepository {
    generic: GenericRepository<FreightProduct>,
    pool: PgPool,
}

impl FreightProductRepository {
    pub fn new(pool: PgPool) -> Self {
        Self {
            generic: GenericRepository::new(pool.clone()),
            pool,
        }
    }
}

impl From<PgPool> for FreightProductRepository {
    fn from(pool: PgPool) -> Self {
        Self::new(pool.clone())
    }
}

#[async_trait]
impl
    AliothRepository<
        FreightProduct,
        CreateFreightProductRequest,
        UpdateFreightProductRequest,
        ApiError,
    > for FreightProductRepository
{
    async fn list(&self, query: &ListQuery) -> Result<PaginatedResponse<FreightProduct>, ApiError> {
        self.generic.list_refs(query).await
    }
    async fn get(&self, id: i64) -> Result<Option<FreightProduct>, ApiError> {
        self.generic.get_refs(id, None).await
    }
    async fn create(
        &self,
        req: CreateFreightProductRequest,
        user_id: i64,
    ) -> Result<FreightProduct, ApiError> {
        let (dk_scene, dk_factor, dk_function) =
            ontology_binding::resolve(&self.pool, "FreightProduct").await?;

        sqlx::query_as::<_, FreightProduct>(
            r#"INSERT INTO "isahl"."zc_id_prod-freight_road-sales"
               (code, notice, comments, fk_previous, "ck_vehicle-form", qk_price, created_by_id, dk_scene, dk_factor, dk_function)
               VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10)
               RETURNING id, code, notice, comments, fk_previous, "ck_vehicle-form", qk_price, created_at, updated_at, deleted_at"#,
        )
        .bind(&req.code).bind(&req.notice).bind(&req.comments)
        .bind(req.fk_previous)
        .bind(req.ck_vehicle_form).bind(req.qk_price)
        .bind(user_id)
        .bind(dk_scene).bind(dk_factor).bind(dk_function)
        .fetch_one(&self.pool)
        .await
        .map_err(ApiError::from)
    }

    async fn update(
        &self,
        id: i64,
        req: UpdateFreightProductRequest,
        user_id: i64,
    ) -> Result<Option<FreightProduct>, ApiError> {
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
        if req.fk_previous.is_some() {
            idx += 1;
            sets.push(format!("fk_previous = ${}", idx));
        }
        if req.ck_vehicle_form.is_some() {
            idx += 1;
            sets.push(format!("\"ck_vehicle-form\" = ${}", idx));
        }
        if req.qk_price.is_some() {
            idx += 1;
            sets.push(format!("qk_price = ${}", idx));
        }
        if sets.is_empty() {
            return self.get(id).await;
        }
        sets.push("updated_at = NOW()".into());
        idx += 1;
        sets.push(format!("updated_by_id = ${}", idx));
        let id_param = idx + 1;

        let sql = format!(
            r#"UPDATE "isahl"."zc_id_prod-freight_road-sales" SET {} WHERE id = ${} AND deleted_at IS NULL
               RETURNING id, code, notice, comments, fk_previous, "ck_vehicle-form", qk_price, created_at, updated_at, deleted_at"#,
            sets.join(", "),
            id_param
        );

        let mut q = sqlx::query_as::<_, FreightProduct>(AssertSqlSafe(sql.as_str()));
        if let Some(ref v) = req.code {
            q = q.bind(v);
        }
        if let Some(ref v) = req.notice {
            q = q.bind(v);
        }
        if let Some(ref v) = req.comments {
            q = q.bind(v);
        }
        if let Some(v) = req.fk_previous {
            q = q.bind(v);
        }
        if let Some(v) = req.ck_vehicle_form {
            q = q.bind(v);
        }
        if let Some(v) = req.qk_price {
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
