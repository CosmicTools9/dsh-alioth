//! InboundBom Repository — isahl."zc_id_bom-inbound" CRUD（crud 模式：GenericRepository 委托读侧/删除）。
use async_trait::async_trait;
use common::AliothError as ApiError;
use crud::{AliothRepository, GenericRepository, ListQuery, PaginatedResponse};
use sqlx::PgPool;

use crate::models::{CreateInboundBomRequest, InboundBom, UpdateInboundBomRequest};

#[derive(Clone)]
pub struct InboundBomRepository {
    generic: GenericRepository<InboundBom>,
    pool: PgPool,
}

impl InboundBomRepository {
    pub fn new(pool: PgPool) -> Self {
        Self {
            generic: GenericRepository::new(pool.clone()),
            pool,
        }
    }

    /// 按 ID 读取（含 _refs 解析）
    pub async fn get_refs(&self, id: i64) -> Result<Option<InboundBom>, ApiError> {
        self.generic.get_refs(id, None).await
    }
}

#[async_trait]
impl AliothRepository<InboundBom, CreateInboundBomRequest, UpdateInboundBomRequest, ApiError>
    for InboundBomRepository
{
    async fn list(&self, query: &ListQuery) -> Result<PaginatedResponse<InboundBom>, ApiError> {
        self.generic.list_refs(query).await
    }

    async fn get(&self, id: i64) -> Result<Option<InboundBom>, ApiError> {
        self.generic.get_refs(id, None).await
    }

    async fn create(
        &self,
        req: CreateInboundBomRequest,
        user_id: i64,
    ) -> Result<InboundBom, ApiError> {
        let id: i64 = sqlx::query_scalar(
            r#"INSERT INTO isahl."zc_id_bom-inbound"
               (notice, code, comments, ak_source, tk_version, tk_batch_no, fk_previous, ck_branch, b_number, fk_editor, "type", created_by_id)
               VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12)
               RETURNING id"#,
        )
        .bind(&req.notice)
        .bind(&req.code)
        .bind(&req.comments)
        .bind(&req.ak_source)
        .bind(req.tk_version)
        .bind(req.tk_batch_no)
        .bind(req.fk_previous)
        .bind(req.ck_branch)
        .bind(&req.b_number)
        .bind(req.fk_editor)
        .bind(&req.r#type)
        .bind(user_id)
        .fetch_one(&self.pool)
        .await
        .map_err(ApiError::from)?;

        self.get_refs(id)
            .await?
            .ok_or_else(|| ApiError::NotFound(format!("inbound_bom {} not found", id)))
    }

    async fn update(
        &self,
        id: i64,
        req: UpdateInboundBomRequest,
        user_id: i64,
    ) -> Result<Option<InboundBom>, ApiError> {
        if self.get_refs(id).await?.is_none() {
            return Ok(None);
        }

        let rows = sqlx::query(
            r#"UPDATE isahl."zc_id_bom-inbound"
               SET                    notice = COALESCE($1, notice),
                   code = COALESCE($2, code),
                   comments = COALESCE($3, comments),
                   ak_source = COALESCE($4, ak_source),
                   tk_version = COALESCE($5, tk_version),
                   tk_batch_no = COALESCE($6, tk_batch_no),
                   fk_previous = COALESCE($7, fk_previous),
                   ck_branch = COALESCE($8, ck_branch),
                   b_number = COALESCE($9, b_number),
                   fk_editor = COALESCE($10, fk_editor),
                   "type" = COALESCE($11, "type"),
                   updated_by_id = $12
               WHERE id = $13 AND deleted_at IS NULL"#,
        )
        .bind(&req.notice)
        .bind(&req.code)
        .bind(&req.comments)
        .bind(&req.ak_source)
        .bind(req.tk_version)
        .bind(req.tk_batch_no)
        .bind(req.fk_previous)
        .bind(req.ck_branch)
        .bind(&req.b_number)
        .bind(req.fk_editor)
        .bind(&req.r#type)
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
