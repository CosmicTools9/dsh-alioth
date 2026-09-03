//! 审批动作记录 Handler — 标准 CRUD 路由
use crate::models::{ApprovalAction, CreateApprovalActionRequest, UpdateApprovalActionRequest};
use crate::repositories::ApprovalActionRepository;
use actix_web::web;
use common::error::AliothError;
use crud::crud_routes;

pub fn register(cfg: &mut web::ServiceConfig) {
    crud_routes::<
        ApprovalAction,
        CreateApprovalActionRequest,
        UpdateApprovalActionRequest,
        ApprovalActionRepository,
        AliothError,
    >("/approval-actions")(cfg);
}
