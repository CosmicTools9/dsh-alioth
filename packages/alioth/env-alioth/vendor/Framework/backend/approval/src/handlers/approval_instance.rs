//! 审批执行实例 Handler — 标准 CRUD 路由
use crate::models::{
    ApprovalInstance, CreateApprovalInstanceRequest, UpdateApprovalInstanceRequest,
};
use crate::repositories::ApprovalInstanceRepository;
use actix_web::web;
use common::error::AliothError;
use crud::crud_routes;

pub fn register(cfg: &mut web::ServiceConfig) {
    crud_routes::<
        ApprovalInstance,
        CreateApprovalInstanceRequest,
        UpdateApprovalInstanceRequest,
        ApprovalInstanceRepository,
        AliothError,
    >("/approval-instances")(cfg);
}
