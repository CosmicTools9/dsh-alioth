//! 审批流程节点 Handler — 标准 CRUD 路由
use crate::models::{CreateFlowNodeRequest, FlowNode, UpdateFlowNodeRequest};
use crate::repositories::FlowNodeRepository;
use actix_web::web;
use common::error::AliothError;
use crud::crud_routes;

pub fn register(cfg: &mut web::ServiceConfig) {
    crud_routes::<
        FlowNode,
        CreateFlowNodeRequest,
        UpdateFlowNodeRequest,
        FlowNodeRepository,
        AliothError,
    >("/flow-nodes")(cfg);
}
