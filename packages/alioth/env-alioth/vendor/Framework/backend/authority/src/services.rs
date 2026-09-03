//! 身份因子 Service — 业务逻辑层

use common::data::{ListQuery, PaginatedResponse};
use common::error::AliothError;
use crud::repository::AliothRepository;
use sqlx::PgPool;

use super::models::{
    ApprovalRole, Approver, CreateApprovalRoleRequest, CreateApproverRequest,
    CreateEmployeeRequest, CreateSkillTagRequest, Employee, SkillTag, UpdateApprovalRoleRequest,
    UpdateApproverRequest, UpdateEmployeeRequest, UpdateSkillTagRequest,
};
use super::repositories::{
    ApprovalRoleRepository, ApproverRepository, EmployeeRepository, SkillTagRepository,
};

// ── EmployeeService ───────────────────────────────────────────

#[derive(Clone)]
pub struct EmployeeService {
    repo: EmployeeRepository,
}

impl EmployeeService {
    pub fn new(pool: PgPool) -> Self {
        Self {
            repo: EmployeeRepository::new(pool),
        }
    }

    pub async fn list(
        &self,
        query: &ListQuery,
    ) -> Result<PaginatedResponse<Employee>, AliothError> {
        self.repo.list(query).await
    }

    /// List with optional RLS filtering.
    pub async fn list_with_rls(
        &self,
        query: &ListQuery,
        visible_ids: Option<&[i64]>,
    ) -> Result<PaginatedResponse<Employee>, AliothError> {
        self.repo.list_with_rls(query, visible_ids).await
    }

    pub async fn get(&self, id: i64) -> Result<Option<Employee>, AliothError> {
        self.repo.get(id).await
    }

    pub async fn create(
        &self,
        req: CreateEmployeeRequest,
        user_id: i64,
    ) -> Result<Employee, AliothError> {
        if req.name.trim().is_empty() {
            return Err(AliothError::Validation {
                field: "name".into(),
                message: "工程师名称不能为空".into(),
            });
        }
        if let Some(code) = &req.code {
            if code.trim().is_empty() {
                return Err(AliothError::Validation {
                    field: "code".into(),
                    message: "工号不能为空字符串".into(),
                });
            }
        }
        self.repo.create(req, user_id).await
    }

    pub async fn update(
        &self,
        id: i64,
        req: UpdateEmployeeRequest,
        user_id: i64,
    ) -> Result<Option<Employee>, AliothError> {
        if let Some(name) = &req.name {
            if name.trim().is_empty() {
                return Err(AliothError::Validation {
                    field: "name".into(),
                    message: "工程师名称不能为空".into(),
                });
            }
        }
        if let Some(code) = &req.code {
            if code.trim().is_empty() {
                return Err(AliothError::Validation {
                    field: "code".into(),
                    message: "工号不能为空字符串".into(),
                });
            }
        }
        self.repo.update(id, req, user_id).await
    }

    pub async fn delete(&self, id: i64, user_id: i64) -> Result<(), AliothError> {
        self.repo.delete(id, user_id).await
    }
}

// ── SkillTagService ───────────────────────────────────────────

#[derive(Clone)]
pub struct SkillTagService {
    repo: SkillTagRepository,
}

impl SkillTagService {
    pub fn new(pool: PgPool) -> Self {
        Self {
            repo: SkillTagRepository::new(pool),
        }
    }

    pub async fn list(
        &self,
        query: &ListQuery,
    ) -> Result<PaginatedResponse<SkillTag>, AliothError> {
        self.repo.list(query).await
    }

    pub async fn list_with_rls(
        &self,
        query: &ListQuery,
        visible_ids: Option<&[i64]>,
    ) -> Result<PaginatedResponse<SkillTag>, AliothError> {
        self.repo.list_with_rls(query, visible_ids).await
    }

    pub async fn get(&self, id: i64) -> Result<Option<SkillTag>, AliothError> {
        self.repo.get(id).await
    }

