//! Alioth namespace approval service — 薄封装层
//!
//! 审批后端逻辑已下沉到 Framework/backend/approval。
//! 本模块仅负责注册 namespace 路由前缀。
//!
//! ## 路由委托
//! - `/service/approval/*` → `Framework/backend/approval` crate
//!
//! ## 本体对应
//! - `ApprovalFlow`       → `isahl.zc_id_process`
//! - `FlowNode`           → `isahl.zc_id_operation` (oper-approve / oper-gate)
//! - `ApprovalInstance`   → `isahl.zc_id_even-approve`
//! - `ApprovalAction`     → `isahl.zc_id_deta-opinion`
//! - `DelegationRule`     → `isahl.zc_id_operation`

use actix_web::web;

pub fn register_service_routes(cfg: &mut actix_web::web::ServiceConfig) {
    cfg.service(web::scope("/service/approval").configure(approval::configure_routes));
}
