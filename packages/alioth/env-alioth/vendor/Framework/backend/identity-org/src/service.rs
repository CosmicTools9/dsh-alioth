//! 身份实体 Service — 业务逻辑层

use common::data::{ListQuery, PaginatedResponse};
use common::AliothError as ApiError;
use crud::repository::AliothRepository;
use sqlx::PgPool;

use crate::models::{CreateIdentityRequest, Identity, UpdateIdentityRequest};
use crate::repository::IdentityRepository;

#[derive(Clone)]
pub struct IdentityService {
    repo: IdentityRepository,
}

impl IdentityService {
    pub fn new(pool: PgPool) -> Self {
        Self {
            repo: IdentityRepository::new(pool),
        }
    }

    pub async fn list(&self, query: &ListQuery) -> Result<PaginatedResponse<Identity>, ApiError> {
        self.repo.list(query).await
    }

    pub async fn get(&self, id: i64) -> Result<Option<Identity>, ApiError> {
        self.repo.get(id).await
    }

    pub async fn create(
        &self,
        req: CreateIdentityRequest,
        user_id: i64,
    ) -> Result<Identity, ApiError> {
        if let Some(ref code) = req.code {
            if code.is_empty() {
                return Err(ApiError::Validation {
                    field: "code".into(),
                    message: "不能为空字符串".into(),
                });
            }
        }
        self.repo.create(req, user_id).await
    }

    pub async fn update(
        &self,
        id: i64,
        req: UpdateIdentityRequest,
        user_id: i64,
    ) -> Result<Option<Identity>, ApiError> {
        self.repo.update(id, req, user_id).await
    }

    pub async fn delete(&self, id: i64, user_id: i64) -> Result<(), ApiError> {
        self.repo.delete(id, user_id).await
    }
}
