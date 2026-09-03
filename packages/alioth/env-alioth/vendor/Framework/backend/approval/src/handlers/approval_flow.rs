//! 审批流程定义 Handler — 标准 CRUD 路由
use crate::models::{ApprovalFlow, CreateApprovalFlowRequest, UpdateApprovalFlowRequest};
use crate::repositories::ApprovalFlowRepository;
use actix_web::web;
use common::error::AliothError;
use crud::crud_routes;

pub fn register(cfg: &mut web::ServiceConfig) {
    crud_routes::<
        ApprovalFlow,
        CreateApprovalFlowRequest,
        UpdateApprovalFlowRequest,
        ApprovalFlowRepository,
        AliothError,
    >("/approval-flows")(cfg);
}
