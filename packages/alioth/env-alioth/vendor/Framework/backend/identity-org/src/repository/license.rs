// LicenseRepository（split 自 repository.rs 单体，④ 候选）
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

use crate::models::{CreateLicenseRequest, License, UpdateLicenseRequest};

use super::ontology_binding;

// ═══════════════════════════════════════════════
// License — isahl.zc_id_prod-license-purchase
// ═══════════════════════════════════════════════

#[derive(Clone)]
pub struct LicenseRepository {
    generic: GenericRepository<License>,
    pool: PgPool,
}

impl LicenseRepository {
    pub fn new(pool: PgPool) -> Self {
        Self {
            generic: GenericRepository::new(pool.clone()),
            pool,
        }
    }
}

impl From<PgPool> for LicenseRepository {
    fn from(pool: PgPool) -> Self {
        Self::new(pool.clone())
    }
}

#[async_trait]
impl AliothRepository<License, CreateLicenseRequest, UpdateLicenseRequest, ApiError>
    for LicenseRepository
{
    async fn list(&self, query: &ListQuery) -> Result<PaginatedResponse<License>, ApiError> {
        self.generic.list_refs(query).await
    }

    async fn get(&self, id: i64) -> Result<Option<License>, ApiError> {
        self.generic.get_refs(id, None).await
    }

    async fn create(&self, req: CreateLicenseRequest, user_id: i64) -> Result<License, ApiError> {
        let (dk_scene, dk_factor, dk_function) =
            ontology_binding::resolve(&self.pool, "License").await?;
        sqlx::query_as::<_, License>(
            r#"INSERT INTO "isahl"."zc_id_prod-license-purchase" (notice, qk_capacity, qk_period, created_by_id, dk_scene, dk_factor, dk_function)
               VALUES ($1, $2, $3, $4, $5, $6, $7)
               RETURNING id, notice AS name, qk_capacity AS qk_qty, qk_period, created_at, updated_at, deleted_at"#,
        )
        .bind(&req.name)
        .bind(req.qk_qty)
        .bind(req.qk_period)
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
        req: UpdateLicenseRequest,
        user_id: i64,
    ) -> Result<Option<License>, ApiError> {
        let mut sets = Vec::new();

        let mut idx: usize = 0;

        if req.name.is_some() {
            idx += 1;
            sets.push(format!("notice = ${}", idx));
        }
        if req.qk_qty.is_some() {
            idx += 1;
            sets.push(format!("qk_capacity = ${}", idx));
        }
        if req.qk_period.is_some() {
            idx += 1;
            sets.push(format!("qk_period = ${}", idx));
        }

        if sets.is_empty() {
            return self.get(id).await;
        }

        sets.push("updated_at = NOW()".into());
        idx += 1;
        sets.push(format!("updated_by_id = ${}", idx));
        let id_param = idx + 1;
        let sql = format!(
            r#"UPDATE "isahl.zc_id_prod-license-purchase" SET {} WHERE id = ${} AND deleted_at IS NULL
               RETURNING id, notice AS name, qk_capacity AS qk_qty, qk_period, created_at, updated_at, deleted_at"#,
            sets.join(", "),
            id_param
        );
        let mut q = sqlx::query_as::<_, License>(AssertSqlSafe(sql.as_str()));
        if let Some(ref v) = req.name {
            q = q.bind(v);
        }
        if let Some(ref v) = req.qk_qty {
            q = q.bind(v);
        }
        if let Some(ref v) = req.qk_period {
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
