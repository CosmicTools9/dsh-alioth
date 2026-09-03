//! 审批委托规则 Handler — 标准 CRUD 路由
use crate::models::{CreateDelegationRuleRequest, DelegationRule, UpdateDelegationRuleRequest};
use crate::repositories::DelegationRuleRepository;
use actix_web::web;
use common::error::AliothError;
use crud::crud_routes;

pub fn register(cfg: &mut web::ServiceConfig) {
    crud_routes::<
        DelegationRule,
        CreateDelegationRuleRequest,
        UpdateDelegationRuleRequest,
        DelegationRuleRepository,
        AliothError,
    >("/delegation-rules")(cfg);
}