    pub async fn create(
        &self,
        req: CreateSkillTagRequest,
        user_id: i64,
    ) -> Result<SkillTag, AliothError> {
        if req.name.trim().is_empty() {
            return Err(AliothError::Validation {
                field: "name".into(),
                message: "技能标签名称不能为空".into(),
            });
        }
        self.repo.create(req, user_id).await
    }

    pub async fn update(
        &self,
        id: i64,
        req: UpdateSkillTagRequest,
        user_id: i64,
    ) -> Result<Option<SkillTag>, AliothError> {
        self.repo.update(id, req, user_id).await
    }

    pub async fn delete(&self, id: i64, user_id: i64) -> Result<(), AliothError> {
        self.repo.delete(id, user_id).await
    }
}

// ── ApprovalRoleService ───────────────────────────────────────

#[derive(Clone)]
pub struct ApprovalRoleService {
    repo: ApprovalRoleRepository,
}

impl ApprovalRoleService {
    pub fn new(pool: PgPool) -> Self {
        Self {
            repo: ApprovalRoleRepository::new(pool),
        }
    }

    pub async fn list(
        &self,
        query: &ListQuery,
    ) -> Result<PaginatedResponse<ApprovalRole>, AliothError> {
        self.repo.list(query).await
    }

    pub async fn list_with_rls(
        &self,
        query: &ListQuery,
        visible_ids: Option<&[i64]>,
    ) -> Result<PaginatedResponse<ApprovalRole>, AliothError> {
        self.repo.list_with_rls(query, visible_ids).await
    }

    pub async fn get(&self, id: i64) -> Result<Option<ApprovalRole>, AliothError> {
        self.repo.get(id).await
    }

    pub async fn create(
        &self,
        req: CreateApprovalRoleRequest,
        user_id: i64,
    ) -> Result<ApprovalRole, AliothError> {
        if req.name.trim().is_empty() {
            return Err(AliothError::Validation {
                field: "name".into(),
                message: "审批岗位名称不能为空".into(),
            });
        }
        self.repo.create(req, user_id).await
    }

    pub async fn update(
        &self,
        id: i64,
        req: UpdateApprovalRoleRequest,
        user_id: i64,
    ) -> Result<Option<ApprovalRole>, AliothError> {
        self.repo.update(id, req, user_id).await
    }

    pub async fn delete(&self, id: i64, user_id: i64) -> Result<(), AliothError> {
        self.repo.delete(id, user_id).await
    }
}

// ── ApproverService ──────────────────────────────────────────

#[derive(Clone)]
pub struct ApproverService {
    repo: ApproverRepository,
}

impl ApproverService {
    pub fn new(pool: PgPool) -> Self {
        Self {
            repo: ApproverRepository::new(pool),
        }
    }

    pub async fn list(
        &self,
        query: &ListQuery,
    ) -> Result<PaginatedResponse<Approver>, AliothError> {
        self.repo.list(query).await
    }

    pub async fn list_with_rls(
        &self,
        query: &ListQuery,
        visible_ids: Option<&[i64]>,
    ) -> Result<PaginatedResponse<Approver>, AliothError> {
        self.repo.list_with_rls(query, visible_ids).await
    }

    pub async fn get(&self, id: i64) -> Result<Option<Approver>, AliothError> {
        self.repo.get(id).await
    }

    pub async fn create(
        &self,
        req: CreateApproverRequest,
        user_id: i64,
    ) -> Result<Approver, AliothError> {
        if req.name.trim().is_empty() {
            return Err(AliothError::Validation {
                field: "name".into(),
                message: "CCB 成员名称不能为空".into(),
            });
        }
        self.repo.create(req, user_id).await
    }

    pub async fn update(
        &self,
        id: i64,
        req: UpdateApproverRequest,
        user_id: i64,
    ) -> Result<Option<Approver>, AliothError> {
        self.repo.update(id, req, user_id).await
    }

    pub async fn delete(&self, id: i64, user_id: i64) -> Result<(), AliothError> {
        self.repo.delete(id, user_id).await
    }
}
