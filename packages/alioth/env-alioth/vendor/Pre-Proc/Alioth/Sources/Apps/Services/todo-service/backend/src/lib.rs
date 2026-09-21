//! Service 后端 — AliothStudio 脚手架生成
//!
//! 路由注册由 Gateway build.rs 按 service.json 自动接线。

pub mod handlers;
pub mod models;
pub mod repositories;

use actix_web::web;

/// Gateway service_registry 约定入口：注册本服务全部路由。
pub fn register_service_routes(cfg: &mut web::ServiceConfig) {
    handlers::register(cfg);
}
