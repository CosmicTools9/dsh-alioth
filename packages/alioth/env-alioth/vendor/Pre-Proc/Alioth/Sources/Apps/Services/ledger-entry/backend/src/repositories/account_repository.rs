//! Account Repository — isahl."zc_id_stor-account" CRUD（crud 模式：GenericRepository 委托读侧/删除）。
use async_trait::async_trait;
use common::AliothError as ApiError;
use crud::{AliothRepository, GenericRepository, ListQuery, PaginatedResponse};
use sqlx::PgPool;

use crate::models::{Account, CreateAccountRequest, UpdateAccountRequest};

#[derive(Clone)]
pub struct AccountRepository {
    generic: GenericRepository<Account>,
    pool: PgPool,
}

impl AccountRepository {
    pub fn new(pool: PgPool) -> Self {
        Self {
            generic: GenericRepository::new(pool.clone()),
            pool,
        }
    }

    /// 按 ID 读取（含 _refs 解析）
    pub async fn get_refs(&self, id: i64) -> Result<Option<Account>, ApiError> {
        self.generic.get_refs(id, None).await
    }
}

#[async_trait]
impl AliothRepository<Account, CreateAccountRequest, UpdateAccountRequest, ApiError>
    for AccountRepository
{
    async fn list(&self, query: &ListQuery) -> Result<PaginatedResponse<Account>, ApiError> {
        self.generic.list_refs(query).await
    }

    async fn get(&self, id: i64) -> Result<Option<Account>, ApiError> {
        self.generic.get_refs(id, None).await
    }

    async fn create(&self, req: CreateAccountRequest, user_id: i64) -> Result<Account, ApiError> {
        let id: i64 = sqlx::query_scalar(
            r#"INSERT INTO isahl."zc_id_stor-account"
               (notice, code, comments, ak_source, sk_unit, fk_trustee, qk_capacity, name, account, created_by_id)
               VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10)
               RETURNING id"#,
        )
        .bind(&req.notice)
        .bind(&req.code)
        .bind(&req.comments)
        .bind(&req.ak_source)
        .bind(req.sk_unit)
        .bind(req.fk_trustee)
        .bind(req.qk_capacity)
        .bind(&req.name)
        .bind(&req.account)
        .bind(user_id)
        .fetch_one(&self.pool)
        .await
        .map_err(ApiError::from)?;

        self.get_refs(id)
            .await?
            .ok_or_else(|| ApiError::NotFound(format!("account {} not found", id)))
    }

    async fn update(
        &self,
        id: i64,
        req: UpdateAccountRequest,
        user_id: i64,
    ) -> Result<Option<Account>, ApiError> {
        if self.get_refs(id).await?.is_none() {
            return Ok(None);
        }

        let rows = sqlx::query(
            r#"UPDATE isahl."zc_id_stor-account"
               SET                    notice = COALESCE($1, notice),
                   code = COALESCE($2, code),
                   comments = COALESCE($3, comments),
                   ak_source = COALESCE($4, ak_source),
                   sk_unit = COALESCE($5, sk_unit),
                   fk_trustee = COALESCE($6, fk_trustee),
                   qk_capacity = COALESCE($7, qk_capacity),
                   name = COALESCE($8, name),
                   account = COALESCE($9, account),
                   updated_by_id = $10
               WHERE id = $11 AND deleted_at IS NULL"#,
        )
        .bind(&req.notice)
        .bind(&req.code)
        .bind(&req.comments)
        .bind(&req.ak_source)
        .bind(req.sk_unit)
        .bind(req.fk_trustee)
        .bind(req.qk_capacity)
        .bind(&req.name)
        .bind(&req.account)
        .bind(user_id)
        .bind(id)
        .execute(&self.pool)
        .await
        .map_err(ApiError::from)?;
        if rows.rows_affected() == 0 {
            return Ok(None);
        }

        self.get_refs(id).await
    }

    async fn delete(&self, id: i64, user_id: i64) -> Result<(), ApiError> {
        self.generic.delete(id, user_id).await
    }
}
