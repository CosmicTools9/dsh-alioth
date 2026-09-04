//! # isahl-db — isahl 全局主体/组织/身份服务壳（Alioth）
//!
//! 壳纯挂载：全部路由来自 identity-org 共享内核（isahl-db-ns-shell-only spec）。
//! Environment/License/Language 已拆分为独立 service，不在此挂载。

pub mod seed;

use actix_web::web;

/// 注册 isahl-db 服务的全部路由（identity-org 通用域全量）。
pub fn register_service_routes(cfg: &mut web::ServiceConfig) {
    cfg.service(
        web::scope("/service/isahl-db")
            .configure(identity_org::register_identity)
            .configure(identity_org::configure_identities)
            .configure(identity_org::handlers::subjects::register)
            .configure(identity_org::handlers::subject_bank_card::register)
            .configure(identity_org::handlers::subject_invoice_info::register)
            .configure(identity_org::handlers::contacts::register)
            .configure(identity_org::handlers::org_tree::register)
            .configure(identity_org::org_scheme::register)
            .configure(identity_org::handlers::subject_bridges::register)
            .configure(identity_org::handlers::org_levels::register)
            .configure(identity_org::handlers::crud::register_subject_domain),
    );
}
