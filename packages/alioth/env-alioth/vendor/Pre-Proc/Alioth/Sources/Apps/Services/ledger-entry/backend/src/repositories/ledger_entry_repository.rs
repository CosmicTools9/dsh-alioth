//! LedgerEntry Repository — isahl."zc_id_docu-accounting" CRUD（crud 模式：GenericRepository 委托读侧/删除）。
use async_trait::async_trait;
use common::AliothError as ApiError;
use crud::{AliothRepository, GenericRepository, ListQuery, PaginatedResponse};
use sqlx::PgPool;

use crate::models::{CreateLedgerEntryRequest, LedgerEntry, UpdateLedgerEntryRequest};

#[derive(Clone)]
pub struct LedgerEntryRepository {
    generic: GenericRepository<LedgerEntry>,
    pool: PgPool,
}

impl LedgerEntryRepository {
    pub fn new(pool: PgPool) -> Self {
        Self {
            generic: GenericRepository::new(pool.clone()),
            pool,
        }
    }

    /// 按 ID 读取（含 _refs 解析）
    pub async fn get_refs(&self, id: i64) -> Result<Option<LedgerEntry>, ApiError> {
        self.generic.get_refs(id, None).await
    }
}

#[async_trait]
impl AliothRepository<LedgerEntry, CreateLedgerEntryRequest, UpdateLedgerEntryRequest, ApiError>
    for LedgerEntryRepository
{
    async fn list(&self, query: &ListQuery) -> Result<PaginatedResponse<LedgerEntry>, ApiError> {
        self.generic.list_refs(query).await
    }

    async fn get(&self, id: i64) -> Result<Option<LedgerEntry>, ApiError> {
        self.generic.get_refs(id, None).await
    }

    async fn create(
        &self,
        req: CreateLedgerEntryRequest,
        user_id: i64,
    ) -> Result<LedgerEntry, ApiError> {
        // 坐标三元组（§6.12 声明即必须）：会计凭证归档坐标（GH/FHA/↓_GA，同表既有写入
        // seed-wms-transaction-data.sql），经 code 子查询解析 ZUID（禁硬编码 ZUID）
        let id: i64 = sqlx::query_scalar(
            r#"INSERT INTO isahl."zc_id_docu-accounting"
               (notice, code, comments, ak_source, tk_version, tk_batch_no, fk_previous, ck_branch, created_by_id,
                dk_scene, dk_factor, dk_function)
               VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9,
                       (SELECT id FROM isahl.zc_id_scene    WHERE code = 'GH'   AND deleted_at IS NULL),
                       (SELECT id FROM isahl.zc_id_factor   WHERE code = 'FHA'  AND deleted_at IS NULL),
                       (SELECT id FROM isahl.zc_id_function WHERE code = '↓_GA' AND deleted_at IS NULL))
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
        .bind(user_id)
        .fetch_one(&self.pool)
        .await
        .map_err(ApiError::from)?;

        self.get_refs(id)
            .await?
            .ok_or_else(|| ApiError::NotFound(format!("ledger_entry {} not found", id)))
    }

    async fn update(
        &self,
        id: i64,
        req: UpdateLedgerEntryRequest,
        user_id: i64,
    ) -> Result<Option<LedgerEntry>, ApiError> {
        if self.get_refs(id).await?.is_none() {
            return Ok(None);
        }

        let rows = sqlx::query(
            r#"UPDATE isahl."zc_id_docu-accounting"
               SET                    notice = COALESCE($1, notice),
                   code = COALESCE($2, code),
                   comments = COALESCE($3, comments),
                   ak_source = COALESCE($4, ak_source),
                   tk_version = COALESCE($5, tk_version),
                   tk_batch_no = COALESCE($6, tk_batch_no),
                   fk_previous = COALESCE($7, fk_previous),
                   ck_branch = COALESCE($8, ck_branch),
                   updated_by_id = $9
               WHERE id = $10 AND deleted_at IS NULL"#,
        )
        .bind(&req.notice)
        .bind(&req.code)
        .bind(&req.comments)
        .bind(&req.ak_source)
        .bind(req.tk_version)
        .bind(req.tk_batch_no)
        .bind(req.fk_previous)
        .bind(req.ck_branch)
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
