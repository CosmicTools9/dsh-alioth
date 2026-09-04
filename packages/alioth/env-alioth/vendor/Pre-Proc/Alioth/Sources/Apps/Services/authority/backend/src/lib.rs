//! # service-authority (Alioth)
//!
//! 主体与权限服务。复用 Framework authority 的 Employee/SkillTag/ApprovalRole/Approver 实现。

use actix_web::web;

pub fn register_service_routes(cfg: &mut web::ServiceConfig) {
    authority::register_service_routes_with_guard::<authority::ngac::NoopNgacGuard>(
        cfg,
        "/service/authority",
    );
}
