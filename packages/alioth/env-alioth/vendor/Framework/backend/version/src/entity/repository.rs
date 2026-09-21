//! 实体版本 Repository（entity 面）——标准 CRUD + 静态 COALESCE PATCH update
//!
//! create 依赖列默认 `gen_next_zuid()`（`zc_id_version` 表 id 默认即 gen_next_zuid——
//! 版本 id 的 ZUID 全局唯一语义正确，非违规）。

use async_trait::async_trait;
use common::data::{ListQuery, PaginatedResponse};
use common::AliothError as ApiError;
use crud::{AliothRepository, GenericRepository};
use sqlx::PgPool;

use crate::entity::models::{CreateVersionRequest, UpdateVersionRequest, VersionRecord};

#[derive(Clone)]
pub struct VersionRepository {
    generic: GenericRepository<VersionRecord>,
}

impl From<PgPool> for VersionRepository {
    fn from(pool: PgPool) -> Self {
        Self {
            generic: GenericRepository::new(pool),
        }
    }
}

impl VersionRepository {
    pub fn new(pool: PgPool) -> Self {
        Self::from(pool)
    }

    /// 获取内部数据库连接池引用
    pub fn pool(&self) -> &PgPool {
        self.generic.pool()
    }

    /// 链维护（Alioth 语义）：将同一 tpl_id 的旧链头（fk_previous IS NULL）指向新记录。
    /// 壳层按需调用（Alioth 壳 create 后调用；WZ 壳显式传 fk_previous，不调用）。
    pub async fn link_chain(&self, new_id: i64, tpl_id: Option<i64>) -> Result<(), ApiError> {
        let p = self.generic.pool();
        sqlx::query(
            r#"UPDATE isahl.zc_id_version
               SET fk_previous = $1, updated_at = NOW()
               WHERE tpl_id IS NOT DISTINCT FROM $2
                 AND id != $1
                 AND fk_previous IS NULL
                 AND deleted_at IS NULL"#,
        )
        .bind(new_id)
        .bind(tpl_id)
        .execute(p)
        .await
        .map_err(ApiError::from)?;
        Ok(())
    }
}

#[async_trait]
impl AliothRepository<VersionRecord, CreateVersionRequest, UpdateVersionRequest, ApiError>
    for VersionRepository
{
    async fn list(&self, query: &ListQuery) -> Result<PaginatedResponse<VersionRecord>, ApiError> {
        self.generic.list(query).await
    }

    async fn get(&self, id: i64) -> Result<Option<VersionRecord>, ApiError> {
        self.generic.get(id).await
    }

    async fn create(
        &self,
        req: CreateVersionRequest,
        user_id: i64,
    ) -> Result<VersionRecord, ApiError> {
        let p = self.generic.pool();
        // 坐标三元组（§6.12 声明即必须）：值经 ontology_binding 解析 code→ZUID，禁硬编码 ZUID
        let (dk_scene, dk_factor, dk_function) =
            ontology_binding::resolve(p, ("JE", "GEB", "↑_DA"))
                .await
                .map_err(ApiError::from)?;
        sqlx::query_as::<_, VersionRecord>(
            r#"INSERT INTO isahl."zc_id_bom-file"
               (tpl_id, notice, code, comments, tk_version, tk_batch_no, fk_previous, ck_branch, created_by_id, dk_scene, dk_factor, dk_function)
               VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12)
               RETURNING id, tpl_id, notice, code, comments, tk_version, tk_batch_no, fk_previous, ck_branch, created_at, updated_at, deleted_at"#,
        )
        .bind(req.tpl_id)
        .bind(&req.notice)
        .bind(&req.code)
        .bind(&req.comments)
        .bind(req.tk_version)
        .bind(req.tk_batch_no)
        .bind(req.fk_previous)
        .bind(req.ck_branch)
        .bind(user_id)
        .bind(dk_scene)
        .bind(dk_factor)
        .bind(dk_function)
        .fetch_one(p)
        .await
        .map_err(ApiError::from)
    }

    async fn update(
        &self,
        id: i64,
        req: UpdateVersionRequest,
        user_id: i64,
    ) -> Result<Option<VersionRecord>, ApiError> {
        let p = self.generic.pool();
        // PATCH 语义：None = 不变。静态 SQL（COALESCE 哨兵），避动态 SET 子句拼接
        // （门禁 dynamic-table-name；先例 customer_repository.rs:75 等）。
        // 全 None ⇒ 不改动，直接返回当前行。
        if req.notice.is_none()
            && req.code.is_none()
            && req.comments.is_none()
            && req.tk_version.is_none()
            && req.tk_batch_no.is_none()
            && req.fk_previous.is_none()
            && req.ck_branch.is_none()
            && req.tpl_id.is_none()
        {
            return self.generic.get(id).await;
        }

        sqlx::query_as::<_, VersionRecord>(
            r#"UPDATE isahl.zc_id_version
               SET notice = COALESCE($1, notice),
                   code = COALESCE($2, code),
                   comments = COALESCE($3, comments),
                   tk_version = COALESCE($4, tk_version),
                   tk_batch_no = COALESCE($5, tk_batch_no),
                   fk_previous = COALESCE($6, fk_previous),
                   ck_branch = COALESCE($7, ck_branch),
                   tpl_id = COALESCE($8, tpl_id),
                   updated_at = NOW(),
                   updated_by_id = $9
               WHERE id = $10 AND deleted_at IS NULL
               RETURNING id, tpl_id, notice, code, comments, tk_version, tk_batch_no, fk_previous, ck_branch, created_at, updated_at, deleted_at"#,
        )
        .bind(&req.notice)
        .bind(&req.code)
        .bind(&req.comments)
        .bind(req.tk_version)
        .bind(req.tk_batch_no)
        .bind(req.fk_previous)
        .bind(req.ck_branch)
        .bind(req.tpl_id)
        .bind(user_id)
        .bind(id)
        .fetch_optional(p)
        .await
        .map_err(ApiError::from)
    }

    async fn delete(&self, id: i64, user_id: i64) -> Result<(), ApiError> {
        self.generic.delete(id, user_id).await
    }
}
