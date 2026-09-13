// ContractRepository（split 自 repository.rs 单体，④ 候选）
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

use crate::models::{Contract, CreateContractRequest, UpdateContractRequest};

use super::ontology_binding;
// ═══════════════════════════════════════════════
// Contract Repository — "isahl"."zc_id_contract"
// ═══════════════════════════════════════════════

#[derive(Clone)]
pub struct ContractRepository {
    generic: GenericRepository<Contract>,
    pool: PgPool,
}

impl ContractRepository {
    pub fn new(pool: PgPool) -> Self {
        Self {
            generic: GenericRepository::new(pool.clone()),
            pool,
        }
    }
}

impl From<PgPool> for ContractRepository {
    fn from(pool: PgPool) -> Self {
        Self::new(pool.clone())
    }
}

#[async_trait]
impl AliothRepository<Contract, CreateContractRequest, UpdateContractRequest, ApiError>
    for ContractRepository
{
    async fn list(&self, query: &ListQuery) -> Result<PaginatedResponse<Contract>, ApiError> {
        self.generic.list_refs(query).await
    }
    async fn get(&self, id: i64) -> Result<Option<Contract>, ApiError> {
        self.generic.get_refs(id, None).await
    }
    async fn create(&self, req: CreateContractRequest, user_id: i64) -> Result<Contract, ApiError> {
        let (dk_scene, dk_factor, dk_function) =
            ontology_binding::resolve(&self.pool, "Contract").await?;
        sqlx::query_as::<_, Contract>(
            r#"INSERT INTO "isahl"."zc_id_cont-proxy" (code, notice, comments, qk_date, created_by_id, dk_scene, dk_factor, dk_function)
               VALUES ($1, $2, $3, $4, $5, $6, $7, $8)
               RETURNING id, notice, code, o_number, comments, projection, t_color_, qk_date, "qk_valid-segm", tpl_id, dk_scene, dk_factor, dk_function, created_at, updated_at, deleted_at"#,
        )
        .bind(&req.code)
        .bind(&req.notice)
        .bind(&req.comments)
        .bind(req.sign_date)
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
        req: UpdateContractRequest,
        _user_id: i64,
    ) -> Result<Option<Contract>, ApiError> {
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
        if req.sign_date.is_some() {
            idx += 1;
            sets.push(format!("qk_date = ${}", idx));
        }
        if sets.is_empty() {
            return self.get(id).await;
        }
        sets.push("updated_at = NOW()".into());
        idx += 1;
        sets.push(format!("updated_by_id = ${}", idx));
        let id_param = idx + 1;
        let sql = format!(
            r#"UPDATE "isahl"."zc_id_contract" SET {} WHERE id = ${} AND deleted_at IS NULL
               RETURNING id, notice, code, o_number, comments, projection, t_color_, qk_date, "qk_valid-segm", tpl_id, dk_scene, dk_factor, dk_function, created_at, updated_at, deleted_at"#,
            sets.join(", "),
            id_param
        );
        let mut q = sqlx::query_as::<_, Contract>(AssertSqlSafe(sql.as_str()));
        if let Some(ref v) = req.code {
            q = q.bind(v);
        }
        if let Some(ref v) = req.notice {
            q = q.bind(v);
        }
        if let Some(ref v) = req.comments {
            q = q.bind(v);
        }
        if let Some(v) = req.sign_date {
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

// ═══════════════════════════════════════════════════════════════════════════════
