//! 实体版本 Service（entity 面）——CRUD + 版本链 + 回滚
//!
//! 逻辑提取自 WZ version 生产实现（consolidate-version-services）。

use common::data::{ListQuery, PaginatedResponse};
use common::AliothError as ApiError;
use crud::repository::AliothRepository;
use sqlx::PgPool;

use crate::entity::models::{CreateVersionRequest, UpdateVersionRequest, VersionRecord};
use crate::entity::repository::VersionRepository;

/// 版本业务服务
#[derive(Clone)]
pub struct VersionService {
    repo: VersionRepository,
}

impl VersionService {
    pub fn new(pool: PgPool) -> Self {
        Self {
            repo: VersionRepository::from(pool),
        }
    }

    pub async fn list_versions(
        &self,
        query: &ListQuery,
    ) -> Result<PaginatedResponse<VersionRecord>, ApiError> {
        self.repo.list(query).await
    }

    pub async fn get_version(&self, id: i64) -> Result<Option<VersionRecord>, ApiError> {
        self.repo.get(id).await
    }

    pub async fn create_version(
        &self,
        req: CreateVersionRequest,
        user_id: i64,
    ) -> Result<VersionRecord, ApiError> {
        self.repo.create(req, user_id).await
    }

    pub async fn update_version(
        &self,
        id: i64,
        req: UpdateVersionRequest,
        user_id: i64,
    ) -> Result<Option<VersionRecord>, ApiError> {
        self.repo.update(id, req, user_id).await
    }

    pub async fn delete_version(&self, id: i64, user_id: i64) -> Result<(), ApiError> {
        self.repo.delete(id, user_id).await
    }

    /// 获取从指定版本到根版本的完整版本链（沿 fk_previous 回溯）
    pub async fn get_version_chain(&self, id: i64) -> Result<Vec<VersionRecord>, ApiError> {
        let mut chain = Vec::new();
        let mut current_id = Some(id);
        while let Some(cid) = current_id {
            if let Some(v) = self.repo.get(cid).await? {
                current_id = v.fk_previous;
                chain.push(v);
            } else {
                break;
            }
        }
        Ok(chain)
    }

    /// 回滚到指定版本：创建新版本，fk_previous 指向目标版本
    pub async fn rollback_to_version(
        &self,
        target_id: i64,
        user_id: i64,
    ) -> Result<VersionRecord, ApiError> {
        let target = self
            .repo
            .get(target_id)
            .await?
            .ok_or_else(|| ApiError::NotFound("version not found".into()))?;
        let req = CreateVersionRequest {
            notice: target.notice.clone(),
            code: Some(format!("rollback-{}", target_id)),
            comments: Some(format!("Rollback from version {}", target_id)),
            tk_version: target.tk_version,
            tk_batch_no: target.tk_batch_no,
            reversion: target.reversion,
            fk_previous: Some(target_id),
            ck_branch: target.ck_branch,
            tpl_id: target.tpl_id,
        };
        self.repo.create(req, user_id).await
    }

    /// 获取内部 pool 引用（用于其他仓库操作）
    pub fn pool(&self) -> &PgPool {
        self.repo.pool()
    }
}
