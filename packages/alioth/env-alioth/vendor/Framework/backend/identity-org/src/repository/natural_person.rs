// NaturalPersonRepository（split 自 repository.rs 单体，④ 候选）
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

use super::ontology_binding;
use crate::models::{CreateNaturalPersonRequest, NaturalPerson, UpdateNaturalPersonRequest};

// ═══════════════════════════════════════════════
// NaturalPerson — "isahl"."zc_id_empl-natural"（司机/操作员主数据）
// ═══════════════════════════════════════════════

#[derive(Clone)]
pub struct NaturalPersonRepository {
    generic: GenericRepository<NaturalPerson>,
    pool: PgPool,
}

impl NaturalPersonRepository {
    pub fn new(pool: PgPool) -> Self {
        Self {
            generic: GenericRepository::new(pool.clone()),
            pool,
        }
    }
}

impl From<PgPool> for NaturalPersonRepository {
    fn from(pool: PgPool) -> Self {
        Self::new(pool.clone())
    }
}

#[async_trait]
impl
    AliothRepository<
        NaturalPerson,
        CreateNaturalPersonRequest,
        UpdateNaturalPersonRequest,
        ApiError,
    > for NaturalPersonRepository
{
    async fn list(&self, query: &ListQuery) -> Result<PaginatedResponse<NaturalPerson>, ApiError> {
        self.generic.list_refs(query).await
    }

    async fn get(&self, id: i64) -> Result<Option<NaturalPerson>, ApiError> {
        self.generic.get_refs(id, None).await
    }

    async fn create(
        &self,
        req: CreateNaturalPersonRequest,
        user_id: i64,
    ) -> Result<NaturalPerson, ApiError> {
        let (dk_scene, dk_factor, dk_function) =
            ontology_binding::resolve(&self.pool, "NaturalPerson").await?;
        sqlx::query_as::<_, NaturalPerson>(
            r#"INSERT INTO "isahl"."zc_id_empl-natural"
               (code, notice, o_number, comments, fk_user, ck_category, sk_unit, created_by_id, dk_scene, dk_factor, dk_function)
               VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11)
               RETURNING id, code, notice, o_number, comments, fk_user, ck_category, sk_unit, created_at, updated_at, deleted_at"#,
        )
        .bind(&req.code)
        .bind(&req.notice)
        .bind(&req.o_number)
        .bind(&req.comments)
        .bind(req.fk_user)
        .bind(req.ck_category)
        .bind(req.sk_unit)
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
        req: UpdateNaturalPersonRequest,
        user_id: i64,
    ) -> Result<Option<NaturalPerson>, ApiError> {
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
        if req.o_number.is_some() {
            idx += 1;
            sets.push(format!("o_number = ${}", idx));
        }
        if req.comments.is_some() {
            idx += 1;
            sets.push(format!("comments = ${}", idx));
        }
        if req.fk_user.is_some() {
            idx += 1;
            sets.push(format!("fk_user = ${}", idx));
        }
        if req.ck_category.is_some() {
            idx += 1;
            sets.push(format!("ck_category = ${}", idx));
        }
        if req.sk_unit.is_some() {
            idx += 1;
            sets.push(format!("sk_unit = ${}", idx));
        }

        if sets.is_empty() {
            return self.get(id).await;
        }

        sets.push("updated_at = NOW()".into());
        idx += 1;
        sets.push(format!("updated_by_id = ${}", idx));
        let id_param = idx + 1;

        let sql = format!(
            r#"UPDATE "isahl"."zc_id_empl-natural" SET {} WHERE id = ${} AND deleted_at IS NULL
               RETURNING id, code, notice, o_number, comments, fk_user, ck_category, sk_unit, created_at, updated_at, deleted_at"#,
            sets.join(", "),
            id_param
        );

        let mut q = sqlx::query_as::<_, NaturalPerson>(AssertSqlSafe(sql.as_str()));
        if let Some(ref v) = req.code {
            q = q.bind(v);
        }
        if let Some(ref v) = req.notice {
            q = q.bind(v);
        }
        if let Some(ref v) = req.o_number {
            q = q.bind(v);
        }
        if let Some(ref v) = req.comments {
            q = q.bind(v);
        }
        if let Some(v) = req.fk_user {
            q = q.bind(v);
        }
        if let Some(v) = req.ck_category {
            q = q.bind(v);
        }
        if let Some(v) = req.sk_unit {
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
