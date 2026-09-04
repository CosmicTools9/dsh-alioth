//! SCIM 2.0 模块（RFC 7643/7644）
//!
//! 路由前缀 `/scim/v2`，由 `lib.rs::configure_public_routes` 注册；认证（静态 Bearer token）
//! 在每个 handler 内通过 `handlers::require_scim_token` 强制。

pub mod handlers;
pub mod models;

use actix_web::web;

/// 注册 `/scim/v2/*` 路由。
pub fn configure(cfg: &mut web::ServiceConfig) {
    cfg.route(
        "/ServiceProviderConfig",
        web::get().to(handlers::get_service_provider_config),
    )
    .route("/Schemas", web::get().to(handlers::get_schemas))
    .route(
        "/ResourceTypes",
        web::get().to(handlers::get_resource_types),
    )
    .route("/Users", web::get().to(handlers::list_users))
    .route("/Users", web::post().to(handlers::create_user))
    .route("/Users/{id}", web::get().to(handlers::get_user))
    .route("/Users/{id}", web::put().to(handlers::replace_user))
    .route("/Users/{id}", web::patch().to(handlers::patch_user))
    .route("/Users/{id}", web::delete().to(handlers::delete_user))
    .route("/Groups", web::get().to(handlers::list_groups))
    .route("/Groups", web::post().to(handlers::create_group))
    .route("/Groups/{id}", web::get().to(handlers::get_group))
    .route("/Groups/{id}", web::put().to(handlers::replace_group))
    .route("/Groups/{id}", web::delete().to(handlers::delete_group));
}
