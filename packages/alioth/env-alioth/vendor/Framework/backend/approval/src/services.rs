//! # 审批承诺因子 — 服务层
//!
//! 当前实现为直通层，委托给各 Repository。
//! 后续在此处添加审批链解析、并行会签、超时自动驳回等业务逻辑。

use crate::models::{
    ApprovalAction, ApprovalFlow, ApprovalInstance, CreateApprovalActionRequest,
    CreateApprovalFlowRequest, CreateApprovalInstanceRequest, CreateDelegationRuleRequest,
    CreateFlowNodeRequest, DelegationRule, FlowNode, UpdateApprovalActionRequest,
    UpdateApprovalFlowRequest, UpdateApprovalInstanceRequest, UpdateDelegationRuleRequest,
    UpdateFlowNodeRequest,
};
use crate::repositories::{
    ApprovalActionRepository, ApprovalFlowRepository, ApprovalInstanceRepository,
    DelegationRuleRepository, FlowNodeRepository,
};
use common::data::{ListQuery, PaginatedResponse};
use common::error::AliothError;
use crud::repository::AliothRepository;

// ── ApprovalFlowService ───────────────────────────────────────

#[derive(Clone)]
pub struct ApprovalFlowService {
    repo: ApprovalFlowRepository,
}

impl ApprovalFlowService {
    pub fn new(repo: ApprovalFlowRepository) -> Self {
        Self { repo }
    }

    pub async fn list(
        &self,
        query: &ListQuery,
    ) -> Result<PaginatedResponse<ApprovalFlow>, AliothError> {
        self.repo.list(query).await
    }

    pub async fn get(&self, id: i64) -> Result<Option<ApprovalFlow>, AliothError> {
        self.repo.get(id).await
    }

    pub async fn create(
        &self,
        req: CreateApprovalFlowRequest,
        user_id: i64,
    ) -> Result<ApprovalFlow, AliothError> {
        self.repo.create(req, user_id).await
    }

    pub async fn update(
        &self,
        id: i64,
        req: UpdateApprovalFlowRequest,
        user_id: i64,
    ) -> Result<Option<ApprovalFlow>, AliothError> {
        self.repo.update(id, req, user_id).await
    }

    pub async fn delete(&self, id: i64, user_id: i64) -> Result<(), AliothError> {
        self.repo.delete(id, user_id).await
    }
}

// ── FlowNodeService ───────────────────────────────────────────

#[derive(Clone)]
pub struct FlowNodeService {
    repo: FlowNodeRepository,
}

impl FlowNodeService {
    pub fn new(repo: FlowNodeRepository) -> Self {
        Self { repo }
    }

    pub async fn list(
        &self,
        query: &ListQuery,
    ) -> Result<PaginatedResponse<FlowNode>, AliothError> {
        self.repo.list(query).await
    }

    pub async fn get(&self, id: i64) -> Result<Option<FlowNode>, AliothError> {
        self.repo.get(id).await
    }

    pub async fn create(
        &self,
        req: CreateFlowNodeRequest,
        user_id: i64,
    ) -> Result<FlowNode, AliothError> {
        self.repo.create(req, user_id).await
    }

    pub async fn update(
        &self,
        id: i64,
        req: UpdateFlowNodeRequest,
        user_id: i64,
    ) -> Result<Option<FlowNode>, AliothError> {
        self.repo.update(id, req, user_id).await
    }

    pub async fn delete(&self, id: i64, user_id: i64) -> Result<(), AliothError> {
        self.repo.delete(id, user_id).await
    }
}

// ── ApprovalInstanceService ───────────────────────────────────

#[derive(Clone)]
pub struct ApprovalInstanceService {
    repo: ApprovalInstanceRepository,
}

impl ApprovalInstanceService {
    pub fn new(repo: ApprovalInstanceRepository) -> Self {
        Self { repo }
    }

    pub async fn list(
        &self,
        query: &ListQuery,
    ) -> Result<PaginatedResponse<ApprovalInstance>, AliothError> {
        self.repo.list(query).await
    }

    pub async fn get(&self, id: i64) -> Result<Option<ApprovalInstance>, AliothError> {
        self.repo.get(id).await
    }

    pub async fn create(
        &self,
        req: CreateApprovalInstanceRequest,
        user_id: i64,
    ) -> Result<ApprovalInstance, AliothError> {
        self.repo.create(req, user_id).await
    }

    pub async fn update(
        &self,
        id: i64,
        req: UpdateApprovalInstanceRequest,
        user_id: i64,
    ) -> Result<Option<ApprovalInstance>, AliothError> {
        self.repo.update(id, req, user_id).await
    }

    pub async fn delete(&self, id: i64, user_id: i64) -> Result<(), AliothError> {
        self.repo.delete(id, user_id).await
    }
}

// ── ApprovalActionService ─────────────────────────────────────

#[derive(Clone)]
pub struct ApprovalActionService {
    repo: ApprovalActionRepository,
}

impl ApprovalActionService {
    pub fn new(repo: ApprovalActionRepository) -> Self {
        Self { repo }
    }

    pub async fn list(
        &self,
        query: &ListQuery,
    ) -> Result<PaginatedResponse<ApprovalAction>, AliothError> {
        self.repo.list(query).await
    }

    pub async fn get(&self, id: i64) -> Result<Option<ApprovalAction>, AliothError> {
        self.repo.get(id).await
    }

    pub async fn create(
        &self,
        req: CreateApprovalActionRequest,
        user_id: i64,
    ) -> Result<ApprovalAction, AliothError> {
        self.repo.create(req, user_id).await
    }

    pub async fn update(
        &self,
        id: i64,
        req: UpdateApprovalActionRequest,
        user_id: i64,
    ) -> Result<Option<ApprovalAction>, AliothError> {
        self.repo.update(id, req, user_id).await
    }

    pub async fn delete(&self, id: i64, user_id: i64) -> Result<(), AliothError> {
        self.repo.delete(id, user_id).await
    }
}

// ── DelegationRuleService ─────────────────────────────────────

#[derive(Clone)]
pub struct DelegationRuleService {
    repo: DelegationRuleRepository,
}

impl DelegationRuleService {
    pub fn new(repo: DelegationRuleRepository) -> Self {
        Self { repo }
    }

    pub async fn list(
        &self,
        query: &ListQuery,
    ) -> Result<PaginatedResponse<DelegationRule>, AliothError> {
        self.repo.list(query).await
    }

    pub async fn get(&self, id: i64) -> Result<Option<DelegationRule>, AliothError> {
        self.repo.get(id).await
    }

    pub async fn create(
        &self,
        req: CreateDelegationRuleRequest,
        user_id: i64,
    ) -> Result<DelegationRule, AliothError> {
        self.repo.create(req, user_id).await
    }

    pub async fn update(
        &self,
        id: i64,
        req: UpdateDelegationRuleRequest,
        user_id: i64,
    ) -> Result<Option<DelegationRule>, AliothError> {
        self.repo.update(id, req, user_id).await
    }

    pub async fn delete(&self, id: i64, user_id: i64) -> Result<(), AliothError> {
        self.repo.delete(id, user_id).await
    }
}
