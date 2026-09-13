// EnvironmentRepository（split 自 repository.rs 单体，④ 候选）
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

use crate::models::{CreateEnvironmentRequest, Environment, UpdateEnvironmentRequest};

use super::ontology_binding;

// ═══════════════════════════════════════════════════════════════════════════════
// 本体维度绑定辅助（Ontology Binding）
//
// 坐标 code 来自 factor.json 的 ontology.entities[].coordinates，运行时通过 DB
// 维度表解析为 ZUID，再注入 create 语句的 dk_scene/dk_factor/dk_function。
// 注意：IdentityRepository 实际写入的是 zc_id_subjects 叶表，仍复用 Identity
// 实体的坐标（scene=JE, factor=FJA, function=↑_DA）。
// ═══════════════════════════════════════════════
// Environment — isahl.zc_id_prot-env_config
// ═══════════════════════════════════════════════

#[derive(Clone)]
pub struct EnvironmentRepository {
    generic: GenericRepository<Environment>,
    pool: PgPool,
}

impl EnvironmentRepository {
    pub fn new(pool: PgPool) -> Self {
        Self {
            generic: GenericRepository::new(pool.clone()),
            pool,
        }
    }
}

impl From<PgPool> for EnvironmentRepository {
    fn from(pool: PgPool) -> Self {
        Self::new(pool.clone())
    }
}

#[async_trait]
impl AliothRepository<Environment, CreateEnvironmentRequest, UpdateEnvironmentRequest, ApiError>
    for EnvironmentRepository
{
    async fn list(&self, query: &ListQuery) -> Result<PaginatedResponse<Environment>, ApiError> {
        self.generic.list_refs(query).await
    }

    async fn get(&self, id: i64) -> Result<Option<Environment>, ApiError> {
        self.generic.get_refs(id, None).await
    }

    async fn create(
        &self,
        req: CreateEnvironmentRequest,
        user_id: i64,
    ) -> Result<Environment, ApiError> {
        let (dk_scene, dk_factor, dk_function) =
            ontology_binding::resolve(&self.pool, "Environment").await?;
        sqlx::query_as::<_, Environment>(
            r#"INSERT INTO "isahl"."zc_id_prot-env_config" (notice, created_by_id, dk_scene, dk_factor, dk_function)
               VALUES ($1, $2, $3, $4, $5)
               RETURNING id, notice AS name, created_at, updated_at, deleted_at"#,
        )
        .bind(&req.name)
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
        req: UpdateEnvironmentRequest,
        user_id: i64,
    ) -> Result<Option<Environment>, ApiError> {
        let mut sets = Vec::new();
        let mut idx: usize = 0;

        if req.name.is_some() {
            idx += 1;
            sets.push(format!("notice = ${}", idx));
        }

        if sets.is_empty() {
            return self.get(id).await;
        }

        sets.push("updated_at = NOW()".into());
        idx += 1;
        sets.push(format!("updated_by_id = ${}", idx));
        let id_param = idx + 1;

        let sql = format!(
            r#"UPDATE "isahl.zc_id_prot-env_config" SET {} WHERE id = ${} AND deleted_at IS NULL
               RETURNING id, notice AS name, created_at, updated_at, deleted_at"#,
            sets.join(", "),
            id_param
        );

        let mut q = sqlx::query_as::<_, Environment>(AssertSqlSafe(sql.as_str()));
        if let Some(ref v) = req.name {
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
