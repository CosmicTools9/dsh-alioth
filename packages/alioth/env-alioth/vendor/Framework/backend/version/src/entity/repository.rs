//! 实体版本 Repository（entity 面）——标准 CRUD + 动态 SET update
//!
//! create 依赖列默认 `gen_next_zuid()`（`zc_id_version` 表 id 默认即 gen_next_zuid——
//! 版本 id 的 ZUID 全局唯一语义正确，非违规）。

use async_trait::async_trait;
use common::data::{ListQuery, PaginatedResponse};
use common::AliothError as ApiError;
use crud::{AliothRepository, GenericRepository};
use sqlx::{AssertSqlSafe, PgPool};

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
        sqlx::query_as::<_, VersionRecord>(
            r#"INSERT INTO isahl.zc_id_version
               (tpl_id, notice, code, comments, tk_version, tk_batch_no, reversion, fk_previous, ck_branch, created_by_id)
               VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10)
               RETURNING id, tpl_id, notice, code, comments, tk_version, tk_batch_no, reversion, fk_previous, ck_branch, created_at, updated_at, deleted_at"#,
        )
        .bind(req.tpl_id)
        .bind(&req.notice)
        .bind(&req.code)
        .bind(&req.comments)
        .bind(req.tk_version)
        .bind(req.tk_batch_no)
        .bind(req.reversion)
        .bind(req.fk_previous)
        .bind(req.ck_branch)
        .bind(user_id)
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
        let mut sets: Vec<String> = Vec::new();
        let mut idx: usize = 0;

        if req.notice.is_some() {
            idx += 1;
            sets.push(format!("notice = ${}", idx));
        }
        if req.code.is_some() {
            idx += 1;
            sets.push(format!("code = ${}", idx));
        }
        if req.comments.is_some() {
            idx += 1;
            sets.push(format!("comments = ${}", idx));
        }
        if req.tk_version.is_some() {
            idx += 1;
            sets.push(format!("tk_version = ${}", idx));
        }
        if req.tk_batch_no.is_some() {
            idx += 1;
            sets.push(format!("tk_batch_no = ${}", idx));
        }
        if req.fk_previous.is_some() {
            idx += 1;
            sets.push(format!("fk_previous = ${}", idx));
        }
        if req.ck_branch.is_some() {
            idx += 1;
            sets.push(format!("ck_branch = ${}", idx));
        }

        if req.tpl_id.is_some() {
            idx += 1;
            sets.push(format!("tpl_id = ${}", idx));
        }
        if req.reversion.is_some() {
            idx += 1;
            sets.push(format!("reversion = ${}", idx));
        }

        if sets.is_empty() {
            return self.generic.get(id).await;
        }

        sets.push("updated_at = NOW()".into());
        idx += 1;
        sets.push(format!("updated_by_id = ${}", idx));
        let id_param = idx + 1;

        let sql = format!(
            r#"UPDATE isahl.zc_id_version
               SET {}
               WHERE id = ${} AND deleted_at IS NULL
               RETURNING id, tpl_id, notice, code, comments, tk_version, tk_batch_no, reversion, fk_previous, ck_branch, created_at, updated_at, deleted_at"#,
            sets.join(", "),
            id_param
        );

        let mut q = sqlx::query_as::<_, VersionRecord>(AssertSqlSafe(sql.as_str()));
        if let Some(ref v) = req.notice {
            q = q.bind(v);
        }
        if let Some(ref v) = req.code {
            q = q.bind(v);
        }
        if let Some(ref v) = req.comments {
            q = q.bind(v);
        }
        if let Some(v) = req.tk_version {
            q = q.bind(v);
        }
        if let Some(v) = req.tk_batch_no {
            q = q.bind(v);
        }
        if let Some(v) = req.reversion {
            q = q.bind(v);
        }
        if let Some(v) = req.fk_previous {
            q = q.bind(v);
        }
        if let Some(v) = req.ck_branch {
            q = q.bind(v);
        }
        if let Some(v) = req.tpl_id {
            q = q.bind(v);
        }
        q = q.bind(user_id);
        q = q.bind(id);

        q.fetch_optional(p).await.map_err(ApiError::from)
    }

    async fn delete(&self, id: i64, user_id: i64) -> Result<(), ApiError> {
        self.generic.delete(id, user_id).await
    }
}
