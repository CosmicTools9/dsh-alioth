//! SubjectAccount Repository — isahl.zc_id_subjects_rr_account CRUD（crud 模式：GenericRepository 委托读侧/删除）。
use async_trait::async_trait;
use common::AliothError as ApiError;
use crud::{AliothRepository, GenericRepository, ListQuery, PaginatedResponse};
use sqlx::PgPool;

use crate::models::{CreateSubjectAccountRequest, SubjectAccount, UpdateSubjectAccountRequest};

#[derive(Clone)]
pub struct SubjectAccountRepository {
    generic: GenericRepository<SubjectAccount>,
    pool: PgPool,
}

impl SubjectAccountRepository {
    pub fn new(pool: PgPool) -> Self {
        Self {
            generic: GenericRepository::new(pool.clone()),
            pool,
        }
    }

    /// 按 ID 读取（含 _refs 解析）
    pub async fn get_refs(&self, id: i64) -> Result<Option<SubjectAccount>, ApiError> {
        self.generic.get_refs(id, None).await
    }
}

#[async_trait]
impl
    AliothRepository<
        SubjectAccount,
        CreateSubjectAccountRequest,
        UpdateSubjectAccountRequest,
        ApiError,
    > for SubjectAccountRepository
{
    async fn list(&self, query: &ListQuery) -> Result<PaginatedResponse<SubjectAccount>, ApiError> {
        self.generic.list_refs(query).await
    }

    async fn get(&self, id: i64) -> Result<Option<SubjectAccount>, ApiError> {
        self.generic.get_refs(id, None).await
    }

    async fn create(
        &self,
        req: CreateSubjectAccountRequest,
        user_id: i64,
    ) -> Result<SubjectAccount, ApiError> {
        let id: i64 = sqlx::query_scalar(
            r#"INSERT INTO isahl.zc_id_subjects_rr_account
               (notice, code, ref_left, ref_right, comments, qk_period, created_by_id)
               VALUES ($1, $2, $3, $4, $5, $6, $7)
               RETURNING id"#,
        )
        .bind(&req.notice)
        .bind(&req.code)
        .bind(req.ref_left)
        .bind(req.ref_right)
        .bind(&req.comments)
        .bind(req.qk_period)
        .bind(user_id)
        .fetch_one(&self.pool)
        .await
        .map_err(ApiError::from)?;

        self.get_refs(id)
            .await?
            .ok_or_else(|| ApiError::NotFound(format!("subject_account {} not found", id)))
    }

    async fn update(
        &self,
        id: i64,
        req: UpdateSubjectAccountRequest,
        user_id: i64,
    ) -> Result<Option<SubjectAccount>, ApiError> {
        if self.get_refs(id).await?.is_none() {
            return Ok(None);
        }

        let rows = sqlx::query(
            r#"UPDATE isahl.zc_id_subjects_rr_account
               SET                    notice = COALESCE($1, notice),
                   code = COALESCE($2, code),
                   ref_left = COALESCE($3, ref_left),
                   ref_right = COALESCE($4, ref_right),
                   comments = COALESCE($5, comments),
                   qk_period = COALESCE($6, qk_period),
                   updated_by_id = $7
               WHERE id = $8 AND deleted_at IS NULL"#,
        )
        .bind(&req.notice)
        .bind(&req.code)
        .bind(req.ref_left)
        .bind(req.ref_right)
        .bind(&req.comments)
        .bind(req.qk_period)
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
